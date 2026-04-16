//! Derive macros for `rust-template-foundation`.
//!
//! Currently a placeholder — the `MergeConfig` derive macro will be added in
//! a follow-up phase.

use proc_macro::TokenStream;

/// Placeholder for the future `MergeConfig` derive macro.
#[proc_macro_derive(MergeConfig, attributes(merge_config))]
pub fn derive_merge_config(_input: TokenStream) -> TokenStream {
  TokenStream::new()
}
