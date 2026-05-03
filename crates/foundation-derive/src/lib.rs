//! Derive macros for `rust-template-foundation`.
//!
//! Provides `#[foundation_main]` which generates the real `fn main()`
//! with CLI parsing, config resolution, logging init, and (for server
//! apps) tokio runtime + server construction.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, FnArg, ItemFn, Pat, PatType, Type};

/// Placeholder for the future `MergeConfig` derive macro.
#[proc_macro_derive(MergeConfig, attributes(merge_config))]
pub fn derive_merge_config(_input: TokenStream) -> TokenStream {
  TokenStream::new()
}

/// Entry-point macro for foundation-managed applications.
///
/// # Detection logic
///
/// - If any parameter's type path ends in `Server` → server app.
/// - Otherwise → CLI app.
/// - `async fn` → wrap in tokio runtime.
/// - `fn` → direct call.
///
/// # Server app
///
/// ```ignore
/// #[foundation_main]
/// pub async fn main(config: Config, server: Server) -> Result<ExitCode, E> { .. }
/// ```
///
/// Generates a real `fn main()` that:
/// 1. Parses CLI via `<Config as CliApp>::CliArgs::parse()`
/// 2. Resolves config via `<Config as CliApp>::from_cli(cli)`
/// 3. Inits server logging from config's log settings
/// 4. Builds tokio runtime
/// 5. Inside `block_on`: `BaseServerState::init`, creates `Server`,
///    calls user function
/// 6. Maps `Result<ExitCode, E>` to `ExitCode` (logs error on `Err`)
///
/// # CLI app
///
/// ```ignore
/// #[foundation_main]
/// pub fn main(config: Config) -> Result<ExitCode, E> { .. }
/// ```
///
/// Generates a real `fn main()` that:
/// 1. Parses CLI
/// 2. Resolves config
/// 3. Inits CLI logging
/// 4. Calls user function
/// 5. Maps result to `ExitCode`
#[proc_macro_attribute]
pub fn foundation_main(_attr: TokenStream, item: TokenStream) -> TokenStream {
  let input = parse_macro_input!(item as ItemFn);
  let user_fn_name = &input.sig.ident;
  let is_async = input.sig.asyncness.is_some();

  // Rename user function to avoid collision with generated main.
  let inner_fn_name = syn::Ident::new(
    &format!("__foundation_user_{}", user_fn_name),
    user_fn_name.span(),
  );

  // Extract parameter info.
  let params: Vec<_> = input
    .sig
    .inputs
    .iter()
    .filter_map(|arg| {
      if let FnArg::Typed(PatType { pat, ty, .. }) = arg {
        Some((pat.as_ref().clone(), ty.as_ref().clone()))
      } else {
        None
      }
    })
    .collect();

  // Determine if this is a server app by checking if any param type
  // path ends in "Server".
  let server_param = params.iter().find(|(_, ty)| type_is_server(ty));
  let is_server = server_param.is_some();

  // The config type is always the first parameter.
  let config_type = if let Some((_, ty)) = params.first() {
    ty.clone()
  } else {
    return syn::Error::new_spanned(
      &input.sig,
      "foundation_main requires at least one parameter (the config type)",
    )
    .to_compile_error()
    .into();
  };

  // Check for tuple server pattern: (primary, admin): (Server, Server)
  let server_tuple_len = server_param.as_ref().and_then(|(pat, ty)| {
    if let (Pat::Tuple(tuple_pat), Type::Tuple(tuple_ty)) = (pat, ty) {
      // Verify all elements are Server types.
      let all_server = tuple_ty.elems.iter().all(|e| type_is_server(e));
      if all_server && tuple_pat.elems.len() == tuple_ty.elems.len() {
        Some(tuple_ty.elems.len())
      } else {
        None
      }
    } else {
      None
    }
  });

  // Build the inner function (user's original, renamed).
  let mut inner_fn = input.clone();
  inner_fn.sig.ident = inner_fn_name.clone();
  // Remove the pub visibility — it's internal.
  inner_fn.vis = syn::Visibility::Inherited;

  let generated = if is_server && is_async {
    generate_server_main(
      &inner_fn,
      &inner_fn_name,
      &config_type,
      server_tuple_len,
    )
  } else if is_server {
    // Server apps must be async.
    return syn::Error::new_spanned(
      &input.sig,
      "Server apps must use async fn",
    )
    .to_compile_error()
    .into();
  } else if is_async {
    generate_async_cli_main(&inner_fn, &inner_fn_name, &config_type)
  } else {
    generate_cli_main(&inner_fn, &inner_fn_name, &config_type)
  };

  generated.into()
}

/// Check if a type path ends in "Server".
fn type_is_server(ty: &Type) -> bool {
  match ty {
    Type::Path(type_path) => type_path
      .path
      .segments
      .last()
      .map(|seg| seg.ident == "Server")
      .unwrap_or(false),
    Type::Tuple(tuple) => {
      // A tuple of Servers is also a server param.
      !tuple.elems.is_empty() && tuple.elems.iter().all(|e| type_is_server(e))
    }
    _ => false,
  }
}

