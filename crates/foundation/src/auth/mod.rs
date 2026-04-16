//! OIDC authentication — login, callback, logout, session guard, and
//! provider discovery.
//!
//! Handlers take `State<Option<Arc<CoreClient>>>` so they are decoupled
//! from the downstream project's `AppState`.  Use `FromRef` to extract
//! this slice from your own state type.

pub mod discovery;
pub mod handlers;
pub mod middleware;
pub mod types;

pub use discovery::{discover_oidc, OidcDiscoveryError};
pub use handlers::{callback_handler, login_handler, logout_handler};
pub use middleware::{current_user, require_auth};
pub use types::{AuthUser, CallbackQuery, OidcConfig};
