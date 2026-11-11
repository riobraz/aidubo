//! UI-facing state with no GPUI or network dependency.
//!
//! Keeping these types pure makes the interesting transitions testable in a
//! plain `cargo test` run. Each store lives in its own module and is re-exported
//! here, so callers keep using `harness_core::SessionStore` and friends.

mod composer;
mod context;
mod diff;
mod form;
mod inbox;
mod mcp;
mod notify;
mod permissions;
mod plugin;
mod provider;
mod revert;
mod saved_permissions;
mod schedule;
mod sessions;
mod status;
mod transcript;
mod usage;
mod util;
mod vcs;

pub use composer::*;
pub use context::*;
pub use diff::*;
pub use form::*;
pub use inbox::*;
pub use mcp::*;
pub use notify::*;
pub use permissions::*;
pub use plugin::*;
pub use provider::*;
pub use revert::*;
pub use saved_permissions::*;
pub use schedule::*;
pub use sessions::*;
pub use status::*;
pub use transcript::*;
pub use usage::*;
pub use util::*;
pub use vcs::*;

#[cfg(test)]
mod tests;
