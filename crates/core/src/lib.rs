//! UI-facing state with no GPUI or network dependency.
//!
//! Keeping these types pure makes the interesting transitions testable in a
//! plain `cargo test` run. Each store lives in its own module and is re-exported
//! here, so callers keep using `harness_core::SessionStore` and friends.

mod composer;
mod diff;
mod permissions;
mod sessions;
mod status;
mod transcript;
mod util;
mod vcs;

pub use composer::*;
pub use diff::*;
pub use permissions::*;
pub use sessions::*;
pub use status::*;
pub use transcript::*;
pub use util::*;
pub use vcs::*;

#[cfg(test)]
mod tests;
