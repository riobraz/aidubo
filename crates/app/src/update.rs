//! Signed update check.
//!
//! Flow: fetch → parse → verify signature → compare version → pick the
//! artifact for this platform. A download additionally verifies size and
//! SHA-256 before the file is revealed. Nothing here installs or executes a
//! download.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::Sender;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};
use harness_protocol::{UpdateArtifact, UpdateManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANIFEST_ENV: &str = "RUST_HARNESS_UPDATE_MANIFEST";
pub const PUBLIC_KEY_ENV: &str = "RUST_HARNESS_UPDATE_PUBLIC_KEY";
pub const SETTINGS_ENV: &str = "RUST_HARNESS_UPDATE_SETTINGS";

/// Release builds can bake the manifest URL in at compile time; runtime
/// configuration wins so a test or a local build can point elsewhere.
pub const DEFAULT_MANIFEST_URL: Option<&str> = option_env!("RUST_HARNESS_UPDATE_MANIFEST");

/// Placeholder release key. Its private half was discarded when the constant
/// was created, so no manifest can be signed for this build until the release
/// pipeline replaces it. That fails closed: an unsigned update is never
/// offered.
const PINNED_PUBLIC_KEY: [u8; 32] = [
    0xc0, 0x12, 0xec, 0xf5, 0xa6, 0x5f, 0xfe, 0x3d, 0x94, 0x3e, 0x27, 0x93, 0xd9, 0x7c, 0x02, 0x45,
    0xca, 0x0b, 0x9b, 0xec, 0x5d, 0x91, 0x94, 0x18, 0x28, 0xaa, 0x2f, 0xe9, 0x2f, 0x21, 0x33, 0x3c,
];

const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    UpToDate {
        checked_at: i64,
    },
    Available {
        version: String,
        url: String,
        sha256: String,
        notes_url: Option<String>,
    },
    Unavailable {
        reason: String,
        checked_at: i64,
    },
}

impl UpdateState {
    /// One safe line for a log or a status strip. Never includes a URL.
    pub fn summary(&self) -> String {
        match self {
            Self::UpToDate { .. } => "up to date".into(),
            Self::Available { version, .. } => format!("version {version} is available"),
            Self::Unavailable { reason, .. } => format!("unavailable: {reason}"),
        }
    }
}

fn unavailable(reason: impl Into<String>, checked_at: i64) -> UpdateState {
    UpdateState::Unavailable {
        reason: reason.into(),
        checked_at,
    }
}

/// A pinned public key, or a test override in debug builds.
#[derive(Clone)]
pub struct Trust {
    key: VerifyingKey,
}

impl Trust {
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, String> {
        VerifyingKey::from_bytes(&bytes)
            .map(|key| Self { key })
            .map_err(|_| "This build's update key is not valid.".to_string())
    }

    pub fn pinned() -> Result<Self, String> {
        Self::from_bytes(PINNED_PUBLIC_KEY)
    }

    pub fn verify(&self, manifest: &UpdateManifest) -> Result<(), String> {
        let invalid = || "The update manifest signature is not valid.".to_string();
        let raw = STANDARD
            .decode(manifest.signature.trim())
            .map_err(|_| invalid())?;
        let signature = Signature::from_slice(&raw).map_err(|_| invalid())?;
        let payload = manifest.signature_payload()?;
        self.key
            .verify_strict(&payload, &signature)
            .map_err(|_| invalid())
    }
}

/// The compiled-in key, unless a debug build points the check at a test key.
/// Release builds ignore the override environment variable.
pub fn configured_trust() -> Result<Trust, String> {
    #[cfg(debug_assertions)]
    if let Ok(hex) = std::env::var(PUBLIC_KEY_ENV) {
        let bytes = parse_hex_key(&hex)?;
        return Trust::from_bytes(bytes);
    }
    Trust::pinned()
}

fn parse_hex_key(hex: &str) -> Result<[u8; 32], String> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err("The update key override must be 64 hex characters.".to_string());
    }
    let mut bytes = [0u8; 32];
    for (index, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let text = std::str::from_utf8(chunk)
            .map_err(|_| "The update key override is not hexadecimal.".to_string())?;
        bytes[index] = u8::from_str_radix(text, 16)
            .map_err(|_| "The update key override is not hexadecimal.".to_string())?;
    }
    Ok(bytes)
}

