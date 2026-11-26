//! Opt-in updates. The app reads a small `latest.json` from the newest *published* GitHub
//! release, tells the user when a newer version exists, and only when they click **Update**
//! downloads the installer, verifies its minisign signature against [`PUBLIC_KEY`], and runs it.
//!
//! Network work sits behind [`UpdateSource`] so tests use a fake; the real one is
//! [`GitHubReleases`], built on `cargo-packager-updater` (the same project that builds and signs
//! the installers, so the manifest and signature formats match).
//!
//! The UI state and actions live in `update_actions.rs`.

use std::sync::Arc;
use std::time::Duration;

use cargo_packager_updater::{Config, Update, UpdaterBuilder};
use semver::Version;

/// Resolves to the newest published, non-draft, non-prerelease release's manifest.
pub const MANIFEST_URL: &str =
    "https://github.com/Mx772/key-keeper/releases/latest/download/latest.json";

/// The release-signing public key (minisign, base64 as `cargo packager signer` writes it).
/// Its private half signs every installer in the release workflow and never enters the repo.
pub const PUBLIC_KEY: &str = include_str!("../keys/updater.pub");

/// First automatic check this long after startup, so it never competes with the first frames.
pub const STARTUP_DELAY_SECS: f64 = 5.0;
/// Automatic checks while the app keeps running.
pub const CHECK_INTERVAL_SECS: f64 = 24.0 * 60.0 * 60.0;

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// The newest published release, as the manifest describes it for this platform.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: Version,
    /// Release notes (Markdown as written on GitHub), possibly empty.
    pub notes: String,
    /// `pub_date` as given, for display.
    pub date: Option<String>,
    /// What the real source needs to download and install; `None` from fakes.
    pub(crate) update: Option<Box<Update>>,
}

impl Release {
    pub fn new(version: Version, notes: impl Into<String>) -> Release {
        Release {
            version,
            notes: notes.into(),
            date: None,
            update: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// No connection, timeout, HTTP error.
    Network(String),
    /// No published release yet, or its manifest has no build for this system.
    NotAvailable(String),
    /// The manifest is malformed.
    Manifest(String),
    /// The download does not match its signature: refuse to install.
    BadSignature(String),
    /// Writing or starting the installer failed.
    Install(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Network(e) => write!(f, "Couldn't reach GitHub: {e}"),
            UpdateError::NotAvailable(e) => write!(f, "{e}"),
            UpdateError::Manifest(e) => write!(f, "The update information is invalid: {e}"),
            UpdateError::BadSignature(e) => write!(
                f,
                "The download's signature doesn't match KeyKeeper's release key, so it was not \
                 installed. It may have been damaged or tampered with. ({e})"
            ),
            UpdateError::Install(e) => write!(f, "Couldn't start the installer: {e}"),
        }
    }
}

impl From<cargo_packager_updater::Error> for UpdateError {
    fn from(e: cargo_packager_updater::Error) -> Self {
        use cargo_packager_updater::Error as E;
        let msg = e.to_string();
        match e {
            E::Reqwest(_) | E::Network(_) | E::Http(_) => UpdateError::Network(msg),
            E::ReleaseNotFound => UpdateError::NotAvailable("No published release found".into()),
            E::TargetNotFound(t) => {
                UpdateError::NotAvailable(format!("The latest release has no build for {t}"))
            }
            E::UnsupportedArch | E::UnsupportedOs | E::UnsupportedUpdateFormat => {
                UpdateError::NotAvailable(msg)
            }
            E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) => UpdateError::BadSignature(msg),
            E::Serialization(_) | E::Semver(_) | E::UrlParse(_) | E::EmptyEndpoints => {
                UpdateError::Manifest(msg)
            }
            _ => UpdateError::Install(msg),
        }
    }
}

/// Where updates come from. `Send + Sync`: checks and downloads run on a background thread.
pub trait UpdateSource: Send + Sync {
    /// The newest published release for this platform, whatever its version.
    fn latest(&self) -> Result<Release, UpdateError>;
    /// Download the installer and verify its signature. Never returns unverified bytes.
    fn download(
        &self,
        release: &Release,
        progress: &(dyn Fn(u64, Option<u64>) + Sync),
    ) -> Result<Vec<u8>, UpdateError>;
    /// Start installing verified bytes. On Windows this launches the installer and **exits the
    /// process**, so the caller must have saved everything first. On macOS it replaces the app
    /// bundle and returns; the caller relaunches.
    fn install(&self, release: &Release, bytes: Vec<u8>) -> Result<(), UpdateError>;
}

