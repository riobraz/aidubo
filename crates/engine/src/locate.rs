//! Locating the `opencode` binary.
//!
//! GUI applications on macOS do not inherit the shell `PATH`, so the lookup
//! includes explicit configuration, a binary next to the app, `PATH`, and the
//! common install locations for Bun/Homebrew.

use std::env;
use std::path::{Path, PathBuf};

use directories::BaseDirs;

#[derive(Debug, thiserror::Error)]
pub enum LocateError {
    #[error("OpenCode binary not found. Install `opencode`, or set RUST_HARNESS_OPENCODE_BIN.")]
    NotFound,
    #[error("OpenCode binary was set to {0}, but that path does not exist")]
    ExplicitMissing(PathBuf),
}

/// Locate the `opencode` executable.
///
/// `explicit` wins; then `RUST_HARNESS_OPENCODE_BIN`; then a sibling of the
/// current executable (for bundled builds); then `PATH`; then well-known
/// install locations.
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
        && let Some(path) = verify(&dir.join(exe_name()))
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
}