/// Compare dot-separated numeric versions. A pre-release suffix is ignored:
/// `0.2.0-beta` is not newer than `0.2.0`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let mut candidate = version_parts(candidate);
    let mut current = version_parts(current);
    let length = candidate.len().max(current.len());
    candidate.resize(length, 0);
    current.resize(length, 0);
    candidate > current
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .map(|part| {
            let digits: String = part
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect();
            digits.parse().unwrap_or(0)
        })
        .collect()
}

/// Verify a fetched manifest and decide what to report. Every failure is an
/// `Unavailable` state with a short, display-safe reason.
pub fn evaluate(raw: &[u8], current: &str, trust: &Trust, checked_at: i64) -> UpdateState {
    let manifest = match UpdateManifest::parse(raw) {
        Ok(manifest) => manifest,
        Err(reason) => return unavailable(reason, checked_at),
    };
    if let Err(reason) = trust.verify(&manifest) {
        return unavailable(reason, checked_at);
    }
    if !is_newer(&manifest.version, current) {
        return UpdateState::UpToDate { checked_at };
    }

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let Some(artifact) = manifest.artifact(os, arch) else {
        return unavailable(
            format!("This release has no build for {os}/{arch}."),
            checked_at,
        );
    };
    let url = artifact.url.clone();
    let sha256 = artifact.sha256.clone();
    UpdateState::Available {
        version: manifest.version,
        url,
        sha256,
        notes_url: manifest.notes_url,
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|_| "The update check could not start.".to_string())
}

pub async fn check(manifest_url: &str, current: &str, trust: &Trust) -> UpdateState {
    let checked_at = now_secs();
    let client = match client() {
        Ok(client) => client,
        Err(reason) => return unavailable(reason, checked_at),
    };
    let response = match client.get(manifest_url).send().await {
        Ok(response) => response,
        Err(_) => return unavailable("The update server could not be reached.", checked_at),
    };
    if !response.status().is_success() {
        return unavailable(
            format!("The update server answered {}.", response.status().as_u16()),
            checked_at,
        );
    }
    match response.bytes().await {
        Ok(bytes) => evaluate(&bytes, current, trust, checked_at),
        Err(_) => unavailable("The update response could not be read.", checked_at),
    }
}

/// Check whether a launch check is due. A recent check (even a failed one) is
/// not repeated for a day, so an offline machine is quiet.
pub fn should_check(settings: &UpdateSettings, now: i64) -> bool {
    settings.enabled
        && settings
            .last_checked_at
            .is_none_or(|last| now.saturating_sub(last) >= CHECK_INTERVAL_SECS)
}

/// Run a launch check on a background thread. No request is made when the
/// setting is off or when the last check is younger than 24 hours.
pub fn spawn_check(
    manifest_url: String,
    current: String,
    trust: Trust,
    sender: Sender<UpdateState>,
) {
    let spawned = std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            let now = now_secs();
            let mut settings = load_settings();
            if !should_check(&settings, now) {
                return;
            }
            let Some(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()
            else {
                tracing::warn!("could not start the update runtime");
                send_state(
                    &sender,
                    unavailable("The update check could not start.", now),
                );
                return;
            };
            let state = runtime.block_on(check(&manifest_url, &current, &trust));
            settings.last_checked_at = Some(now_secs());
            if save_settings(&settings).is_err() {
                tracing::warn!("could not persist the update check time");
            }
            send_state(&sender, state);
        });
    if spawned.is_err() {
        tracing::warn!("could not start the update check thread");
    }
}

fn send_state(sender: &Sender<UpdateState>, state: UpdateState) {
    if sender.send_blocking(state).is_err() {
        tracing::warn!("the update result had no listener");
    }
}

