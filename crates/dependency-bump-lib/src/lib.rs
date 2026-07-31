//! rust-template-dependency-bump-lib — the engine behind the scheduled
//! dependency-bump flow.
//!
//! Replaces Dependabot as the update signal: `cargo update` consults the
//! crates index directly, so no bot PR, comment command, or GitHub product
//! surface is involved.  The engine owns the working-tree transformation —
//! audit-informed classification, the whole-workspace update, hold
//! re-pinning, changelog composition, and the TSV report — while the
//! scheduled workflow (reusable-dependency-bump.yml) owns everything with a
//! remote side effect: branch, commit, PR, CI dispatch, merge.
//!
//! Policy lives in the workspace manifest under
//! `[workspace.metadata.dependency-bump]`; see [`holds`] for the v1 hold
//! table and tasks.org for the planned coupled/conditional rules.

mod audit;
mod compose;
mod error;
mod holds;
mod lockfile;
mod run;

pub use audit::Advisories;
pub use compose::{Entry, Heading};
pub use error::DependencyBumpError;
pub use holds::Hold;
pub use lockfile::Bump;
pub use run::{run, AppliedBump, BumpOutcome, RunOptions};
pub use rust_template_foundation::logging::{LogFormat, LogLevel};
