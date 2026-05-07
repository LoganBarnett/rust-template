//! Authentication primitives.
//!
//! Two complementary flows live here:
//!
//! - **Session-based OIDC** for browser users: login / callback /
//!   logout handlers, the [`require_auth`] middleware, and provider
//!   discovery.  Handlers take `State<Option<Arc<CoreClient>>>` so
//!   they are decoupled from the downstream project's `AppState`.
//! - **JWT bearer tokens** for service / automation callers: see
//!   [`jwt`] for [`jwt::build_decoder`], [`jwt::ServiceClaims`], and
//!   re-exports of the `axum-jwt-auth` extractor.

pub mod discovery;
pub mod handlers;
pub mod jwt;
pub mod middleware;
pub mod types;

pub use discovery::{discover_oidc, OidcDiscoveryError};
pub use handlers::{callback_handler, login_handler, logout_handler};
pub use middleware::{current_user, require_auth};
pub use types::{AuthUser, CallbackQuery, OidcConfig};