/// Generate `fn main()` for an async server app.
fn generate_server_main(
  inner_fn: &ItemFn,
  inner_fn_name: &syn::Ident,
  config_type: &Type,
  tuple_len: Option<usize>,
) -> proc_macro2::TokenStream {
  let call_expr = if let Some(n) = tuple_len {
    // Tuple of N servers: create N servers and pass as tuple.
    let server_creates: Vec<_> = (0..n)
      .map(|i| {
        let var = syn::Ident::new(
          &format!("__server_{}", i),
          proc_macro2::Span::call_site(),
        );
        quote! {
          let #var = ::rust_template_foundation::Server::new(
            __base.clone(),
            __configs.remove(0),
          );
        }
      })
      .collect();

    let server_vars: Vec<_> = (0..n)
      .map(|i| {
        syn::Ident::new(
          &format!("__server_{}", i),
          proc_macro2::Span::call_site(),
        )
      })
      .collect();

    let n_lit =
      syn::LitInt::new(&n.to_string(), proc_macro2::Span::call_site());

    quote! {
      let mut __configs = <#config_type as ::rust_template_foundation::ServerApp>::server_run_configs(&__config);
      assert_eq!(
        __configs.len(),
        #n_lit,
        "server_run_configs() returned {} configs but the entry point expects {}",
        __configs.len(),
        #n_lit,
      );
      #(#server_creates)*
      #inner_fn_name(__config, (#(#server_vars),*)).await
    }
  } else {
    // Single server.
    quote! {
      let mut __configs = <#config_type as ::rust_template_foundation::ServerApp>::server_run_configs(&__config);
      assert_eq!(
        __configs.len(),
        1,
        "server_run_configs() returned {} configs but the entry point expects 1",
        __configs.len(),
      );
      let __server = ::rust_template_foundation::Server::new(
        __base.clone(),
        __configs.remove(0),
      );
      #inner_fn_name(__config, __server).await
    }
  };

  quote! {
    #inner_fn

    fn main() -> ::std::process::ExitCode {
      use ::clap::Parser as _;

      // 1. Parse CLI.
      let __cli = <#config_type as ::rust_template_foundation::CliApp>::CliArgs::parse();

      // 2. Resolve config.
      let __config = match <#config_type as ::rust_template_foundation::CliApp>::from_cli(__cli) {
        Ok(c) => c,
        Err(e) => {
          ::std::eprintln!("Configuration error: {}", e);
          return ::std::process::ExitCode::FAILURE;
        }
      };

      // 3. Init server logging.
      ::rust_template_foundation::logging::init_server_logging(
        <#config_type as ::rust_template_foundation::CliApp>::log_level(&__config),
        <#config_type as ::rust_template_foundation::CliApp>::log_format(&__config),
      );

      // 4. Build tokio runtime.
      let __rt = ::tokio::runtime::Runtime::new()
        .expect("failed to create tokio runtime");

      // 5. Run async block.
      let __result = __rt.block_on(async {
        // a. Init base server state from first config.
        let __first_config = <#config_type as ::rust_template_foundation::ServerApp>::server_run_configs(&__config);
        let __base = match ::rust_template_foundation::BaseServerState::init(
          &__first_config[0],
        ).await {
          Ok(b) => b,
          Err(e) => {
            ::tracing::error!("Failed to initialize server state: {}", e);
            return ::std::process::ExitCode::FAILURE;
          }
        };

        // b. Create server(s) and call user function.
        match { #call_expr } {
          Ok(code) => code,
          Err(e) => {
            ::tracing::error!("Application error: {}", e);
            ::std::process::ExitCode::FAILURE
          }
        }
      });

      __result
    }
  }
}

/// Generate `fn main()` for a sync CLI app.
fn generate_cli_main(
  inner_fn: &ItemFn,
  inner_fn_name: &syn::Ident,
  config_type: &Type,
) -> proc_macro2::TokenStream {
  quote! {
    #inner_fn

    fn main() -> ::std::process::ExitCode {
      use ::clap::Parser as _;

      let __cli = <#config_type as ::rust_template_foundation::CliApp>::CliArgs::parse();

      let __config = match <#config_type as ::rust_template_foundation::CliApp>::from_cli(__cli) {
        Ok(c) => c,
        Err(e) => {
          ::std::eprintln!("Configuration error: {}", e);
          return ::std::process::ExitCode::FAILURE;
        }
      };

      ::rust_template_foundation::logging::init_cli_logging(
        <#config_type as ::rust_template_foundation::CliApp>::log_level(&__config),
        <#config_type as ::rust_template_foundation::CliApp>::log_format(&__config),
      );

      match #inner_fn_name(__config) {
        Ok(code) => code,
        Err(e) => {
          ::tracing::error!("Application error: {}", e);
          ::std::process::ExitCode::FAILURE
        }
      }
    }
  }
}

/// Generate `fn main()` for an async CLI app.
fn generate_async_cli_main(
  inner_fn: &ItemFn,
  inner_fn_name: &syn::Ident,
  config_type: &Type,
) -> proc_macro2::TokenStream {
  quote! {
    #inner_fn

    fn main() -> ::std::process::ExitCode {
      use ::clap::Parser as _;

      let __cli = <#config_type as ::rust_template_foundation::CliApp>::CliArgs::parse();

      let __config = match <#config_type as ::rust_template_foundation::CliApp>::from_cli(__cli) {
        Ok(c) => c,
        Err(e) => {
          ::std::eprintln!("Configuration error: {}", e);
          return ::std::process::ExitCode::FAILURE;
        }
      };

      ::rust_template_foundation::logging::init_cli_logging(
        <#config_type as ::rust_template_foundation::CliApp>::log_level(&__config),
        <#config_type as ::rust_template_foundation::CliApp>::log_format(&__config),
      );

      let __rt = ::tokio::runtime::Runtime::new()
        .expect("failed to create tokio runtime");

      let __result = __rt.block_on(async {
        match #inner_fn_name(__config).await {
          Ok(code) => code,
          Err(e) => {
            ::tracing::error!("Application error: {}", e);
            ::std::process::ExitCode::FAILURE
          }
        }
      });

      __result
    }
  }
}
