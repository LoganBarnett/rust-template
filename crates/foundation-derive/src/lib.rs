//! Derive macros for `rust-template-foundation`.
//!
//! Provides `#[foundation_main]` which generates the real `fn main()`
//! with CLI parsing, config resolution, logging init, and (for server
//! apps) tokio runtime + server construction.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
  parse_macro_input, Data, DataStruct, DeriveInput, Expr, Fields, FieldsNamed,
  FnArg, Ident, ItemFn, LitChar, LitStr, Pat, PatType, Type,
};

/// Derive macro that generates config boilerplate from a single
/// annotated struct.
///
/// Produces `CliRaw` (clap `Parser`), `ConfigFileRaw` (serde
/// `Deserialize`), `ConfigError`, `from_cli_and_file`, and `CliApp`
/// trait implementation.
///
/// See `crates/foundation/USAGE.org` for full documentation.
#[proc_macro_derive(MergeConfig, attributes(merge_config))]
pub fn derive_merge_config(input: TokenStream) -> TokenStream {
  let input = parse_macro_input!(input as DeriveInput);
  match mc_derive_impl(input) {
    Ok(ts) => ts.into(),
    Err(e) => e.to_compile_error().into(),
  }
}

// ── MergeConfig internals ──────────────────────────────────────────────

struct McStructAttrs {
  app_name: LitStr,
  extra_cli: Option<syn::Path>,
  extra_file: Option<syn::Path>,
}

enum McShortFlag {
  None,
  Auto,
  Explicit(LitChar),
}

enum McFieldKind {
  Common,
  Merged {
    raw_name: Ident,
    env: Option<LitStr>,
    short: McShortFlag,
    default: Option<Expr>,
    required: bool,
    parse: bool,
    cli_only: bool,
  },
  Skip,
}

struct McFieldInfo {
  ident: Ident,
  ty: Type,
  kind: McFieldKind,
  doc_attrs: Vec<syn::Attribute>,
}

fn mc_parse_struct_attrs(
  attrs: &[syn::Attribute],
) -> syn::Result<McStructAttrs> {
  let mut app_name: Option<LitStr> = None;
  let mut extra_cli: Option<syn::Path> = None;
  let mut extra_file: Option<syn::Path> = None;

  for attr in attrs {
    if !attr.path().is_ident("merge_config") {
      continue;
    }
    attr.parse_nested_meta(|meta| {
      if meta.path.is_ident("app_name") {
        app_name = Some(meta.value()?.parse()?);
      } else if meta.path.is_ident("extra_cli") {
        let s: LitStr = meta.value()?.parse()?;
        extra_cli = Some(syn::parse_str(&s.value())?);
      } else if meta.path.is_ident("extra_file") {
        let s: LitStr = meta.value()?.parse()?;
        extra_file = Some(syn::parse_str(&s.value())?);
      } else {
        return Err(meta.error("unknown merge_config attribute"));
      }
      Ok(())
    })?;
  }

  let app_name = app_name.ok_or_else(|| {
    syn::Error::new(
      proc_macro2::Span::call_site(),
      "merge_config requires `app_name`",
    )
  })?;

  Ok(McStructAttrs {
    app_name,
    extra_cli,
    extra_file,
  })
}

