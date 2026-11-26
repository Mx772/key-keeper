//! Safety net for app upgrades: the first time a newer KeyKeeper opens a vault, it keeps a
//! byte-for-byte copy of the file as it was (`<vault>.pre-<version>`) before saving anything.
//!
//! The copy is the original encrypted file, not a re-encryption of the loaded data, so it opens
//! with the same password in the *previous* app version even if the new one migrated the schema.

use std::fs;
use std::path::{Path, PathBuf};

use semver::Version;

use crate::data::VaultData;
use crate::error::VaultError;
use crate::vault::Vault;

/// `vault.kkvault` → `vault.kkvault.pre-0.2.0`.
pub fn pre_upgrade_path(vault: &Path, app_version: &str) -> PathBuf {
    let mut s = vault.as_os_str().to_owned();
    s.push(format!(".pre-{app_version}"));
    PathBuf::from(s)
}

/// Whether `current` is newer than the version that last opened the vault. A vault with no
/// recorded version predates this check and counts as older. Unparsable versions compare as
/// "different", so an odd value errs on the side of making a backup.
pub fn is_upgrade(last: Option<&str>, current: &str) -> bool {
    let Some(last) = last else { return true };
    match (Version::parse(last), Version::parse(current)) {
        (Ok(l), Ok(c)) => c > l,
        _ => last != current,
    }
}

/// Call right after [`Vault::open`], before anything else is saved. When `app_version` is newer
/// than the vault's `last_app_version`, copies the vault file to [`pre_upgrade_path`] (unless
/// that copy already exists), records `app_version` in `data`, and saves. Returns the backup
/// path when one was written.
///
/// An older app opening the vault changes nothing, so the newest version seen is kept.
pub fn backup_before_upgrade(
    vault: &Vault,
    data: &mut VaultData,
    app_version: &str,
) -> Result<Option<PathBuf>, VaultError> {
    if !is_upgrade(data.last_app_version.as_deref(), app_version) {
        return Ok(None);
    }
    let backup = pre_upgrade_path(vault.path(), app_version);
    let written = if backup.exists() {
        None
    } else {
        copy_atomic(vault.path(), &backup)?;
        Some(backup)
    };
    data.last_app_version = Some(app_version.to_string());
    vault.save(data)?;
    Ok(written)
}

fn copy_atomic(from: &Path, to: &Path) -> Result<(), VaultError> {
    let mut tmp = to.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::copy(from, &tmp)?;
    // FlushFileBuffers needs a writable handle on Windows.
    fs::OpenOptions::new().write(true).open(&tmp)?.sync_all()?;
    fs::rename(&tmp, to)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GameKey;
    use crate::vault::KdfParams;

    const PW: &str = "correct horse";

    fn vault_with(dir: &Path, last: Option<&str>) -> PathBuf {
        let path = dir.join("v.kkvault");
        let mut data = VaultData::default();
        data.keys.push(GameKey::new("Celeste", "QQ7RT-PP2MN-ZZ9XY"));
        data.last_app_version = last.map(str::to_string);
        Vault::create(&path, PW, KdfParams::INSECURE_FAST_FOR_TESTS, &data).unwrap();
        path
    }

    #[test]
    fn version_comparison() {
        assert!(is_upgrade(None, "0.1.0"));
        assert!(is_upgrade(Some("0.1.0"), "0.2.0"));
        assert!(is_upgrade(Some("0.9.0"), "0.10.0"), "numeric, not lexical");
        assert!(!is_upgrade(Some("0.2.0"), "0.2.0"));
        assert!(!is_upgrade(Some("0.3.0"), "0.2.0"), "older app: no backup");
        assert!(is_upgrade(Some("garbage"), "0.2.0"));
        assert!(!is_upgrade(Some("same-garbage"), "same-garbage"));
    }

    #[test]
    fn first_open_by_newer_version_copies_the_original_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = vault_with(dir.path(), Some("0.1.0"));
        let original = fs::read(&path).unwrap();

        let (vault, mut data) = Vault::open(&path, PW).unwrap();
        let backup = backup_before_upgrade(&vault, &mut data, "0.2.0").unwrap();
        let backup = backup.expect("backup written");
        assert_eq!(backup, dir.path().join("v.kkvault.pre-0.2.0"));
        assert_eq!(fs::read(&backup).unwrap(), original, "byte-for-byte copy");
        assert_eq!(data.last_app_version.as_deref(), Some("0.2.0"));

        // The version was saved, and the backup still opens with the same password.
        let (_, reopened) = Vault::open(&path, PW).unwrap();
        assert_eq!(reopened.last_app_version.as_deref(), Some("0.2.0"));
        let (_, old) = Vault::open(&backup, PW).unwrap();
        assert_eq!(old.last_app_version.as_deref(), Some("0.1.0"));
        assert_eq!(old.keys.len(), 1);

        // Opening again with the same version does nothing.
        let (vault, mut data) = Vault::open(&path, PW).unwrap();
        assert_eq!(
            backup_before_upgrade(&vault, &mut data, "0.2.0").unwrap(),
            None
        );
        assert_eq!(fs::read(&backup).unwrap(), original, "backup untouched");
    }

    #[test]
    fn vault_without_a_recorded_version_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = vault_with(dir.path(), None);
        let (vault, mut data) = Vault::open(&path, PW).unwrap();
        let backup = backup_before_upgrade(&vault, &mut data, "0.1.0").unwrap();
        assert!(backup.is_some_and(|b| b.exists()));
    }

    #[test]
    fn older_app_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = vault_with(dir.path(), Some("0.3.0"));
        let before = fs::read(&path).unwrap();
        let (vault, mut data) = Vault::open(&path, PW).unwrap();
        assert_eq!(
            backup_before_upgrade(&vault, &mut data, "0.2.0").unwrap(),
            None
        );
        assert_eq!(data.last_app_version.as_deref(), Some("0.3.0"));
        assert_eq!(fs::read(&path).unwrap(), before, "not re-saved");
        assert!(!pre_upgrade_path(&path, "0.2.0").exists());
    }

    #[test]
    fn existing_backup_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = vault_with(dir.path(), Some("0.1.0"));
        let backup = pre_upgrade_path(&path, "0.2.0");
        fs::write(&backup, b"earlier backup").unwrap();
        let (vault, mut data) = Vault::open(&path, PW).unwrap();
        assert_eq!(
            backup_before_upgrade(&vault, &mut data, "0.2.0").unwrap(),
            None
        );
        assert_eq!(fs::read(&backup).unwrap(), b"earlier backup");
        assert_eq!(data.last_app_version.as_deref(), Some("0.2.0"));
    }
}