/// Whether to offer `latest` to a user running `current` who chose to skip `skipped`.
/// Only strictly newer, non-prerelease versions are offered (no downgrades, no betas).
pub fn should_offer(current: &Version, latest: &Version, skipped: Option<&str>) -> bool {
    latest > current
        && latest.pre.is_empty()
        && skipped.and_then(|s| Version::parse(s).ok()).as_ref() != Some(latest)
}

/// The real source: `latest.json` from GitHub Releases, verified with [`PUBLIC_KEY`].
pub struct GitHubReleases {
    url: String,
    pubkey: String,
}

impl GitHubReleases {
    pub fn new() -> GitHubReleases {
        GitHubReleases::with(MANIFEST_URL, PUBLIC_KEY)
    }

    pub fn with(url: &str, pubkey: &str) -> GitHubReleases {
        GitHubReleases {
            url: url.to_string(),
            pubkey: pubkey.trim().to_string(),
        }
    }
}

impl Default for GitHubReleases {
    fn default() -> Self {
        GitHubReleases::new()
    }
}

fn user_agent() -> String {
    concat!("KeyKeeper/", env!("CARGO_PKG_VERSION")).to_string()
}

impl UpdateSource for GitHubReleases {
    fn latest(&self) -> Result<Release, UpdateError> {
        let url = self
            .url
            .parse()
            .map_err(|e: url_error::ParseError| UpdateError::Manifest(e.to_string()))?;
        let config = Config {
            endpoints: vec![url],
            pubkey: self.pubkey.clone(),
            windows: None,
        };
        let updater = UpdaterBuilder::new(current_version(), config)
            // Report the latest release whatever its version; `should_offer` decides.
            .version_comparator(|_, _| true)
            .timeout(CHECK_TIMEOUT)
            .header("User-Agent", user_agent())?
            .build()?;
        let update = updater
            .check()?
            .ok_or_else(|| UpdateError::NotAvailable("No published release found".into()))?;
        let version =
            Version::parse(&update.version).map_err(|e| UpdateError::Manifest(e.to_string()))?;
        Ok(Release {
            version,
            notes: update.body.clone().unwrap_or_default(),
            date: update.date.map(|d| d.date().to_string()),
            update: Some(Box::new(update)),
        })
    }

    fn download(
        &self,
        release: &Release,
        progress: &(dyn Fn(u64, Option<u64>) + Sync),
    ) -> Result<Vec<u8>, UpdateError> {
        let mut update = release
            .update
            .as_deref()
            .cloned()
            .ok_or_else(|| UpdateError::Install("no download for this release".into()))?;
        update.timeout = Some(DOWNLOAD_TIMEOUT);
        let done = std::sync::atomic::AtomicU64::new(0);
        let bytes = update.download_extended(
            |chunk, total| {
                let n = done.fetch_add(chunk as u64, std::sync::atomic::Ordering::Relaxed);
                progress(n + chunk as u64, total);
            },
            || {},
        )?;
        Ok(bytes)
    }

    fn install(&self, release: &Release, bytes: Vec<u8>) -> Result<(), UpdateError> {
        let update = release
            .update
            .as_deref()
            .ok_or_else(|| UpdateError::Install("no installer for this release".into()))?;
        update.install(bytes)?;
        Ok(())
    }
}

/// `url::ParseError`, named without a direct `url` dependency.
mod url_error {
    pub use cargo_packager_updater::url::ParseError;
}

/// The no-network source for demo scenes and the self-test.
pub struct NoUpdates;

impl UpdateSource for NoUpdates {
    fn latest(&self) -> Result<Release, UpdateError> {
        Err(UpdateError::NotAvailable("Update checks are off".into()))
    }
    fn download(
        &self,
        _: &Release,
        _: &(dyn Fn(u64, Option<u64>) + Sync),
    ) -> Result<Vec<u8>, UpdateError> {
        Err(UpdateError::NotAvailable("Update checks are off".into()))
    }
    fn install(&self, _: &Release, _: Vec<u8>) -> Result<(), UpdateError> {
        Err(UpdateError::NotAvailable("Update checks are off".into()))
    }
}

