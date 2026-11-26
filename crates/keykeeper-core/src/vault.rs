//! The encrypted vault file (`docs/PLAN.md` §5).
//!
//! A password-derived KEK (Argon2id) wraps a random 256-bit DEK; the DEK encrypts the JSON payload.
//! Both use XChaCha20-Poly1305 with random 192-bit nonces. Changing the password re-wraps the DEK;
//! every save uses a fresh payload nonce.
//!
//! File layout (integers little-endian):
//!
//! | offset | len | field |
//! |---:|---:|---|
//! | 0   | 4  | magic `KKV\0` |
//! | 4   | 2  | format version (1) |
//! | 6   | 1  | KDF id (1 = Argon2id v0x13) |
//! | 7   | 12 | Argon2 m (KiB), t, p as 3 × u32 |
//! | 19  | 16 | salt |
//! | 35  | 24 | DEK nonce |
//! | 59  | 48 | wrapped DEK (32 + 16-byte tag) |
//! | 107 | 24 | payload nonce |
//! | 131 | …  | payload ciphertext + 16-byte tag |
//!
//! AAD: bytes `0..35` for the DEK, the whole header `0..131` for the payload, so changing any
//! header byte makes decryption fail.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::data::VaultData;
use crate::error::VaultError;

const MAGIC: [u8; 4] = *b"KKV\0";
pub const FORMAT_VERSION: u16 = 1;
const KDF_ARGON2ID: u8 = 1;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const DEK_LEN: usize = 32;
const TAG_LEN: usize = 16;
const WRAPPED_DEK_LEN: usize = DEK_LEN + TAG_LEN;

const OFF_SALT: usize = 19;
const OFF_DEK_NONCE: usize = OFF_SALT + SALT_LEN; // 35
const OFF_WRAPPED: usize = OFF_DEK_NONCE + NONCE_LEN; // 59
const OFF_PAYLOAD_NONCE: usize = OFF_WRAPPED + WRAPPED_DEK_LEN; // 107
pub const HEADER_LEN: usize = OFF_PAYLOAD_NONCE + NONCE_LEN; // 131

/// Upper bound on backups we'll ever look for (and on `Settings::backups_to_keep`).
pub const MAX_BACKUPS: u32 = 50;

/// Argon2id cost parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl KdfParams {
    /// 256 MiB, 3 passes, 1 lane: ~0.3 s on a fast desktop CPU (measured), more on laptops.
    pub const DEFAULT: KdfParams = KdfParams {
        m_kib: 256 * 1024,
        t: 3,
        p: 1,
    };
    /// **Insecure.** Fast parameters for tests only.
    pub const INSECURE_FAST_FOR_TESTS: KdfParams = KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    };
    /// Refuse to open files demanding more than 1 GiB / 16 passes / 16 lanes, so a crafted
    /// header can't make the app allocate or spin forever.
    const MAX_M_KIB: u32 = 1024 * 1024;
    const MAX_T: u32 = 16;
    const MAX_P: u32 = 16;

    pub fn validate(self) -> Result<(), VaultError> {
        let ok = (1..=Self::MAX_P).contains(&self.p)
            && (1..=Self::MAX_T).contains(&self.t)
            && self.m_kib >= 8 * self.p
            && self.m_kib <= Self::MAX_M_KIB;
        if ok {
            Ok(())
        } else {
            Err(VaultError::InvalidKdfParams)
        }
    }
}

