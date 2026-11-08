//! Locating the `opencode` binary.
//!
//! GUI applications on macOS do not inherit the shell `PATH`, so the lookup
//! includes explicit configuration, the app bundle's `Contents/MacOS` and
//! `Contents/Resources`, `PATH`, and the common install locations for
//! Bun/Homebrew. A bundled binary is checked against the SHA-256 manifest the
//! packager writes next to it.

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum LocateError {
    #[error("OpenCode binary not found. Install `opencode`, or set RUST_HARNESS_OPENCODE_BIN.")]
    NotFound,
    #[error("OpenCode binary was set to {0}, but that path does not exist")]
    ExplicitMissing(PathBuf),
    #[error("The bundled OpenCode binary failed its integrity check. Reinstall Rust Harness.")]
    ChecksumMismatch,
}

/// Locate the `opencode` executable.
///
/// `explicit` wins; then `RUST_HARNESS_OPENCODE_BIN`; then the app bundle
/// (`Contents/MacOS`, then `Contents/Resources`, verifying the recorded
/// checksum); then `PATH`; then well-known install locations.
pub fn locate_binary(explicit: Option<&Path>) -> Result<PathBuf, LocateError> {
    if let Some(path) = explicit {
        return verify(path).ok_or_else(|| LocateError::ExplicitMissing(path.to_path_buf()));
    }

    if let Some(path) = env::var_os("RUST_HARNESS_OPENCODE_BIN") {
        let path = PathBuf::from(path);
        return verify(&path).ok_or(LocateError::ExplicitMissing(path));
    }

    if let Ok(current_exe) = env::current_exe()
        && let Some(dir) = current_exe.parent()
        && let Some(path) = bundled(dir)?
    {
        return Ok(path);
    }

    if let Some(path_var) = env::var_os("PATH") {
        for dir in env::split_paths(&path_var) {
            if let Some(path) = verify(&dir.join(exe_name())) {
                return Ok(path);
            }
        }
    }

    if let Some(home) = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        for relative in [".bun/bin/opencode", ".local/bin/opencode"] {
            if let Some(path) = verify(&home.join(relative)) {
                return Ok(path);
            }
        }
    }

    for candidate in ["/opt/homebrew/bin/opencode", "/usr/local/bin/opencode"] {
        if let Some(path) = verify(Path::new(candidate)) {
            return Ok(path);
        }
    }

    Err(LocateError::NotFound)
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "opencode.exe"
    } else {
        "opencode"
    }
}

fn verify(path: &Path) -> Option<PathBuf> {
    std::fs::metadata(path)
        .ok()?
        .is_file()
        .then(|| path.to_path_buf())
}

/// The sidecar shipped inside the app bundle, if present.
///
/// The architecture keeps it in `Contents/Resources`; `Contents/MacOS` is still
/// accepted so a developer can drop a binary next to `target/debug`'s binary.
fn bundled(exe_dir: &Path) -> Result<Option<PathBuf>, LocateError> {
    let resources = exe_dir.join("../Resources");
    for candidate in [exe_dir.join(exe_name()), resources.join(exe_name())] {
        let Some(path) = verify(&candidate) else {
            continue;
        };
        verify_checksum(&path)?;
        // Resolve the `..` in the Resources lookup so logs and errors show a
        // clean path.
        return Ok(Some(fs::canonicalize(&path).unwrap_or(path)));
    }
    Ok(None)
}

/// Check a bundled binary against the SHA-256 manifest written by the packager.
///
/// Development builds have no manifest, so a missing file is allowed; a
/// present-but-unreadable or mismatching manifest is an error.
fn verify_checksum(binary: &Path) -> Result<(), LocateError> {
    let manifest = binary.with_extension("sha256");
    let recorded = match fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(LocateError::ChecksumMismatch),
    };
    let Some(expected) = recorded.split_whitespace().next() else {
        return Err(LocateError::ChecksumMismatch);
    };
    let actual = sha256(binary).map_err(|_| LocateError::ChecksumMismatch)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(LocateError::ChecksumMismatch);
    }
    Ok(())
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "rust-harness-{name}-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::write(&path, b"").unwrap();
        path
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    #[test]
    fn explicit_file_wins() {
        let path = temp_file("explicit");
        let found = locate_binary(Some(&path)).unwrap();
        assert_eq!(found, path);
        let _ = std::fs::remove_file(found);
    }

    #[test]
    fn missing_explicit_path_is_reported() {
        let path = env::temp_dir().join("rust-harness-does-not-exist/opencode");
        assert!(matches!(
            locate_binary(Some(&path)),
            Err(LocateError::ExplicitMissing(missing)) if missing == path
        ));
    }

    #[test]
    fn bundle_resources_binary_is_found_and_verified() {
        let root = temp_dir("bundle-ok");
        let macos = root.join("MacOS");
        let resources = root.join("Resources");
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&resources).unwrap();

        let sidecar = resources.join("opencode");
        fs::write(&sidecar, b"fake sidecar").unwrap();
        let digest = sha256(&sidecar).unwrap();
        fs::write(
            resources.join("opencode.sha256"),
            format!("{digest}  opencode\n"),
        )
        .unwrap();

        assert_eq!(
            bundled(&macos).unwrap(),
            Some(fs::canonicalize(&sidecar).unwrap())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bundled_binary_without_manifest_is_accepted() {
        let root = temp_dir("bundle-no-manifest");
        let macos = root.join("MacOS");
        let resources = root.join("Resources");
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&resources).unwrap();

        let sidecar = resources.join("opencode");
        fs::write(&sidecar, b"fake sidecar").unwrap();

        assert_eq!(
            bundled(&macos).unwrap(),
            Some(fs::canonicalize(&sidecar).unwrap())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bundled_binary_with_wrong_checksum_is_rejected() {
        let root = temp_dir("bundle-tampered");
        let macos = root.join("MacOS");
        let resources = root.join("Resources");
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&resources).unwrap();

        fs::write(resources.join("opencode"), b"fake sidecar").unwrap();
        fs::write(resources.join("opencode.sha256"), "0".repeat(64)).unwrap();

        assert!(matches!(
            bundled(&macos),
            Err(LocateError::ChecksumMismatch)
        ));
        let _ = fs::remove_dir_all(root);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "rust-harness-dir-{name}-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
