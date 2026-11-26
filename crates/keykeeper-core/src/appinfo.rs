//! Store details about a game (description, tags, release, Steam Deck support, artwork links),
//! cached in the vault by Steam app id, and [`BlobCipher`]: encryption for the artwork cache
//! that lives next to the vault.
//!
//! The artwork itself is public, but which games a vault holds is not, so cached images are
//! encrypted with a key derived from the vault's data key and stored under names that don't
//! reveal the app id.

use std::path::{Path, PathBuf};

use blake2::{Blake2b512, Digest};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::VaultError;

/// Steam's own verdict on how a game runs on the Steam Deck.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeckCompat {
    #[default]
    Unknown,
    Unsupported,
    Playable,
    Verified,
}

impl DeckCompat {
    /// Steam's `steam_deck_compat_category` number.
    pub fn from_category(n: u64) -> DeckCompat {
        match n {
            1 => DeckCompat::Unsupported,
            2 => DeckCompat::Playable,
            3 => DeckCompat::Verified,
            _ => DeckCompat::Unknown,
        }
    }

    pub fn label(self) -> Option<&'static str> {
        match self {
            DeckCompat::Unknown => None,
            DeckCompat::Unsupported => Some("Deck: unsupported"),
            DeckCompat::Playable => Some("Deck: playable"),
            DeckCompat::Verified => Some("Deck verified"),
        }
    }

    /// "Verified", "Playable", "Unsupported".
    pub fn short_label(self) -> Option<&'static str> {
        match self {
            DeckCompat::Unknown => None,
            DeckCompat::Unsupported => Some("Unsupported"),
            DeckCompat::Playable => Some("Playable"),
            DeckCompat::Verified => Some("Verified"),
        }
    }
}

/// What the store says about one Steam app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    /// The store's name for the app (may differ from the key's game name).
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub developers: Vec<String>,
    /// The store's top user tags, most popular first ("Metroidvania", "Souls-like", …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Release date on Steam, if released.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<Timestamp>,
    #[serde(default)]
    pub deck: DeckCompat,
    /// Header art (460×215), wide capsule (616×353): full https URLs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capsule_url: Option<String>,
    pub fetched_at: Timestamp,
}

impl AppInfo {
    /// The header image URL, falling back to Steam's classic path for the app.
    pub fn header_or_default(&self, app_id: u32) -> String {
        self.header_url
            .clone()
            .unwrap_or_else(|| default_header_url(app_id))
    }
}

/// Steam's long-standing header path; still served for nearly every app.
pub fn default_header_url(app_id: u32) -> String {
    format!(
        "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{app_id}/header.jpg"
    )
}

// ── encrypted artwork cache ───────────────────────────────────────────────────────────────────

const NONCE_LEN: usize = 24;
const BLOB_MAGIC: &[u8; 4] = b"KKA1";

/// Encrypts small cached files (artwork) with a key derived from a vault's data key. Clone it
/// into background threads freely; the key is zeroised on drop.
#[derive(Clone)]
pub struct BlobCipher {
    key: Zeroizing<[u8; 32]>,
    names: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for BlobCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlobCipher(..)")
    }
}

fn derive(dek: &[u8], context: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut h = Blake2b512::new();
    h.update(b"keykeeper/blob/v1/");
    h.update(context);
    h.update([0u8]);
    h.update(dek);
    let digest = h.finalize();
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&digest[..32]);
    out
}

impl BlobCipher {
    pub(crate) fn from_dek(dek: &[u8]) -> BlobCipher {
        BlobCipher {
            key: derive(dek, b"encrypt"),
            names: derive(dek, b"names"),
        }
    }

    /// A file name for `id` (e.g. `"art-367520"`) that doesn't reveal it: 32 hex characters.
    pub fn file_name(&self, id: &str) -> String {
        let mut h = Blake2b512::new();
        h.update(&self.names[..]);
        h.update(id.as_bytes());
        h.finalize()[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// `magic ‖ nonce ‖ ciphertext`, bound to `id` so files can't be swapped.
    pub fn seal(&self, id: &str, plain: &[u8]) -> Result<Vec<u8>, VaultError> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| VaultError::Rng)?;
        let ct = XChaCha20Poly1305::new_from_slice(&self.key[..])
            .map_err(|_| VaultError::Corrupt)?
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plain,
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| VaultError::Corrupt)?;
        let mut out = Vec::with_capacity(4 + NONCE_LEN + ct.len());
        out.extend_from_slice(BLOB_MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn open(&self, id: &str, sealed: &[u8]) -> Result<Vec<u8>, VaultError> {
        let body = sealed
            .strip_prefix(BLOB_MAGIC)
            .filter(|b| b.len() >= NONCE_LEN)
            .ok_or(VaultError::Corrupt)?;
        let (nonce, ct) = body.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| VaultError::Corrupt)?;
        XChaCha20Poly1305::new_from_slice(&self.key[..])
            .map_err(|_| VaultError::Corrupt)?
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ct,
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| VaultError::Corrupt)
    }
}