impl Default for KdfParams {
    fn default() -> Self {
        KdfParams::DEFAULT
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Header {
    params: KdfParams,
    salt: [u8; SALT_LEN],
    dek_nonce: [u8; NONCE_LEN],
    wrapped_dek: [u8; WRAPPED_DEK_LEN],
}

impl Header {
    fn prefix(params: KdfParams, salt: &[u8; SALT_LEN]) -> [u8; OFF_DEK_NONCE] {
        let mut b = [0u8; OFF_DEK_NONCE];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        b[6] = KDF_ARGON2ID;
        b[7..11].copy_from_slice(&params.m_kib.to_le_bytes());
        b[11..15].copy_from_slice(&params.t.to_le_bytes());
        b[15..19].copy_from_slice(&params.p.to_le_bytes());
        b[OFF_SALT..OFF_DEK_NONCE].copy_from_slice(salt);
        b
    }

    fn encode(&self, payload_nonce: &[u8; NONCE_LEN]) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[..OFF_DEK_NONCE].copy_from_slice(&Header::prefix(self.params, &self.salt));
        b[OFF_DEK_NONCE..OFF_WRAPPED].copy_from_slice(&self.dek_nonce);
        b[OFF_WRAPPED..OFF_PAYLOAD_NONCE].copy_from_slice(&self.wrapped_dek);
        b[OFF_PAYLOAD_NONCE..HEADER_LEN].copy_from_slice(payload_nonce);
        b
    }

    fn decode(bytes: &[u8]) -> Result<(Header, [u8; NONCE_LEN]), VaultError> {
        if bytes.len() < 4 || bytes[0..4] != MAGIC {
            return Err(VaultError::NotAVault);
        }
        if bytes.len() < 6 {
            return Err(VaultError::Corrupt);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != FORMAT_VERSION {
            return Err(VaultError::UnsupportedFormat(version));
        }
        if bytes.len() < HEADER_LEN + TAG_LEN {
            return Err(VaultError::Corrupt);
        }
        if bytes[6] != KDF_ARGON2ID {
            return Err(VaultError::InvalidKdfParams);
        }
        let u32_at =
            |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let params = KdfParams {
            m_kib: u32_at(7),
            t: u32_at(11),
            p: u32_at(15),
        };
        params.validate()?;
        let arr = |from: usize, to: usize| bytes[from..to].to_vec();
        let header = Header {
            params,
            salt: arr(OFF_SALT, OFF_DEK_NONCE)
                .try_into()
                .map_err(|_| VaultError::Corrupt)?,
            dek_nonce: arr(OFF_DEK_NONCE, OFF_WRAPPED)
                .try_into()
                .map_err(|_| VaultError::Corrupt)?,
            wrapped_dek: arr(OFF_WRAPPED, OFF_PAYLOAD_NONCE)
                .try_into()
                .map_err(|_| VaultError::Corrupt)?,
        };
        let nonce = arr(OFF_PAYLOAD_NONCE, HEADER_LEN)
            .try_into()
            .map_err(|_| VaultError::Corrupt)?;
        Ok((header, nonce))
    }
}

fn random<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| VaultError::Rng)?;
    Ok(b)
}

fn cipher(key: &[u8; 32]) -> Result<XChaCha20Poly1305, VaultError> {
    XChaCha20Poly1305::new_from_slice(key).map_err(|_| VaultError::Corrupt)
}

fn derive_kek(
    password: &str,
    salt: &[u8; SALT_LEN],
    params: KdfParams,
) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    params.validate()?;
    let p = Params::new(params.m_kib, params.t, params.p, Some(32))
        .map_err(|_| VaultError::InvalidKdfParams)?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, p)
        .hash_password_into(password.as_bytes(), salt, out.as_mut())
        .map_err(|_| VaultError::InvalidKdfParams)?;
    Ok(out)
}

/// The unlocked key material for one vault: the header that wraps the DEK, and the DEK itself.
/// Holds no plaintext data. Zeroised on drop.
pub struct VaultKeys {
    header: Header,
    dek: Zeroizing<[u8; DEK_LEN]>,
}

impl VaultKeys {
    /// New random DEK wrapped under `password`.
    pub fn new(password: &str, params: KdfParams) -> Result<VaultKeys, VaultError> {
        let dek = Zeroizing::new(random::<DEK_LEN>()?);
        let header = wrap(password, params, &dek)?;
        Ok(VaultKeys { header, dek })
    }

    /// Same DEK, wrapped under a new password with a fresh salt.
    pub fn rewrap(&self, password: &str, params: KdfParams) -> Result<VaultKeys, VaultError> {
        let header = wrap(password, params, &self.dek)?;
        Ok(VaultKeys {
            header,
            dek: self.dek.clone(),
        })
    }

    pub fn params(&self) -> KdfParams {
        self.header.params
    }

