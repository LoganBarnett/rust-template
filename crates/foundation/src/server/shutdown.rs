//! Graceful shutdown signal that resolves on SIGTERM or Ctrl+C.

use std::io;
use tracing::info;

/// Resolves when the process receives SIGTERM or Ctrl+C, or returns
/// `Err` if the platform signal handler cannot be installed.  Wrap
/// this in an async block that logs and resolves to `()` before
/// passing to `axum::serve(...).with_graceful_shutdown(...)`.
pub async fn shutdown_signal() -> Result<(), io::Error> {
  // On non-unix platforms the SIGTERM branch is a never-resolving
  // future, so installation cannot fail there.
  #[cfg(unix)]
  let mut terminate =
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

  let ctrl_c = tokio::signal::ctrl_c();

  #[cfg(unix)]
  {
    tokio::select! {
      result = ctrl_c => {
        result?;
        info!("Received Ctrl+C, shutting down gracefully");
      },
      _ = terminate.recv() => {
        info!("Received SIGTERM, shutting down gracefully");
      },
    }
  }

  #[cfg(not(unix))]
  {
    ctrl_c.await?;
    info!("Received Ctrl+C, shutting down gracefully");
  }

  Ok(())
}
