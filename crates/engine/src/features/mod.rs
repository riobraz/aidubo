//! One module per API area. Every module adds an `impl SessionApi` block, so
//! a new feature only touches its own file.

pub(crate) mod catalogs;
pub(crate) mod context;
pub(crate) mod diff;
pub(crate) mod files;
pub(crate) mod form;
pub(crate) mod inbox;
pub(crate) mod mcp;
pub(crate) mod permissions;
pub(crate) mod plugin;
pub(crate) mod prompt;
pub(crate) mod provider;
pub(crate) mod revert;
pub(crate) mod saved_permissions;
pub(crate) mod schedule;
pub(crate) mod sessions;
pub(crate) mod transcript;
pub(crate) mod usage;
pub(crate) mod vcs;