/// Start the launch check when a manifest URL and a usable key are
/// configured. Returns immediately.
pub fn spawn_launch_check() {
    let manifest_url = std::env::var(MANIFEST_ENV)
        .ok()
        .filter(|url| !url.trim().is_empty())
        .or_else(|| DEFAULT_MANIFEST_URL.map(str::to_string));
    let Some(manifest_url) = manifest_url else {
        return;
    };
    let trust = match configured_trust() {
        Ok(trust) => trust,
        Err(reason) => {
            tracing::info!(%reason, "update check skipped");
            return;
        }
    };

    let (sender, receiver) = async_channel::unbounded();
    spawn_check(
        manifest_url,
        env!("CARGO_PKG_VERSION").to_string(),
        trust,
        sender,
    );
    let spawned = std::thread::Builder::new()
        .name("update-report".into())
        .spawn(move || {
            while let Ok(state) = receiver.recv_blocking() {
                tracing::info!(status = %state.summary(), "update check finished");
            }
        });
    if spawned.is_err() {
        tracing::warn!("could not start the update report thread");
    }
}

/// Fetch an artifact, verify it, and save it. The caller decides whether to
/// reveal the file; nothing is executed.
pub async fn download_verified(
    artifact: &UpdateArtifact,
    directory: &Path,
) -> Result<PathBuf, String> {
    let client = client()?;
    let response = client
        .get(&artifact.url)
        .send()
        .await
        .map_err(|_| "The update download could not start.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "The update server answered {}.",
            response.status().as_u16()
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "The update download could not be read.".to_string())?;
    verify_bytes(artifact, &bytes)?;

    std::fs::create_dir_all(directory)
        .map_err(|_| "Could not create the Downloads directory.".to_string())?;
    let path = directory.join(download_name(&artifact.url));
    std::fs::write(&path, &bytes).map_err(|_| "Could not save the update.".to_string())?;
    Ok(path)
}

/// Size and digest checks, in that order.
pub fn verify_bytes(artifact: &UpdateArtifact, bytes: &[u8]) -> Result<(), String> {
    if artifact.size > 0 && bytes.len() as u64 != artifact.size {
        return Err("The downloaded file has the wrong size.".to_string());
    }
    let digest = sha256_hex(bytes);
    if !artifact.sha256_matches(&digest) {
        return Err("The downloaded file does not match its checksum.".to_string());
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        if write!(hex, "{byte:02x}").is_err() {
            return String::new();
        }
    }
    hex
}

fn download_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "RustHarness-update".to_string()
    } else {
        cleaned
    }
}