    /// Encrypt `data` into a complete vault file image.
    pub fn seal(&self, data: &VaultData) -> Result<Vec<u8>, VaultError> {
        let json = data.to_json()?;
        let nonce = random::<NONCE_LEN>()?;
        let header = self.header.encode(&nonce);
        let ct = cipher(&self.dek)?
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &json,
                    aad: &header,
                },
            )
            .map_err(|_| VaultError::Corrupt)?;
        let mut out = Vec::with_capacity(HEADER_LEN + ct.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Decrypt a vault file image.
    pub fn open(bytes: &[u8], password: &str) -> Result<(VaultKeys, VaultData), VaultError> {
        let (header, nonce) = Header::decode(bytes)?;
        let dek = unwrap(&header, password)?;
        let plain = cipher(&dek)?
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &bytes[HEADER_LEN..],
                    aad: &bytes[..HEADER_LEN],
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| VaultError::Corrupt)?;
        let data = VaultData::from_json(&plain)?;
        Ok((VaultKeys { header, dek }, data))
    }

    /// Whether `password` unlocks this vault. Costs one key derivation.
    pub fn verify_password(&self, password: &str) -> bool {
        unwrap(&self.header, password).is_ok_and(|dek| *dek == *self.dek)
    }
}

fn wrap(password: &str, params: KdfParams, dek: &[u8; DEK_LEN]) -> Result<Header, VaultError> {
    if password.is_empty() {
        return Err(VaultError::EmptyPassword);
    }
    let salt = random::<SALT_LEN>()?;
    let dek_nonce = random::<NONCE_LEN>()?;
    let kek = derive_kek(password, &salt, params)?;
    let aad = Header::prefix(params, &salt);
    let wrapped = cipher(&kek)?
        .encrypt(
            &XNonce::from(dek_nonce),
            Payload {
                msg: dek,
                aad: &aad,
            },
        )
        .map_err(|_| VaultError::Corrupt)?;
    let wrapped_dek: [u8; WRAPPED_DEK_LEN] = wrapped.try_into().map_err(|_| VaultError::Corrupt)?;
    Ok(Header {
        params,
        salt,
        dek_nonce,
        wrapped_dek,
    })
}