/// An encrypted file cache in one directory (next to the vault: `<vault>.cache/`).
#[derive(Clone, Debug)]
pub struct BlobCache {
    dir: PathBuf,
    cipher: BlobCipher,
}

impl BlobCache {
    pub fn new(dir: impl Into<PathBuf>, cipher: BlobCipher) -> BlobCache {
        BlobCache {
            dir: dir.into(),
            cipher,
        }
    }

    /// `vault.kkvault` → `vault.kkvault.cache`.
    pub fn dir_for(vault: &Path) -> PathBuf {
        let mut s = vault.as_os_str().to_owned();
        s.push(".cache");
        PathBuf::from(s)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(self.cipher.file_name(id))
    }

    /// The cached bytes for `id`, if present and intact.
    pub fn get(&self, id: &str) -> Option<Vec<u8>> {
        let sealed = std::fs::read(self.path(id)).ok()?;
        self.cipher.open(id, &sealed).ok()
    }

    pub fn put(&self, id: &str, plain: &[u8]) -> Result<(), VaultError> {
        std::fs::create_dir_all(&self.dir)?;
        let sealed = self.cipher.seal(id, plain)?;
        let path = self.path(id);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, sealed)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Delete every cached file. Returns how many bytes were freed.
    pub fn clear(&self) -> std::io::Result<u64> {
        let mut freed = 0;
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                freed += entry.metadata().map(|m| m.len()).unwrap_or(0);
                std::fs::remove_file(entry.path())?;
            }
        }
        let _ = std::fs::remove_dir(&self.dir);
        Ok(freed)
    }

    /// Total size of the cache on disk, in bytes.
    pub fn size(&self) -> u64 {
        std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter_map(|e| e.metadata().ok())
                    .filter(|m| m.is_file())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher(b: u8) -> BlobCipher {
        BlobCipher::from_dek(&[b; 32])
    }

    #[test]
    fn seal_open_round_trip_and_binding() {
        let c = cipher(7);
        let sealed = c.seal("art-1", b"jpeg bytes").unwrap();
        assert_eq!(c.open("art-1", &sealed).unwrap(), b"jpeg bytes");
        assert!(c.open("art-2", &sealed).is_err(), "bound to its id");
        assert!(
            cipher(8).open("art-1", &sealed).is_err(),
            "other vaults can't read it"
        );
        assert!(c.open("art-1", b"KKA1short").is_err());
        assert!(c.open("art-1", b"").is_err());
        let mut flipped = sealed.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert!(c.open("art-1", &flipped).is_err());
    }

    #[test]
    fn file_names_hide_ids_and_differ_per_vault() {
        let (a, b) = (cipher(1), cipher(2));
        let n = a.file_name("art-367520");
        assert_eq!(n.len(), 32);
        assert!(!n.contains("367520"));
        assert_eq!(n, a.file_name("art-367520"), "stable");
        assert_ne!(n, a.file_name("art-367521"));
        assert_ne!(n, b.file_name("art-367520"));
    }

    #[test]
    fn cache_put_get_clear() {
        let dir = tempfile::tempdir().unwrap();
        let cache = BlobCache::new(dir.path().join("v.kkvault.cache"), cipher(3));
        assert_eq!(cache.get("art-1"), None);
        assert_eq!(
            cache.clear().unwrap(),
            0,
            "clearing a missing cache is fine"
        );
        cache.put("art-1", b"hello").unwrap();
        assert_eq!(cache.get("art-1").as_deref(), Some(&b"hello"[..]));
        assert!(cache.size() > 5);
        let other = BlobCache::new(cache.dir().to_path_buf(), cipher(4));
        assert_eq!(other.get("art-1"), None, "another key can't read it");
        assert!(cache.clear().unwrap() > 0);
        assert_eq!(cache.get("art-1"), None);
        assert_eq!(cache.size(), 0);
        assert_eq!(
            BlobCache::dir_for(Path::new("x/vault.kkvault")),
            PathBuf::from("x/vault.kkvault.cache")
        );
    }

    #[test]
    fn deck_categories() {
        assert_eq!(DeckCompat::from_category(3), DeckCompat::Verified);
        assert_eq!(DeckCompat::from_category(2), DeckCompat::Playable);
        assert_eq!(DeckCompat::from_category(1), DeckCompat::Unsupported);
        assert_eq!(DeckCompat::from_category(0), DeckCompat::Unknown);
        assert_eq!(DeckCompat::Unknown.label(), None);
        assert!(DeckCompat::Verified.label().is_some());
    }

    #[test]
    fn header_fallback() {
        let info = AppInfo {
            name: String::new(),
            description: String::new(),
            developers: vec![],
            tags: vec![],
            released: None,
            deck: DeckCompat::Unknown,
            header_url: None,
            capsule_url: None,
            fetched_at: Timestamp::UNIX_EPOCH,
        };
        assert!(info.header_or_default(10).contains("/10/header.jpg"));
    }
}
