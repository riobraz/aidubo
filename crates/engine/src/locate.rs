//! Locating the `opencode` binary.
//!
//! GUI applications on macOS do not inherit the shell `PATH`, so the lookup
//! includes explicit configuration, the app bundle's `Contents/MacOS` and
//! `Contents/Resources`, `PATH`, and the common install locations for
//! Bun/Homebrew. A bundled binary is checked against the SHA-256 manifest the
//! packager writes next to it.

use std::env;
use std::ffi::OsStr;
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
    #[error("The bundled OpenCode binary has no checksum manifest. Reinstall Rust Harness.")]
    ManifestMissing,
}

/// Locate the `opencode` executable.
///
/// `explicit` wins; then the app bundle (`Contents/MacOS`, `Contents/Resources`
/// on macOS, `usr/lib/rust-harness` on Linux, verifying the recorded checksum);
/// then `RUST_HARNESS_OPENCODE_BIN`; then `PATH`; then well-known install
/// locations. The bundle comes before the environment so an installed release
/// keeps its verified sidecar; an explicit path remains the escape hatch.
pub fn locate_binary(explicit: Option<&Path>) -> Result<PathBuf, LocateError> {
    let env_binary = env::var_os("RUST_HARNESS_OPENCODE_BIN").map(PathBuf::from);
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let path_var = env::var_os("PATH");
    let home = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());

    locate_from(
        explicit,
        env_binary.as_deref(),
        exe_dir.as_deref(),
        path_var.as_deref(),
        home.as_deref(),
    )
}

fn locate_from(
    explicit: Option<&Path>,
    env_binary: Option<&Path>,
    exe_dir: Option<&Path>,
    path_var: Option<&OsStr>,
    home: Option<&Path>,
) -> Result<PathBuf, LocateError> {
    if let Some(path) = explicit {
        return verify(path).ok_or_else(|| LocateError::ExplicitMissing(path.to_path_buf()));
    }

    if let Some(dir) = exe_dir
        && let Some(path) = bundled(dir)?
    {
        return Ok(path);
    }

    if let Some(path) = env_binary {
        return verify(path).ok_or_else(|| LocateError::ExplicitMissing(path.to_path_buf()));
    }

    if let Some(path_var) = path_var {
        for dir in env::split_paths(path_var) {
            if let Some(path) = verify(&dir.join(exe_name())) {
                return Ok(path);
            }
        }
    }

    if let Some(home) = home {
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

/// The sidecar shipped inside the app, if present.
///
/// Packaged layouts keep it in `Contents/Resources` (macOS) or
/// `usr/lib/rust-harness` (Linux); both fail closed when the checksum manifest
/// is missing. A binary dropped next to the executable remains a developer
/// affordance, so its manifest stays optional.
fn bundled(exe_dir: &Path) -> Result<Option<PathBuf>, LocateError> {
    let candidates = [
        (exe_dir.join(exe_name()), false),
        (exe_dir.join("../Resources").join(exe_name()), true),
        (exe_dir.join("../lib/rust-harness").join(exe_name()), true),
    ];
    for (candidate, manifest_required) in candidates {
        let Some(path) = verify(&candidate) else {
            continue;
        };
        verify_checksum(&path, manifest_required)?;
        // Resolve the `..` in the packaged lookups so logs and errors show a
        // clean path.
        return Ok(Some(fs::canonicalize(&path).unwrap_or(path)));
    }
    Ok(None)
}

/// Check a bundled binary against the SHA-256 manifest written by the packager.
///
/// A developer binary beside the executable may have no manifest; a packaged
/// binary without one fails closed.
fn verify_checksum(binary: &Path, required: bool) -> Result<(), LocateError> {
    let manifest = binary.with_extension("sha256");
    let recorded = match fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return if required {
                Err(LocateError::ManifestMissing)
            } else {
                Ok(())
            };
        }
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
    fn dev_sibling_binary_without_manifest_is_accepted() {
        let root = temp_dir("bundle-no-manifest");
        let macos = root.join("MacOS");
        fs::create_dir_all(&macos).unwrap();

        // A developer drop beside the executable; packaged resource dirs are
        // covered by the fail-closed test below.
        let sidecar = macos.join("opencode");
        fs::write(&sidecar, b"fake sidecar").unwrap();

        assert_eq!(
            bundled(&macos).unwrap(),
            Some(fs::canonicalize(&sidecar).unwrap())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_rejects_wrong_hash() {
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

    #[test]
    fn manifest_missing_blocks_start() {
        let root = temp_dir("bundle-missing-manifest");
        let macos = root.join("MacOS");
        let resources = root.join("Resources");
        let path_dir = root.join("bin");
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&resources).unwrap();
        fs::create_dir_all(&path_dir).unwrap();
        fs::write(resources.join("opencode"), b"fake sidecar").unwrap();
        fs::write(path_dir.join("opencode"), b"path sidecar").unwrap();

        // The packaged binary blocks startup and does not fall through to PATH.
        let path_var = env::join_paths([&path_dir]).unwrap();
        let error = locate_from(None, None, Some(&macos), Some(path_var.as_os_str()), None)
            .expect_err("a packaged binary without a manifest must block");
        assert!(matches!(error, LocateError::ManifestMissing));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn locate_prefers_bundle_then_env_then_path() {
        let root = temp_dir("locate-order");
        let macos = root.join("MacOS");
        let resources = root.join("Resources");
        let path_dir = root.join("bin");
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&resources).unwrap();
        fs::create_dir_all(&path_dir).unwrap();

        let bundled_sidecar = resources.join("opencode");
        fs::write(&bundled_sidecar, b"bundled").unwrap();
        fs::write(
            resources.join("opencode.sha256"),
            format!("{}  opencode\n", sha256(&bundled_sidecar).unwrap()),
        )
        .unwrap();
        let env_sidecar = root.join("env-opencode");
        fs::write(&env_sidecar, b"env").unwrap();
        let path_sidecar = path_dir.join("opencode");
        fs::write(&path_sidecar, b"path").unwrap();
        let path_var = env::join_paths([&path_dir]).unwrap();

        let found = locate_from(
            None,
            Some(&env_sidecar),
            Some(&macos),
            Some(path_var.as_os_str()),
            None,
        )
        .expect("bundle");
        assert_eq!(found, fs::canonicalize(&bundled_sidecar).unwrap());

        fs::remove_dir_all(&resources).unwrap();
        let found = locate_from(
            None,
            Some(&env_sidecar),
            Some(&macos),
            Some(path_var.as_os_str()),
            None,
        )
        .expect("environment");
        assert_eq!(found, env_sidecar);

        let found =
            locate_from(None, None, Some(&macos), Some(path_var.as_os_str()), None).expect("path");
        assert_eq!(found, path_sidecar);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn linux_bundle_directory_is_found() {
        let root = temp_dir("linux-bundle");
        let bin = root.join("usr/bin");
        let lib = root.join("usr/lib/rust-harness");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&lib).unwrap();

        let sidecar = lib.join("opencode");
        fs::write(&sidecar, b"linux sidecar").unwrap();
        fs::write(
            lib.join("opencode.sha256"),
            format!("{}  opencode\n", sha256(&sidecar).unwrap()),
        )
        .unwrap();

        assert_eq!(
            bundled(&bin).unwrap(),
            Some(fs::canonicalize(&sidecar).unwrap())
        );
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
