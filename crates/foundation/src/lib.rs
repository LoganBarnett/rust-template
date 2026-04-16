//! Shared infrastructure for projects spawned from `rust-template`.
//!
//! This crate extracts the generic plumbing (config loading, logging,
//! health checks, metrics, OpenAPI, OIDC auth, systemd integration) so
//! downstream projects consume it as a git dependency and receive
//! improvements via `cargo update`.
//!
//! # Feature flags
//!
//! - **`cli`** — `CommonCli` / `CommonConfigFile` structs for `clap`
//!   integration, plus CLI logging initialisation.
//! - **`server`** — Health registry, metrics endpoint, OpenAPI/Scalar
//!   helpers, SPA fallback, systemd notify/watchdog, server logging.
//! - **`auth`** (implies `server`) — OIDC login/callback/logout
//!   handlers and `require_auth` middleware.

pub mod config;
pub mod logging;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "auth")]
pub mod auth;

/// Convenience re-exports used by most downstream crates.
pub mod prelude {
  pub use crate::config::find_config_file;
  pub use crate::config::load_toml;
  pub use crate::logging::{LogFormat, LogLevel};
}