fn mc_parse_field(field: &syn::Field) -> syn::Result<McFieldInfo> {
  let ident = field.ident.clone().ok_or_else(|| {
    syn::Error::new_spanned(field, "unnamed fields not supported")
  })?;
  let ty = field.ty.clone();

  let doc_attrs: Vec<_> = field
    .attrs
    .iter()
    .filter(|a| a.path().is_ident("doc"))
    .cloned()
    .collect();

  let has_mc = field
    .attrs
    .iter()
    .any(|a| a.path().is_ident("merge_config"));
  if !has_mc {
    return Err(syn::Error::new_spanned(
      &ident,
      "every field must have a #[merge_config(...)] attribute",
    ));
  }

  let mut is_common = false;
  let mut is_skip = false;
  let mut name: Option<LitStr> = None;
  let mut env: Option<LitStr> = None;
  let mut short = McShortFlag::None;
  let mut default: Option<Expr> = None;
  let mut required = false;
  let mut parse = false;
  let mut cli_only = false;

  for attr in &field.attrs {
    if !attr.path().is_ident("merge_config") {
      continue;
    }
    attr.parse_nested_meta(|meta| {
      if meta.path.is_ident("common") {
        is_common = true;
      } else if meta.path.is_ident("skip") {
        is_skip = true;
      } else if meta.path.is_ident("name") {
        name = Some(meta.value()?.parse()?);
      } else if meta.path.is_ident("env") {
        env = Some(meta.value()?.parse()?);
      } else if meta.path.is_ident("short") {
        if meta.input.peek(syn::Token![=]) {
          short = McShortFlag::Explicit(meta.value()?.parse()?);
        } else {
          short = McShortFlag::Auto;
        }
      } else if meta.path.is_ident("default") {
        let s: LitStr = meta.value()?.parse()?;
        default = Some(syn::parse_str(&s.value())?);
      } else if meta.path.is_ident("required") {
        required = true;
      } else if meta.path.is_ident("parse") {
        parse = true;
      } else if meta.path.is_ident("cli_only") {
        cli_only = true;
      } else {
        return Err(meta.error("unknown merge_config field attribute"));
      }
      Ok(())
    })?;
  }

  let kind = if is_common {
    McFieldKind::Common
  } else if is_skip {
    McFieldKind::Skip
  } else {
    if default.is_none() && !required {
      return Err(syn::Error::new_spanned(
        &ident,
        "merged fields require `default` or `required`",
      ));
    }
    if default.is_some() && required {
      return Err(syn::Error::new_spanned(
        &ident,
        "`default` and `required` are mutually exclusive",
      ));
    }

    let raw_name = name
      .as_ref()
      .map(|n| Ident::new(&n.value(), n.span()))
      .unwrap_or_else(|| ident.clone());

    McFieldKind::Merged {
      raw_name,
      env,
      short,
      default,
      required,
      parse,
      cli_only,
    }
  };

  Ok(McFieldInfo {
    ident,
    ty,
    kind,
    doc_attrs,
  })
}