fn unwrap(header: &Header, password: &str) -> Result<Zeroizing<[u8; DEK_LEN]>, VaultError> {
    let kek = derive_kek(password, &header.salt, header.params)?;
    let aad = Header::prefix(header.params, &header.salt);
    let plain = cipher(&kek)?
        .decrypt(
            &XNonce::from(header.dek_nonce),
            Payload {
                msg: &header.wrapped_dek,
                aad: &aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| VaultError::BadPassword)?;
    let mut dek = Zeroizing::new([0u8; DEK_LEN]);
    if plain.len() != DEK_LEN {
        return Err(VaultError::Corrupt);
    }
    dek.copy_from_slice(&plain);
    Ok(dek)
}

// ── Files ─────────────────────────────────────────────────────────────────────────────────────

/// An unlocked vault file on disk.
pub struct Vault {
    path: PathBuf,
    keys: VaultKeys,
}

impl Vault {
    /// Create a new vault file. Refuses to overwrite an existing file.
    pub fn create(
        path: impl Into<PathBuf>,
        password: &str,
        params: KdfParams,
        data: &VaultData,
    ) -> Result<Vault, VaultError> {
        let path = path.into();
        if path.exists() {
            return Err(VaultError::AlreadyExists(path));
        }
        let keys = VaultKeys::new(password, params)?;
        write_atomic(&path, &keys.seal(data)?)?;
        Ok(Vault { path, keys })
    }

    pub fn open(
        path: impl Into<PathBuf>,
        password: &str,
    ) -> Result<(Vault, VaultData), VaultError> {
        let path = path.into();
        let bytes = fs::read(&path)?;
        let (keys, data) = VaultKeys::open(&bytes, password)?;
        Ok((Vault { path, keys }, data))
    }

    /// Encrypt and atomically replace the file, rotating up to `data.settings.backups_to_keep`
    /// encrypted backups (`<file>.bak1` is the newest).
    pub fn save(&self, data: &VaultData) -> Result<(), VaultError> {
        let bytes = self.keys.seal(data)?;
        rotate_backups(&self.path, data.settings.backups_to_keep.min(MAX_BACKUPS))?;
        write_atomic(&self.path, &bytes)
    }

    /// Re-wrap under a new password and save. Existing backups are **deleted**, because they
    /// would still open with the old password.
    pub fn change_password(
        &mut self,
        new_password: &str,
        params: KdfParams,
        data: &VaultData,
    ) -> Result<(), VaultError> {
        let keys = self.keys.rewrap(new_password, params)?;
        write_atomic(&self.path, &keys.seal(data)?)?;
        self.keys = keys;
        delete_backups(&self.path)
    }

    /// Write an encrypted copy (same password) somewhere else, e.g. a manual backup.
    /// Writing over the live vault path is refused; use [`Vault::save`] for that.
    pub fn save_copy(&self, path: &Path, data: &VaultData) -> Result<(), VaultError> {
        if path == self.path {
            return Err(VaultError::AlreadyExists(path.to_path_buf()));
        }
        write_atomic(path, &self.keys.seal(data)?)
    }

    pub fn verify_password(&self, password: &str) -> bool {
        self.keys.verify_password(password)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn params(&self) -> KdfParams {
        self.keys.params()
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// `vault.kkvault` → `vault.kkvault.bak{n}`.
pub fn backup_path(path: &Path, n: u32) -> PathBuf {
    with_suffix(path, &format!(".bak{n}"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let tmp = with_suffix(path, ".tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    // `rename` replaces the destination atomically on both Windows (MoveFileEx) and Unix.
    fs::rename(&tmp, path)?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), VaultError> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn rotate_backups(path: &Path, keep: u32) -> Result<(), VaultError> {
    for n in (keep + 1)..=MAX_BACKUPS {
        remove_if_exists(&backup_path(path, n))?;
    }
    if keep == 0 || !path.exists() {
        return Ok(());
    }
    for n in (1..keep).rev() {
        let from = backup_path(path, n);
        if from.exists() {
            fs::rename(&from, backup_path(path, n + 1))?;
        }
    }
    fs::copy(path, backup_path(path, 1))?;
    Ok(())
}

fn delete_backups(path: &Path) -> Result<(), VaultError> {
    for n in 1..=MAX_BACKUPS {
        remove_if_exists(&backup_path(path, n))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GameKey;

    const FAST: KdfParams = KdfParams::INSECURE_FAST_FOR_TESTS;

    fn sample() -> VaultData {
        let mut d = VaultData::default();
        d.keys.push(GameKey::new("Undertale", "UNDR7-TALE0-00001"));
        d.keys
            .push(GameKey::new("Haiku, the Robot", "HA1KU-R0B0T-00002"));
        d
    }

    #[test]
    fn header_layout_constants() {
        assert_eq!(OFF_DEK_NONCE, 35);
        assert_eq!(OFF_WRAPPED, 59);
        assert_eq!(OFF_PAYLOAD_NONCE, 107);
        assert_eq!(HEADER_LEN, 131);
    }

    #[test]
    fn seal_open_round_trip() {
        let keys = VaultKeys::new("hunter2", FAST).unwrap();
        let original = sample();
        let bytes = keys.seal(&original).unwrap();
        assert_eq!(&bytes[0..4], b"KKV\0");
        let (_, data) = VaultKeys::open(&bytes, "hunter2").unwrap();
        assert_eq!(data, original);
    }

    #[test]
    fn plaintext_never_appears_in_file() {
        let keys = VaultKeys::new("pw", FAST).unwrap();
        let bytes = keys.seal(&sample()).unwrap();
        let hay = String::from_utf8_lossy(&bytes);
        assert!(!hay.contains("UNDR7"));
        assert!(!hay.contains("Undertale"));
    }

    #[test]
    fn wrong_password_is_bad_password() {
        let keys = VaultKeys::new("right", FAST).unwrap();
        let bytes = keys.seal(&sample()).unwrap();
        assert!(matches!(
            VaultKeys::open(&bytes, "wrong"),
            Err(VaultError::BadPassword)
        ));
        assert!(matches!(
            VaultKeys::open(&bytes, ""),
            Err(VaultError::BadPassword)
        ));
    }

    #[test]
    fn empty_password_is_refused_on_create() {
        assert!(matches!(
            VaultKeys::new("", FAST),
            Err(VaultError::EmptyPassword)
        ));
    }

    #[test]
    fn every_flipped_byte_is_detected() {
        let keys = VaultKeys::new("pw", FAST).unwrap();
        let bytes = keys.seal(&VaultData::default()).unwrap();
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0x01;
            assert!(
                VaultKeys::open(&b, "pw").is_err(),
                "flipping byte {i} went undetected"
            );
        }
    }

    #[test]
    fn truncation_and_garbage_are_rejected() {
        let keys = VaultKeys::new("pw", FAST).unwrap();
        let bytes = keys.seal(&sample()).unwrap();
        for len in [
            0,
            3,
            4,
            6,
            HEADER_LEN,
            HEADER_LEN + TAG_LEN,
            bytes.len() - 1,
        ] {
            assert!(
                VaultKeys::open(&bytes[..len], "pw").is_err(),
                "truncated to {len}"
            );
        }
        assert!(matches!(
            VaultKeys::open(b"PK\x03\x04 not a vault", "pw"),
            Err(VaultError::NotAVault)
        ));
        let mut appended = bytes.clone();
        appended.push(0);
        assert!(matches!(
            VaultKeys::open(&appended, "pw"),
            Err(VaultError::Corrupt)
        ));
    }

    #[test]
    fn future_format_version_is_reported() {
        let keys = VaultKeys::new("pw", FAST).unwrap();
        let mut bytes = keys.seal(&sample()).unwrap();
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert!(matches!(
            VaultKeys::open(&bytes, "pw"),
            Err(VaultError::UnsupportedFormat(2))
        ));
    }

    #[test]
    fn hostile_kdf_params_are_refused_before_deriving() {
        let keys = VaultKeys::new("pw", FAST).unwrap();
        let mut bytes = keys.seal(&sample()).unwrap();
        bytes[7..11].copy_from_slice(&u32::MAX.to_le_bytes()); // 4 TiB of memory
        assert!(matches!(
            VaultKeys::open(&bytes, "pw"),
            Err(VaultError::InvalidKdfParams)
        ));
    }

    #[test]
    fn params_validation() {
        assert!(KdfParams::DEFAULT.validate().is_ok());
        assert!(FAST.validate().is_ok());
        assert!(
            KdfParams {
                m_kib: 7,
                t: 1,
                p: 1
            }
            .validate()
            .is_err()
        );
        assert!(
            KdfParams {
                m_kib: 64,
                t: 0,
                p: 1
            }
            .validate()
            .is_err()
        );
        assert!(
            KdfParams {
                m_kib: 64,
                t: 1,
                p: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            KdfParams {
                m_kib: 64,
                t: 1,
                p: 9
            }
            .validate()
            .is_err()
        ); // m < 8 * p
        assert!(
            KdfParams {
                m_kib: 64,
                t: 17,
                p: 1
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn each_save_uses_a_fresh_nonce() {
        let keys = VaultKeys::new("pw", FAST).unwrap();
        let a = keys.seal(&sample()).unwrap();
        let b = keys.seal(&sample()).unwrap();
        assert_eq!(
            a[..OFF_PAYLOAD_NONCE],
            b[..OFF_PAYLOAD_NONCE],
            "wrapped DEK is unchanged"
        );
        assert_ne!(
            a[OFF_PAYLOAD_NONCE..HEADER_LEN],
            b[OFF_PAYLOAD_NONCE..HEADER_LEN]
        );
        assert_ne!(a[HEADER_LEN..], b[HEADER_LEN..]);
    }

    #[test]
    fn verify_password() {
        let keys = VaultKeys::new("pw", FAST).unwrap();
        assert!(keys.verify_password("pw"));
        assert!(!keys.verify_password("pW"));
    }

    // ── file tests ──

    #[test]
    fn create_open_save_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my.kkvault");
        let mut data = sample();
        let vault = Vault::create(&path, "pw", FAST, &data).unwrap();
        data.keys.push(GameKey::new("Hades", "AAAAA-BBBBB-CCCCC"));
        vault.save(&data).unwrap();
        let (_, back) = Vault::open(&path, "pw").unwrap();
        assert_eq!(back, data);
        assert!(
            !with_suffix(&path, ".tmp").exists(),
            "temp file is renamed away"
        );
    }

    #[test]
    fn create_refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my.kkvault");
        fs::write(&path, b"precious").unwrap();
        assert!(matches!(
            Vault::create(&path, "pw", FAST, &sample()),
            Err(VaultError::AlreadyExists(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"precious");
    }

    #[test]
    fn stale_temp_file_does_not_matter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my.kkvault");
        let vault = Vault::create(&path, "pw", FAST, &sample()).unwrap();
        fs::write(
            with_suffix(&path, ".tmp"),
            b"half-written garbage from a crash",
        )
        .unwrap();
        assert!(Vault::open(&path, "pw").is_ok());
        vault.save(&sample()).unwrap();
        assert!(Vault::open(&path, "pw").is_ok());
    }

    #[test]
    fn backups_rotate_and_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my.kkvault");
        let mut data = VaultData::default();
        data.settings.backups_to_keep = 3;
        let vault = Vault::create(&path, "pw", FAST, &data).unwrap();
        for i in 0..6 {
            data.keys
                .push(GameKey::new(format!("Game {i}"), format!("KEY-{i}")));
            vault.save(&data).unwrap();
        }
        for n in 1..=3 {
            assert!(backup_path(&path, n).exists(), "bak{n}");
        }
        assert!(!backup_path(&path, 4).exists());
        // bak1 is the state before the last save: 5 keys.
        let (_, bak1) = Vault::open(backup_path(&path, 1), "pw").unwrap();
        assert_eq!(bak1.keys.len(), 5);
        let (_, bak3) = Vault::open(backup_path(&path, 3), "pw").unwrap();
        assert_eq!(bak3.keys.len(), 3);

        // Lowering the setting prunes extras on the next save.
        data.settings.backups_to_keep = 1;
        vault.save(&data).unwrap();
        assert!(backup_path(&path, 1).exists());
        assert!(!backup_path(&path, 2).exists());
    }

    #[test]
    fn change_password_reencrypts_and_drops_backups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my.kkvault");
        let data = sample();
        let mut vault = Vault::create(&path, "old", FAST, &data).unwrap();
        vault.save(&data).unwrap();
        assert!(backup_path(&path, 1).exists());

        vault.change_password("new", FAST, &data).unwrap();
        assert!(
            !backup_path(&path, 1).exists(),
            "old-password backups are removed"
        );
        assert!(matches!(
            Vault::open(&path, "old"),
            Err(VaultError::BadPassword)
        ));
        let (v2, back) = Vault::open(&path, "new").unwrap();
        assert_eq!(back, data);
        assert!(v2.verify_password("new"));
        // The same session keeps saving under the new password.
        vault.save(&data).unwrap();
        assert!(Vault::open(&path, "new").is_ok());
    }

    #[test]
    fn default_params_take_reasonable_time() {
        // Guard against accidentally shipping test parameters; also documents the cost.
        let start = std::time::Instant::now();
        let _ = VaultKeys::new("pw", KdfParams::DEFAULT).unwrap();
        let ms = start.elapsed().as_millis();
        eprintln!("Argon2id default params: {ms} ms");
        assert!(ms > 20, "Argon2 defaults look too cheap ({ms} ms)");
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::model::{GameKey, KeyStatus};
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn arbitrary_data_round_trips(
            entries in prop::collection::vec(("\\PC{0,40}", "[A-Z0-9-]{0,29}", 0usize..6), 0..20),
            password in "\\PC{1,32}",
        ) {
            let mut data = VaultData::default();
            for (name, key, s) in entries {
                let mut k = GameKey::new(name, key);
                k.status = KeyStatus::ALL[s];
                data.keys.push(k);
            }
            let keys = VaultKeys::new(&password, KdfParams::INSECURE_FAST_FOR_TESTS).unwrap();
            let bytes = keys.seal(&data).unwrap();
            let (_, back) = VaultKeys::open(&bytes, &password).unwrap();
            prop_assert_eq!(back, data);
        }

        #[test]
        fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
            let _ = VaultKeys::open(&bytes, "pw");
        }
    }
}
