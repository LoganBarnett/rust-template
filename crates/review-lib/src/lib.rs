//! The review engine.
//!
//! A review compares the working tree against a base, sends only what it has
//! not already judged to a nested reviewer, and records the verdict per file
//! so the next pass can skip what has not moved.  The front-end decides how to
//! render the result; everything else lives here.

pub mod base;
pub mod error;
pub mod fingerprint;
pub mod markup;
pub mod patch;
pub mod prompt;
pub mod record;
pub mod review;
pub mod reviewer;
pub mod worktree;

pub use base::{Base, DiffMode};
pub use error::ReviewError;
pub use markup::Markup;
pub use record::Standing;
pub use review::{run, Outcome, Request};
pub use reviewer::Finding;

// Re-exported so the front-end takes every leaf type its staged config names
// from this crate, the way the sibling front-ends do, rather than some from
// here and some from foundation directly.
pub use rust_template_foundation::logging::{LogFormat, LogLevel};