fn mc_gen_cli_raw(
  fields: &[McFieldInfo],
  attrs: &McStructAttrs,
) -> proc_macro2::TokenStream {
  let app_name = &attrs.app_name;

  let field_defs: Vec<_> = fields
    .iter()
    .filter_map(|f| {
      let McFieldKind::Merged {
        raw_name,
        env,
        short,
        parse,
        ..
      } = &f.kind
      else {
        return None;
      };

      let docs = &f.doc_attrs;

      let mut arg_parts = Vec::new();
      match short {
        McShortFlag::Auto => arg_parts.push(quote! { short }),
        McShortFlag::Explicit(c) => arg_parts.push(quote! { short = #c }),
        McShortFlag::None => {}
      }
      arg_parts.push(quote! { long });
      if let Some(env_val) = env {
        arg_parts.push(quote! { env = #env_val });
      }

      let field_ty = if *parse {
        quote! { ::std::option::Option<::std::string::String> }
      } else {
        let ty = &f.ty;
        quote! { ::std::option::Option<#ty> }
      };

      Some(quote! {
        #(#docs)*
        #[arg(#(#arg_parts),*)]
        pub #raw_name: #field_ty,
      })
    })
    .collect();

  let extra_field = attrs.extra_cli.as_ref().map(|extra_ty| {
    quote! {
      #[command(flatten)]
      pub extra: #extra_ty,
    }
  });

  quote! {
    #[derive(::std::fmt::Debug, ::clap::Parser)]
    #[command(name = #app_name, version, about)]
    pub struct CliRaw {
      #[command(flatten)]
      pub common: ::rust_template_foundation::config::CommonCli,
      #(#field_defs)*
      #extra_field
    }
  }
}

fn mc_gen_config_file_raw(
  fields: &[McFieldInfo],
  attrs: &McStructAttrs,
) -> proc_macro2::TokenStream {
  let field_defs: Vec<_> = fields
    .iter()
    .filter_map(|f| {
      let McFieldKind::Merged {
        raw_name,
        parse,
        cli_only,
        ..
      } = &f.kind
      else {
        return None;
      };
      if *cli_only {
        return None;
      }

      let field_ty = if *parse {
        quote! { ::std::option::Option<::std::string::String> }
      } else {
        let ty = &f.ty;
        quote! { ::std::option::Option<#ty> }
      };

      Some(quote! {
        pub #raw_name: #field_ty,
      })
    })
    .collect();

  let extra_field = attrs.extra_file.as_ref().map(|extra_ty| {
    quote! {
      #[serde(flatten)]
      pub extra: #extra_ty,
    }
  });

  quote! {
    #[derive(::std::fmt::Debug, ::serde::Deserialize, Default)]
    pub struct ConfigFileRaw {
      #[serde(flatten)]
      pub common:
        ::rust_template_foundation::config::CommonConfigFile,
      #(#field_defs)*
      #extra_field
    }
  }
}

fn mc_gen_config_error() -> proc_macro2::TokenStream {
  quote! {
    #[derive(::std::fmt::Debug)]
    pub enum ConfigError {
      File(::rust_template_foundation::config::ConfigFileError),
      Validation(::std::string::String),
    }

    impl ::std::fmt::Display for ConfigError {
      fn fmt(
        &self,
        f: &mut ::std::fmt::Formatter<'_>,
      ) -> ::std::fmt::Result {
        match self {
          ConfigError::File(e) => {
            write!(
              f,
              "Failed to load configuration file: {}",
              e
            )
          }
          ConfigError::Validation(msg) => {
            write!(
              f,
              "Configuration validation failed: {}",
              msg
            )
          }
        }
      }
    }

    impl ::std::error::Error for ConfigError {
      fn source(
        &self,
      ) -> ::std::option::Option<
        &(dyn ::std::error::Error + 'static),
      > {
        match self {
          ConfigError::File(e) => {
            ::std::option::Option::Some(e)
          }
          ConfigError::Validation(_) => {
            ::std::option::Option::None
          }
        }
      }
    }

    impl
      ::std::convert::From<
        ::rust_template_foundation::config::ConfigFileError,
      > for ConfigError
    {
      fn from(
        e: ::rust_template_foundation::config::ConfigFileError,
      ) -> Self {
        ConfigError::File(e)
      }
    }
  }
}

fn mc_gen_from_cli_and_file(
  struct_name: &Ident,
  fields: &[McFieldInfo],
  attrs: &McStructAttrs,
) -> proc_macro2::TokenStream {
  let app_name = &attrs.app_name;

  // Find common field idents by name.
  let log_level_ident = fields
    .iter()
    .find(|f| matches!(f.kind, McFieldKind::Common) && f.ident == "log_level")
    .map(|f| &f.ident);
  let log_format_ident = fields
    .iter()
    .find(|f| matches!(f.kind, McFieldKind::Common) && f.ident == "log_format")
    .map(|f| &f.ident);

  let log_resolve = match (log_level_ident, log_format_ident) {
    (Some(lvl), Some(fmt)) => quote! {
      let (#lvl, #fmt) =
        ::rust_template_foundation::config::resolve_log_settings(
          cli.common.log_level.clone(),
          cli.common.log_format.clone(),
          &file.common,
        )
        .map_err(ConfigError::Validation)?;
    },
    _ => quote! {},
  };

  // Skip field resolution (borrows cli/file, must come before
  // merged fields which move from them).
  let skip_stmts: Vec<_> = fields
    .iter()
    .filter_map(|f| {
      if !matches!(f.kind, McFieldKind::Skip) {
        return None;
      }
      let field_name = &f.ident;
      let resolve_fn =
        Ident::new(&format!("resolve_{}", field_name), field_name.span());
      Some(quote! {
        let #field_name = Self::#resolve_fn(&cli, &file)?;
      })
    })
    .collect();

  // Merged field resolution (moves from cli/file).
  let merge_stmts: Vec<_> = fields
    .iter()
    .filter_map(|f| {
      let McFieldKind::Merged {
        raw_name,
        default,
        required,
        parse,
        cli_only,
        ..
      } = &f.kind
      else {
        return None;
      };

      let field_name = &f.ident;
      let field_ty = &f.ty;

      let or_file = if *cli_only {
        quote! {}
      } else {
        quote! { .or(file.#raw_name) }
      };

      let unwrap = if let Some(default_expr) = default {
        quote! { .unwrap_or_else(|| #default_expr) }
      } else if *required {
        quote! {
          .ok_or_else(|| ConfigError::Validation(
            ::std::format!(
              "{} is required",
              ::std::stringify!(#field_name),
            )
          ))?
        }
      } else {
        quote! {}
      };

      if *parse {
        let raw_var =
          Ident::new(&format!("__raw_{}", field_name), field_name.span());
        Some(quote! {
          let #raw_var =
            cli.#raw_name #or_file #unwrap;
          let #field_name =
            #raw_var.parse::<#field_ty>().map_err(|e| {
              ConfigError::Validation(::std::format!(
                "invalid {}: '{}': {}",
                ::std::stringify!(#field_name),
                #raw_var,
                e,
              ))
            })?;
        })
      } else {
        Some(quote! {
          let #field_name =
            cli.#raw_name #or_file #unwrap;
        })
      }
    })
    .collect();

  let field_names: Vec<_> = fields.iter().map(|f| &f.ident).collect();

  quote! {
    impl #struct_name {
      /// Resolve configuration from parsed CLI arguments.
      ///
      /// Loads the config file (if found), merges CLI and file
      /// values with appropriate defaults, and validates the
      /// result.
      pub fn from_cli_and_file(
        cli: CliRaw,
      ) -> ::std::result::Result<Self, ConfigError> {
        let file: ConfigFileRaw =
          match ::rust_template_foundation::config::find_config_file(
            #app_name,
            cli.common.config.as_deref(),
          ) {
            ::std::option::Option::Some(path) => {
              ::rust_template_foundation::config::load_toml(
                &path,
              )?
            }
            ::std::option::Option::None => {
              ConfigFileRaw::default()
            }
          };

        #log_resolve
        #(#skip_stmts)*
        #(#merge_stmts)*

        ::std::result::Result::Ok(#struct_name {
          #(#field_names),*
        })
      }
    }
  }
}

fn mc_gen_cli_app_impl(
  struct_name: &Ident,
  fields: &[McFieldInfo],
  attrs: &McStructAttrs,
) -> proc_macro2::TokenStream {
  let app_name = &attrs.app_name;

  let log_level_ident = fields
    .iter()
    .find(|f| matches!(f.kind, McFieldKind::Common) && f.ident == "log_level")
    .map(|f| &f.ident);
  let log_format_ident = fields
    .iter()
    .find(|f| matches!(f.kind, McFieldKind::Common) && f.ident == "log_format")
    .map(|f| &f.ident);

  let log_level_fn = log_level_ident.map(|id| {
    quote! {
      fn log_level(
        &self,
      ) -> ::rust_template_foundation::logging::LogLevel {
        self.#id
      }
    }
  });

  let log_format_fn = log_format_ident.map(|id| {
    quote! {
      fn log_format(
        &self,
      ) -> ::rust_template_foundation::logging::LogFormat {
        self.#id
      }
    }
  });

  quote! {
    impl ::rust_template_foundation::CliApp for #struct_name {
      type CliArgs = CliRaw;
      type Error = ConfigError;

      fn app_name() -> &'static str {
        #app_name
      }

      fn from_cli(
        cli: CliRaw,
      ) -> ::std::result::Result<Self, ConfigError> {
        Self::from_cli_and_file(cli)
      }

      #log_level_fn
      #log_format_fn
    }
  }
}

fn mc_derive_impl(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
  let struct_name = &input.ident;

  let attrs = mc_parse_struct_attrs(&input.attrs)?;

  let named = match &input.data {
    Data::Struct(DataStruct {
      fields: Fields::Named(FieldsNamed { named, .. }),
      ..
    }) => named,
    _ => {
      return Err(syn::Error::new_spanned(
        &input,
        "MergeConfig requires a struct with named fields",
      ))
    }
  };

  let field_infos: Vec<McFieldInfo> = named
    .iter()
    .map(mc_parse_field)
    .collect::<syn::Result<_>>()?;

  // Validate common fields.
  let common_count = field_infos
    .iter()
    .filter(|f| matches!(f.kind, McFieldKind::Common))
    .count();
  if common_count != 2 {
    return Err(syn::Error::new_spanned(
      &input,
      "MergeConfig requires exactly two `common` fields \
       (log_level and log_format)",
    ));
  }
  let has_log_level = field_infos
    .iter()
    .any(|f| matches!(f.kind, McFieldKind::Common) && f.ident == "log_level");
  let has_log_format = field_infos
    .iter()
    .any(|f| matches!(f.kind, McFieldKind::Common) && f.ident == "log_format");
  if !has_log_level || !has_log_format {
    return Err(syn::Error::new_spanned(
      &input,
      "common fields must be named `log_level` and \
       `log_format`",
    ));
  }

  let cli_raw = mc_gen_cli_raw(&field_infos, &attrs);
  let config_file_raw = mc_gen_config_file_raw(&field_infos, &attrs);
  let config_error = mc_gen_config_error();
  let from_cli = mc_gen_from_cli_and_file(struct_name, &field_infos, &attrs);
  let cli_app = mc_gen_cli_app_impl(struct_name, &field_infos, &attrs);

  Ok(quote! {
    #cli_raw
    #config_file_raw
    #config_error
    #from_cli
    #cli_app
  })
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
