//! One module per API area. Every module adds an `impl SessionApi` block, so
//! a new feature only touches its own file.

pub(crate) mod catalogs;
pub(crate) mod diff;
pub(crate) mod files;
pub(crate) mod permissions;
pub(crate) mod prompt;
pub(crate) mod sessions;
pub(crate) mod transcript;
pub(crate) mod vcs;