pub fn real_source() -> Arc<dyn UpdateSource> {
    Arc::new(GitHubReleases::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cargo_packager_updater::{RemoteRelease, UpdateFormat};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn fixture(name: &str) -> Vec<u8> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/update")
            .join(name);
        std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    fn fixture_str(name: &str) -> String {
        String::from_utf8(fixture(name)).unwrap()
    }

    #[test]
    fn offers_only_newer_stable_unskipped_versions() {
        let cur = v("0.2.0");
        assert!(should_offer(&cur, &v("0.2.1"), None));
        assert!(should_offer(&cur, &v("0.10.0"), None), "semver, not text");
        assert!(should_offer(&cur, &v("1.0.0"), Some("0.3.0")));
        assert!(!should_offer(&cur, &v("0.2.0"), None), "same version");
        assert!(!should_offer(&cur, &v("0.1.9"), None), "no downgrades");
        assert!(
            !should_offer(&cur, &v("0.3.0-beta.1"), None),
            "no prereleases"
        );
        assert!(!should_offer(&cur, &v("0.3.0"), Some("0.3.0")), "skipped");
        assert!(should_offer(&cur, &v("0.3.0"), Some("garbage")));
    }

    #[test]
    fn embedded_public_key_is_a_minisign_key() {
        use base64::Engine;
        let text = base64::engine::general_purpose::STANDARD
            .decode(PUBLIC_KEY.trim())
            .expect("base64");
        let text = String::from_utf8(text).expect("utf-8");
        minisign_verify::PublicKey::decode(&text).expect("valid minisign public key");
        assert!(text.starts_with("untrusted comment:"));
    }

    /// The manifest the release workflow writes (see `.github/workflows/release.yml`).
    #[test]
    fn manifest_parses_for_every_platform() {
        let r: RemoteRelease = serde_json::from_str(&fixture_str("latest.json")).unwrap();
        assert_eq!(r.version, v("0.2.0"));
        assert!(r.notes.as_deref().is_some_and(|n| n.contains("What's new")));
        assert!(r.pub_date.is_some());
        for (target, ext, fmt) in [
            ("windows-x86_64", "_x64-setup.exe", "nsis"),
            ("macos-aarch64", "_aarch64.app.tar.gz", "app"),
            ("macos-x86_64", "_x64.app.tar.gz", "app"),
        ] {
            let url = r.download_url(target).unwrap().to_string();
            assert!(
                url.starts_with("https://github.com/Mx772/key-keeper/releases/download/v0.2.0/"),
                "{url}"
            );
            assert!(url.ends_with(ext), "{target}: {url}");
            assert!(!r.signature(target).unwrap().is_empty());
            let format = r.format(target).unwrap();
            assert_eq!(format.to_string(), fmt, "{target}");
        }
        assert!(matches!(
            r.format("windows-x86_64").unwrap(),
            UpdateFormat::Nsis
        ));
        assert!(r.download_url("linux-x86_64").is_err());
    }

    #[test]
    fn this_platform_has_a_manifest_entry() {
        let target = cargo_packager_updater::target().unwrap();
        let r: RemoteRelease = serde_json::from_str(&fixture_str("latest.json")).unwrap();
        if cfg!(any(windows, target_os = "macos")) {
            r.download_url(&target)
                .unwrap_or_else(|_| panic!("manifest key {target}"));
        }
    }

    /// Serves `routes` over HTTP/1.1 on 127.0.0.1 (one request per connection) until dropped.
    struct Server {
        base: String,
    }

    fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = routes.iter().find(|(p, _)| *p == path).map(|(_, b)| b);
                let _ = match body {
                    Some(b) => s
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                b.len()
                            )
                            .as_bytes(),
                        )
                        .and_then(|_| s.write_all(b)),
                    None => s.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    ),
                };
            }
        });
        Server { base }
    }

    /// A manifest pointing every platform at `/payload` with `signature`.
    fn local_manifest(base: &str, signature: &str) -> Vec<u8> {
        let entry = |fmt: &str| serde_json::json!({"url": format!("{base}/payload"), "signature": signature, "format": fmt});
        serde_json::to_vec(&serde_json::json!({
            "version": "9.9.9",
            "notes": "Test release",
            "pub_date": "2026-11-25T00:00:00Z",
            "platforms": {
                "windows-x86_64": entry("nsis"),
                "windows-aarch64": entry("nsis"),
                "macos-aarch64": entry("app"),
                "macos-x86_64": entry("app"),
                "linux-x86_64": entry("appimage"),
                "linux-aarch64": entry("appimage"),
            }
        }))
        .unwrap()
    }

    fn source_for(payload: Vec<u8>, pubkey: &str) -> (Server, GitHubReleases) {
        source_with(payload, &fixture_str("payload.bin.sig"), pubkey)
    }

    fn source_with(payload: Vec<u8>, sig: &str, pubkey: &str) -> (Server, GitHubReleases) {
        // The manifest names the payload server's address, so it's served separately.
        // (Server threads run until the test process exits.)
        let payload_srv = serve(vec![("/payload", payload)]);
        let manifest = local_manifest(&payload_srv.base, sig.trim());
        let manifest_srv = serve(vec![("/latest.json", manifest)]);
        let src = GitHubReleases::with(&format!("{}/latest.json", manifest_srv.base), pubkey);
        (manifest_srv, src)
    }

    /// Checks a real release artifact against the key built into the app, through the same
    /// download-and-verify path an update takes:
    /// `KEYKEEPER_VERIFY=<absolute path to installer> cargo test -p keykeeper-app verify_artifact -- --ignored`
    /// (the `.sig` must sit next to the file).
    #[test]
    #[ignore = "manual: verify a built artifact"]
    fn verify_artifact() {
        let path = std::env::var("KEYKEEPER_VERIFY").expect("set KEYKEEPER_VERIFY");
        let bytes = std::fs::read(&path).unwrap();
        let sig = std::fs::read_to_string(format!("{path}.sig")).unwrap();
        let (_srv, src) = source_with(bytes, &sig, PUBLIC_KEY);
        let release = src.latest().unwrap();
        src.download(&release, &|_, _| {})
            .expect("signed by the release key");
    }

    #[test]
    fn genuine_download_verifies() {
        let pubkey = fixture_str("test.key.pub");
        let (_srv, src) = source_for(fixture("payload.bin"), &pubkey);
        let release = src.latest().expect("manifest");
        assert_eq!(release.version, v("9.9.9"));
        assert_eq!(release.notes, "Test release");
        let seen = std::sync::Mutex::new(0u64);
        let bytes = src
            .download(&release, &|n, _| *seen.lock().unwrap() = n)
            .expect("signature verifies");
        assert_eq!(bytes, fixture("payload.bin"));
        assert_eq!(
            *seen.lock().unwrap(),
            bytes.len() as u64,
            "progress reported"
        );
    }

    #[test]
    fn tampered_download_is_refused() {
        let pubkey = fixture_str("test.key.pub");
        let mut tampered = fixture("payload.bin");
        tampered[0] ^= 1;
        let (_srv, src) = source_for(tampered, &pubkey);
        let release = src.latest().unwrap();
        let err = src.download(&release, &|_, _| {}).unwrap_err();
        assert!(matches!(err, UpdateError::BadSignature(_)), "{err:?}");
        assert!(err.to_string().contains("not installed"));
    }

    #[test]
    fn download_signed_by_another_key_is_refused() {
        // The real release key did not sign the test payload.
        let (_srv, src) = source_for(fixture("payload.bin"), PUBLIC_KEY);
        let release = src.latest().unwrap();
        let err = src.download(&release, &|_, _| {}).unwrap_err();
        assert!(matches!(err, UpdateError::BadSignature(_)), "{err:?}");
    }

    #[test]
    fn missing_manifest_and_dead_server_are_reported() {
        let srv = serve(vec![]);
        let src = GitHubReleases::with(&format!("{}/latest.json", srv.base), PUBLIC_KEY);
        assert!(matches!(src.latest(), Err(UpdateError::NotAvailable(_))));

        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let src = GitHubReleases::with(&format!("http://127.0.0.1:{port}/latest.json"), PUBLIC_KEY);
        assert!(matches!(src.latest(), Err(UpdateError::Network(_))));
    }
}
