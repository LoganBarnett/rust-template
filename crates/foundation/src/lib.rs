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
//!   integration, CLI logging, `CliApp` trait, and `#[foundation_main]`.
//! - **`server`** — Health registry, metrics endpoint, OpenAPI/Scalar
//!   helpers, SPA fallback, systemd notify/watchdog, server logging.
//! - **`auth`** (implies `server` + `cli`) — OIDC login/callback/logout
//!   handlers, `require_auth` middleware, `Server` runner, `ServerApp`
//!   trait, and `BaseServerState`.

pub mod config;
pub mod logging;

#[cfg(feature = "cli")]
pub mod app;

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

// Re-export proc macros so users write
// `use rust_template_foundation::main` and
// `use rust_template_foundation::MergeConfig`.
#[cfg(feature = "cli")]
pub use rust_template_foundation_derive::foundation_main as main;
#[cfg(feature = "cli")]
pub use rust_template_foundation_derive::MergeConfig;

// Re-export CliApp at crate root for CLI apps.
#[cfg(feature = "cli")]
pub use app::CliApp;

// Re-export key runner types at crate root for server apps.
#[cfg(feature = "auth")]
pub use server::runner::{
  BaseServerState, Server, ServerApp, ServerError, ServerRunConfig,
};
