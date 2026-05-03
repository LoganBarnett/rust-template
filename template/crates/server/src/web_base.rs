use rust_template_foundation::{
  impl_server_state, server::runner::BaseServerState,
};

#[derive(Clone)]
pub struct AppState {
  pub base: BaseServerState,
}

impl_server_state!(AppState, base);
