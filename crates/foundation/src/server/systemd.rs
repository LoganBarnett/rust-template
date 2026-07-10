//! Systemd readiness notification and watchdog heartbeats.
//!
//! systemd is a Unix-only facility, so the real `sd_notify` integration
//! compiles only on Unix.  A cross-compiled server for a non-Unix target (a
//! Windows build) gets the no-op stubs below, which share the same signatures,
//! so callers need no `cfg` of their own and simply perform no systemd
//! signalling there.

#[cfg(unix)]
pub use unix::{notify_ready, spawn_watchdog};

#[cfg(not(unix))]
pub use stub::{notify_ready, spawn_watchdog};

#[cfg(unix)]
mod unix {
  use std::time::Duration;
  use tracing::{info, warn};

  /// Notify systemd that the service is ready to accept connections.  Safe to
  /// call when NOTIFY_SOCKET is unset; sd_notify returns Ok(()) silently in
  /// that case.
  pub fn notify_ready() {
    if let Err(e) = sd_notify::notify(&[sd_notify::NotifyState::Ready]) {
      warn!("sd-notify ready signal failed: {}", e);
    }
  }

  /// Spawn a background task that sends watchdog heartbeats to systemd if
  /// WATCHDOG_USEC is set in the environment.  The task runs for the lifetime
  /// of the Tokio runtime and is dropped automatically during shutdown.
  pub fn spawn_watchdog() {
    let Some(interval) = watchdog_interval() else {
      return;
    };
    info!("Watchdog enabled; sending heartbeats every {:?}", interval);
    tokio::spawn(async move {
      let mut ticker = tokio::time::interval(interval);
      loop {
        ticker.tick().await;
        if let Err(e) = sd_notify::notify(&[sd_notify::NotifyState::Watchdog]) {
          warn!("sd-notify watchdog heartbeat failed: {}", e);
        }
      }
    });
  }

  /// Returns half of the watchdog timeout, or None when the watchdog is not
  /// configured.  Uses sd_notify::watchdog_enabled, which validates
  /// WATCHDOG_PID in addition to WATCHDOG_USEC so spurious inherited env vars
  /// are ignored.  Pinging at half the interval provides a safety margin before
  /// systemd declares the service dead.
  fn watchdog_interval() -> Option<Duration> {
    sd_notify::watchdog_enabled().map(|timeout| timeout / 2)
  }
}

#[cfg(not(unix))]
mod stub {
  /// No-op off Unix: systemd's readiness protocol does not exist there, so a
  /// cross-compiled server signals nothing.
  pub fn notify_ready() {}

  /// No-op off Unix: see `notify_ready`.  No watchdog task is spawned.
  pub fn spawn_watchdog() {}
}