/// Show the downloaded file in the platform file manager. This is the
/// strongest action the app takes: it never opens or runs the file.
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        return Err("Revealing a download is not supported on this platform.".to_string());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let status = command
            .status()
            .map_err(|_| "Could not open the Downloads folder.".to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err("Could not open the Downloads folder.".to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSettings {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub last_checked_at: Option<i64>,
}

fn enabled_by_default() -> bool {
    true
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            last_checked_at: None,
        }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(SETTINGS_ENV) {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    {
        home.map(|home| home.join("Library/Application Support/Rust Harness/updates.json"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(config).join("rust-harness/updates.json"));
        }
        home.map(|home| home.join(".config/rust-harness/updates.json"))
    }
}

pub fn load_settings() -> UpdateSettings {
    let Some(path) = settings_path() else {
        return UpdateSettings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| {
            tracing::warn!("ignoring an invalid update settings file");
            UpdateSettings::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => UpdateSettings::default(),
        Err(_) => {
            tracing::warn!("could not read the update settings file");
            UpdateSettings::default()
        }
    }
}

pub fn save_settings(settings: &UpdateSettings) -> Result<(), String> {
    let Some(path) = settings_path() else {
        return Err("no settings directory is available".to_string());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "could not create the settings directory".to_string())?;
    }
    let raw = serde_json::to_vec_pretty(settings)
        .map_err(|_| "could not encode the update settings".to_string())?;
    std::fs::write(&path, raw).map_err(|_| "could not write the settings file".to_string())
}

pub fn downloads_directory() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer as _, SigningKey};
    use serde_json::{Value, json};

    use super::*;

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn artifact_for_current_platform() -> Value {
        json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "url": "https://example.test/RustHarness-0.2.0.dmg",
            "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "size": 4,
        })
    }

    fn signed(version: &str, artifacts: Vec<Value>) -> Vec<u8> {
        let mut value = json!({
            "version": version,
            "channel": "stable",
            "artifacts": artifacts,
            "notes_url": "https://example.test/notes",
            "issued_at": 1_762_732_800,
        });
        let mut manifest: UpdateManifest =
            serde_json::from_value(value.clone()).expect("manifest struct");
        manifest.signature = String::new();
        let payload = manifest.signature_payload().expect("payload");
        let signature = signing_key().sign(&payload);
        value["signature"] = json!(STANDARD.encode(signature.to_bytes()));
        serde_json::to_vec(&value).expect("signed manifest")
    }

    fn trust() -> Trust {
        Trust::from_bytes(signing_key().verifying_key().to_bytes()).expect("trust")
    }

    #[test]
    fn update_signature_rejects_tampered_payload() {
        let raw = signed("0.2.0", vec![artifact_for_current_platform()]);
        let state = evaluate(&raw, "0.1.0", &trust(), 100);
        assert!(matches!(state, UpdateState::Available { .. }), "{state:?}");

        let mut value: Value = serde_json::from_slice(&raw).expect("json");
        value["notes_url"] = json!("https://evil.test/notes");
        let tampered = serde_json::to_vec(&value).expect("encode");
        let state = evaluate(&tampered, "0.1.0", &trust(), 100);
        let UpdateState::Unavailable { reason, .. } = state else {
            panic!("tampered manifest must not be available");
        };
        assert!(reason.contains("signature"), "{reason}");
    }

    #[test]
    fn update_signature_rejects_tampered_hash() {
        let raw = signed("0.2.0", vec![artifact_for_current_platform()]);
        let mut value: Value = serde_json::from_slice(&raw).expect("json");
        value["artifacts"][0]["sha256"] = json!("f".repeat(64));
        let tampered = serde_json::to_vec(&value).expect("encode");
        let state = evaluate(&tampered, "0.1.0", &trust(), 100);
        assert!(matches!(state, UpdateState::Unavailable { .. }));
    }

    #[test]
    fn update_version_compare_is_numeric() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("0.2", "0.2.0"));
        assert!(is_newer("v0.3.0", "0.2.0"));
    }

    #[test]
    fn update_missing_arch_is_unavailable() {
        let mut artifact = artifact_for_current_platform();
        artifact["arch"] = json!("definitely-not-this-arch");
        let raw = signed("0.2.0", vec![artifact]);
        let state = evaluate(&raw, "0.1.0", &trust(), 100);
        let UpdateState::Unavailable { reason, .. } = state else {
            panic!("a missing platform build must not be available");
        };
        assert!(reason.contains("no build"), "{reason}");
    }

    #[test]
    fn update_download_verifies_hash() {
        let bytes = b"dmg!";
        let digest = sha256_hex(bytes);
        let artifact = UpdateArtifact {
            os: "macos".into(),
            arch: "aarch64".into(),
            url: "https://example.test/app.dmg".into(),
            sha256: digest.clone(),
            size: bytes.len() as u64,
            extra: Default::default(),
        };
        verify_bytes(&artifact, bytes).expect("matching download");

        let mut wrong_hash = artifact.clone();
        wrong_hash.sha256 = "0".repeat(64);
        assert!(verify_bytes(&wrong_hash, bytes).is_err());

        let mut wrong_size = artifact;
        wrong_size.size = bytes.len() as u64 + 1;
        assert!(verify_bytes(&wrong_size, bytes).is_err());
    }

    #[test]
    fn update_check_runs_at_most_once_a_day() {
        let now = 2_000_000_000;
        assert!(should_check(&UpdateSettings::default(), now));
        assert!(!should_check(
            &UpdateSettings {
                enabled: false,
                last_checked_at: None,
            },
            now
        ));
        assert!(!should_check(
            &UpdateSettings {
                enabled: true,
                last_checked_at: Some(now - 10),
            },
            now
        ));
        assert!(should_check(
            &UpdateSettings {
                enabled: true,
                last_checked_at: Some(now - CHECK_INTERVAL_SECS),
            },
            now
        ));
    }

    #[test]
    fn download_names_strip_paths() {
        assert_eq!(
            download_name("https://x.test/a/RustHarness.dmg?token=1"),
            "RustHarness.dmg"
        );
        assert_eq!(download_name("https://x.test/"), "RustHarness-update");
        assert_eq!(download_name("https://x.test/../"), "RustHarness-update");
    }
}
