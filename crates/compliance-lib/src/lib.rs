//! Compliance engine for projects spawned from rust-template.
//!
//! The engine reads the spawn registry (`config.json`), a declarative check
//! manifest (`compliance-checks.toml`), and each spawn's provenance
//! (`rust-template.json`), then runs every check against every non-archived
//! spawn whose directory exists.  Each check has a stable symbolic id that is
//! surfaced on failure, and a spawn may opt out of named checks via the
//! `compliance-ignores` key in its provenance file.
//!
//! The library is consumed by the `rust-template-compliance-cli` front-end,
//! which renders the [`run::RunReport`] as human or JSON output.

pub mod check;
pub mod error;
pub mod manifest;
pub mod org;
pub mod pins;
pub mod provenance;
pub mod registry;
pub mod run;

// Re-exported so the cli's staged config can name the logging enums without
// declaring a foundation dependency edge of its own.
pub use rust_template_foundation::logging::{LogFormat, LogLevel};

pub use check::CheckOutcome;
pub use error::ComplianceError;
pub use run::{
  run, CheckResult, RunOptions, RunReport, SpawnReport, SpawnStatus,
};
