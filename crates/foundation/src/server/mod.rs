//! Server infrastructure: health checks, metrics, OpenAPI, SPA fallback,
//! graceful shutdown, and systemd integration.

pub mod health;
pub mod metrics;
pub mod openapi;
pub mod shutdown;
pub mod spa;
pub mod systemd;
