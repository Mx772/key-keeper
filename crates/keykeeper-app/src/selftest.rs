//! `keykeeper --self-test`: an internal smoke test the release workflow runs on the built
//! binary before publishing. It needs no GPU, no window and no network, and never touches the
//! user's vault or config: everything happens in a fresh temp directory that is removed after.
//!
//! It creates a vault with fast test KDF parameters, unlocks it through the real lock-screen
//! path (which also writes the pre-upgrade backup), and lays out one headless frame of the
//! library. Exit code 0 on success, 1 on failure.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use keykeeper_core::{KdfParams, Vault, upgrade};
use rune::{Env, Runtime, TextEdit};

use crate::app::AppState;
use crate::config::Config;
use crate::{demo, ui, update};

pub const FLAG: &str = "--self-test";
const PASSWORD: &str = "self-test-password";

pub fn run() -> i32 {
    let dir = std::env::temp_dir().join(format!("keykeeper-self-test-{}", std::process::id()));
    let result = check(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => {
            println!("keykeeper {} self-test: ok", env!("CARGO_PKG_VERSION"));
            0
        }
        Err(e) => {
            eprintln!("keykeeper self-test FAILED: {e}");
            1
        }
    }
}

fn check(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("temp dir: {e}"))?;
    let path: PathBuf = dir.join("self-test.kkvault");

    // 1. Create a vault as an "older version" would have left it.
    let mut data = demo::sample_data();
    data.last_app_version = Some("0.0.0".into());
    Vault::create(&path, PASSWORD, KdfParams::INSECURE_FAST_FOR_TESTS, &data)
        .map_err(|e| format!("create vault: {e}"))?;

    // 2. Unlock it through the lock screen, headless.
    let config = Config {
        vault_path: Some(path.clone()),
        check_updates: Some(false),
        ..Config::default()
    };
    let mut s = AppState::new(config, None, KdfParams::INSECURE_FAST_FOR_TESTS);
    s.updates.source = std::sync::Arc::new(update::NoUpdates);
    let mut rt = Runtime::new(s, ui::view, (1280.0, 800.0), Env::new(true, 1.0));
    rt.frame(0.0);
    if rt.find("lock-submit").is_none() {
        return Err("the lock screen did not render".into());
    }
    rt.update(|s, _| {
        if let Some(l) = s.lock_ui_mut() {
            l.password = TextEdit::new(PASSWORD);
        }
        s.start_unlock();
    });
    let start = Instant::now();
    while !rt.state.is_unlocked() {
        if start.elapsed() > Duration::from_secs(20) {
            return Err("unlock timed out".into());
        }
        if let Some(e) = rt.state.lock_ui_mut().and_then(|l| l.error.clone()) {
            return Err(format!("unlock failed: {e}"));
        }
        rt.update(|s, tx| {
            s.poll(0.0, tx);
        });
        std::thread::sleep(Duration::from_millis(5));
    }
    rt.frame(1.0 / 60.0);

    // 3. The data survived the round trip and the library renders it.
    let sess = rt.state.session().ok_or("not unlocked")?;
    let keys = sess.repo.keys();
    if keys.len() != data.keys.len()
        || keys
            .iter()
            .zip(&data.keys)
            .any(|(a, b)| a.key.expose() != b.key.expose() || a.game_name != b.game_name)
    {
        return Err("vault contents changed between save and open".into());
    }
    for key in ["library", "sidebar", "library-count"] {
        if rt.find(key).is_none() {
            return Err(format!("'{key}' did not render"));
        }
    }

    // 4. The pre-upgrade backup was written and opens.
    let backup = upgrade::pre_upgrade_path(&path, env!("CARGO_PKG_VERSION"));
    let (_, old) =
        Vault::open(&backup, PASSWORD).map_err(|e| format!("pre-upgrade backup: {e}"))?;
    if old.last_app_version.as_deref() != Some("0.0.0") {
        return Err("pre-upgrade backup is not the original file".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn self_test_passes() {
        let dir = tempfile::tempdir().unwrap();
        super::check(&dir.path().join("x")).unwrap();
    }
}
