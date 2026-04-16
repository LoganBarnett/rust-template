//! OIDC authentication — thin re-export of foundation auth handlers.
//!
//! The foundation handlers take `State<Option<Arc<CoreClient>>>` which is
//! extracted from `AppState` via the `FromRef` impl in `web_base.rs`.

pub use rust_template_foundation::auth::{
  callback_handler, current_user, login_handler, logout_handler, require_auth,
  AuthUser, CallbackQuery, OidcConfig,
};
