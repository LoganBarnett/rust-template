//! Derive macros for `rust-template-foundation`.
//!
//! Provides `#[foundation_main]` which generates the real `fn main()`
//! with CLI parsing, config resolution, logging init, and (for server
//! apps) tokio runtime + server construction.

use proc_macro::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{
  parse_macro_input, Data, DataStruct, DeriveInput, Expr, ExprLit, Fields,
  FieldsNamed, FnArg, Ident, ItemFn, Lit, LitChar, LitStr, Meta, MetaNameValue,
  Pat, PatType, Token, Type,
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

struct MergeConfigStructAttrs {
  app_name: LitStr,
  extra_cli: Option<syn::Path>,
  extra_file: Option<syn::Path>,
  extra_error: Option<syn::Path>,
}

/// The struct attributes as written, before `app_name` is known to be there.
#[derive(Default)]
struct StructSpec {
  app_name: Option<LitStr>,
  extra_cli: Option<syn::Path>,
  extra_file: Option<syn::Path>,
  extra_error: Option<syn::Path>,
}

/// One item of the struct-level `merge_config` attribute.
enum StructAttr {
  AppName(LitStr),
  ExtraCli(syn::Path),
  ExtraFile(syn::Path),
  ExtraError(syn::Path),
}

#[derive(Default)]
enum MergeConfigArgShortFlag {
  #[default]
  None,
  Auto,
  Explicit(LitChar),
}

impl MergeConfigArgShortFlag {
  /// The clap attribute part for the short flag, if the field has one.
  fn arg_part(&self) -> Option<proc_macro2::TokenStream> {
    match self {
      Self::None => None,
      Self::Auto => Some(quote! { short }),
      Self::Explicit(c) => Some(quote! { short = #c }),
    }
  }
}

/// Environment-variable binding for a merged field.
///
/// `Auto` is what every merged field gets with no attribute: the name is
/// derived as `<env_prefix>_<raw_name>` (lowercase per POSIX §8.1's
/// application namespace).  The other two are deviations that draw
/// attention, so a comment beside the attribute should explain why:
/// `Literal` reads a name imposed from outside (`env = "..."`), and
/// `Disabled` reads nothing (`no_env`).
enum MergeConfigArgEnv {
  Auto,
  Literal(LitStr),
  Disabled,
}

impl MergeConfigArgEnv {
  /// The clap attribute part binding the env var, if the field reads one.
  fn arg_part(
    &self,
    prefix: &str,
    raw_name: &Ident,
  ) -> Option<proc_macro2::TokenStream> {
    match self {
      Self::Disabled => None,
      Self::Auto => {
        let derived = ::std::format!("{prefix}_{raw_name}");
        Some(quote! { env = #derived })
      }
      Self::Literal(name) => Some(quote! { env = #name }),
    }
  }
}

struct MergeConfigMergedArg {
  raw_name: Ident,
  env: MergeConfigArgEnv,
  short: MergeConfigArgShortFlag,
  default: Option<Expr>,
  required: bool,
  parse: bool,
  cli_only: bool,
}

enum MergeConfigFieldKind {
  Common,
  // Boxed because the inner struct dwarfs the unit variants — keeping
  // it inline would push every `MergeConfigFieldKind` to ~232 bytes.
  Merged(Box<MergeConfigMergedArg>),
  Skip,
  // Pure passthrough of a clap `#[derive(Subcommand)]` enum.  Forwarded
  // to `CliRaw` with `#[command(subcommand)]` and copied verbatim into
  // the resolved `Config`.  Never appears in `ConfigFileRaw` — TOML has
  // no clean tagged-enum mapping for clap subcommands, so subcommands
  // are CLI-only by construction.
  Subcommand,
}

struct MergeConfigFieldInfo {
  ident: Ident,
  ty: Type,
  kind: MergeConfigFieldKind,
  doc_attrs: Vec<syn::Attribute>,
}

/// One item of a field's `merge_config` attribute.
enum FieldAttr {
  Common,
  Skip,
  Subcommand,
  Name(LitStr),
  EnvLiteral(LitStr),
  NoEnv,
  Short(MergeConfigArgShortFlag),
  Default(Expr),
  Required,
  Parse,
  CliOnly,
}

/// A field's attributes gathered by meaning, before the field's kind is
/// known.  Where an item can be written twice, the last spelling wins.
#[derive(Default)]
struct FieldSpec {
  common: bool,
  skip: bool,
  subcommand: bool,
  name: Option<LitStr>,
  env_literal: Option<LitStr>,
  no_env: bool,
  short: MergeConfigArgShortFlag,
  default: Option<Expr>,
  required: bool,
  parse: bool,
  cli_only: bool,
}

/// Every item written across the `merge_config` attributes, in order.
fn merge_config_items(attrs: &[syn::Attribute]) -> syn::Result<Vec<Meta>> {
  attrs
    .iter()
    .filter(|attr| attr.path().is_ident("merge_config"))
    .map(|attr| {
      attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
    })
    .collect::<syn::Result<Vec<_>>>()
    .map(|lists| lists.into_iter().flatten().collect())
}

/// The name of an item, when its path is a single identifier, which is the
/// only shape any item accepts.
fn item_name(meta: &Meta) -> Option<String> {
  meta.path().get_ident().map(ToString::to_string)
}

/// The string literal an `= "..."` item carries.
fn string_value(pair: &MetaNameValue) -> syn::Result<LitStr> {
  match &pair.value {
    Expr::Lit(ExprLit {
      lit: Lit::Str(text),
      ..
    }) => Ok(text.clone()),
    other => Err(syn::Error::new_spanned(other, "expected a string literal")),
  }
}

/// The char literal an `= 'x'` item carries.
fn char_value(pair: &MetaNameValue) -> syn::Result<LitChar> {
  match &pair.value {
    Expr::Lit(ExprLit {
      lit: Lit::Char(c), ..
    }) => Ok(c.clone()),
    other => Err(syn::Error::new_spanned(other, "expected a char literal")),
  }
}

/// A path written as a string literal, as `extra_cli = "Type"` spells it.
fn path_value(pair: &MetaNameValue) -> syn::Result<syn::Path> {
  string_value(pair).and_then(|text| syn::parse_str(&text.value()))
}

fn struct_attr(meta: Meta) -> syn::Result<StructAttr> {
  match (item_name(&meta).as_deref(), &meta) {
    (Some("app_name"), Meta::NameValue(pair)) => {
      string_value(pair).map(StructAttr::AppName)
    }
    (Some("extra_cli"), Meta::NameValue(pair)) => {
      path_value(pair).map(StructAttr::ExtraCli)
    }
    (Some("extra_file"), Meta::NameValue(pair)) => {
      path_value(pair).map(StructAttr::ExtraFile)
    }
    (Some("extra_error"), Meta::NameValue(pair)) => {
      path_value(pair).map(StructAttr::ExtraError)
    }
    _ => Err(syn::Error::new_spanned(&meta, "unknown merge_config attribute")),
  }
}

fn mc_parse_struct_attrs(
  attrs: &[syn::Attribute],
) -> syn::Result<MergeConfigStructAttrs> {
  let spec = merge_config_items(attrs)?
    .into_iter()
    .map(struct_attr)
    .collect::<syn::Result<Vec<_>>>()?
    .into_iter()
    .fold(StructSpec::default(), |so_far, attr| match attr {
      StructAttr::AppName(name) => StructSpec {
        app_name: Some(name),
        ..so_far
      },
      StructAttr::ExtraCli(path) => StructSpec {
        extra_cli: Some(path),
        ..so_far
      },
      StructAttr::ExtraFile(path) => StructSpec {
        extra_file: Some(path),
        ..so_far
      },
      StructAttr::ExtraError(path) => StructSpec {
        extra_error: Some(path),
        ..so_far
      },
    });
  Ok(MergeConfigStructAttrs {
    app_name: spec.app_name.ok_or_else(|| {
      syn::Error::new(
        proc_macro2::Span::call_site(),
        "merge_config requires `app_name`",
      )
    })?,
    extra_cli: spec.extra_cli,
    extra_file: spec.extra_file,
    extra_error: spec.extra_error,
  })
}

fn field_attr(meta: Meta) -> syn::Result<FieldAttr> {
  match (item_name(&meta).as_deref(), &meta) {
    (Some("common"), Meta::Path(_)) => Ok(FieldAttr::Common),
    (Some("skip"), Meta::Path(_)) => Ok(FieldAttr::Skip),
    (Some("subcommand"), Meta::Path(_)) => Ok(FieldAttr::Subcommand),
    (Some("name"), Meta::NameValue(pair)) => {
      string_value(pair).map(FieldAttr::Name)
    }
    (Some("env"), Meta::NameValue(pair)) => {
      string_value(pair).map(FieldAttr::EnvLiteral)
    }
    // A bare `env` would be a no-op that reads as meaningful, so it is
    // refused with the fix named rather than silently accepted.
    (Some("env"), Meta::Path(_)) => Err(syn::Error::new_spanned(
      &meta,
      "`env` without a value is redundant: every merged field reads \
       `<app>_<flag>` by default.  Remove it, or write `no_env` to opt the \
       field out.",
    )),
    (Some("no_env"), Meta::Path(_)) => Ok(FieldAttr::NoEnv),
    (Some("short"), Meta::Path(_)) => {
      Ok(FieldAttr::Short(MergeConfigArgShortFlag::Auto))
    }
    (Some("short"), Meta::NameValue(pair)) => char_value(pair)
      .map(|c| FieldAttr::Short(MergeConfigArgShortFlag::Explicit(c))),
    (Some("default"), Meta::NameValue(pair)) => string_value(pair)
      .and_then(|text| syn::parse_str(&text.value()))
      .map(FieldAttr::Default),
    (Some("required"), Meta::Path(_)) => Ok(FieldAttr::Required),
    (Some("parse"), Meta::Path(_)) => Ok(FieldAttr::Parse),
    (Some("cli_only"), Meta::Path(_)) => Ok(FieldAttr::CliOnly),
    _ => Err(syn::Error::new_spanned(
      &meta,
      "unknown merge_config field attribute",
    )),
  }
}

/// The field's attributes gathered by meaning.
fn field_spec(items: Vec<Meta>) -> syn::Result<FieldSpec> {
  items
    .into_iter()
    .map(field_attr)
    .collect::<syn::Result<Vec<_>>>()
    .map(|attrs| {
      attrs
        .into_iter()
        .fold(FieldSpec::default(), |so_far, attr| match attr {
          FieldAttr::Common => FieldSpec {
            common: true,
            ..so_far
          },
          FieldAttr::Skip => FieldSpec {
            skip: true,
            ..so_far
          },
          FieldAttr::Subcommand => FieldSpec {
            subcommand: true,
            ..so_far
          },
          FieldAttr::Name(name) => FieldSpec {
            name: Some(name),
            ..so_far
          },
          FieldAttr::EnvLiteral(name) => FieldSpec {
            env_literal: Some(name),
            ..so_far
          },
          FieldAttr::NoEnv => FieldSpec {
            no_env: true,
            ..so_far
          },
          FieldAttr::Short(short) => FieldSpec { short, ..so_far },
          FieldAttr::Default(default) => FieldSpec {
            default: Some(default),
            ..so_far
          },
          FieldAttr::Required => FieldSpec {
            required: true,
            ..so_far
          },
          FieldAttr::Parse => FieldSpec {
            parse: true,
            ..so_far
          },
          FieldAttr::CliOnly => FieldSpec {
            cli_only: true,
            ..so_far
          },
        })
    })
}

fn mc_parse_field(field: &syn::Field) -> syn::Result<MergeConfigFieldInfo> {
  let ident = field.ident.clone().ok_or_else(|| {
    syn::Error::new_spanned(field, "unnamed fields not supported")
  })?;
  if !field
    .attrs
    .iter()
    .any(|attr| attr.path().is_ident("merge_config"))
  {
    Err(syn::Error::new_spanned(
      &ident,
      "every field must have a #[merge_config(...)] attribute",
    ))
  } else {
    Ok(MergeConfigFieldInfo {
      ty: field.ty.clone(),
      kind: field_kind(&ident, field_spec(merge_config_items(&field.attrs)?)?)?,
      doc_attrs: field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .cloned()
        .collect(),
      ident,
    })
  }
}

/// The kind a field's attributes name, or why they name none.
fn field_kind(
  ident: &Ident,
  spec: FieldSpec,
) -> syn::Result<MergeConfigFieldKind> {
  let exclusive = [spec.common, spec.skip, spec.subcommand]
    .into_iter()
    .filter(|set| *set)
    .count();
  if exclusive > 1 {
    Err(syn::Error::new_spanned(
      ident,
      "`common`, `skip`, and `subcommand` are mutually exclusive",
    ))
  } else if spec.common {
    Ok(MergeConfigFieldKind::Common)
  } else if spec.skip {
    Ok(MergeConfigFieldKind::Skip)
  } else if spec.subcommand {
    subcommand_kind(ident, &spec)
  } else {
    merged_kind(ident, spec)
  }
}

/// A subcommand field is pure clap passthrough, so every merge-semantics
/// attribute on it is refused: none has a meaning there, and forbidding them
/// keeps the surface honest.
fn subcommand_kind(
  ident: &Ident,
  spec: &FieldSpec,
) -> syn::Result<MergeConfigFieldKind> {
  [
    ("name", spec.name.is_some()),
    ("env", spec.env_literal.is_some()),
    ("no_env", spec.no_env),
    ("short", !matches!(spec.short, MergeConfigArgShortFlag::None)),
    ("default", spec.default.is_some()),
    ("required", spec.required),
    ("parse", spec.parse),
    ("cli_only", spec.cli_only),
  ]
  .into_iter()
  .find(|(_, present)| *present)
  .map_or(Ok(MergeConfigFieldKind::Subcommand), |(attr, _)| {
    Err(syn::Error::new_spanned(
      ident,
      ::std::format!("`{attr}` is not allowed on a `subcommand` field"),
    ))
  })
}

fn merged_kind(
  ident: &Ident,
  spec: FieldSpec,
) -> syn::Result<MergeConfigFieldKind> {
  if spec.default.is_none() && !spec.required {
    Err(syn::Error::new_spanned(
      ident,
      "merged fields require `default` or `required`",
    ))
  } else if spec.default.is_some() && spec.required {
    Err(syn::Error::new_spanned(
      ident,
      "`default` and `required` are mutually exclusive",
    ))
  } else if spec.no_env && spec.env_literal.is_some() {
    Err(syn::Error::new_spanned(
      ident,
      "`no_env` and `env = \"...\"` are mutually exclusive",
    ))
  } else {
    Ok(MergeConfigFieldKind::Merged(Box::new(MergeConfigMergedArg {
      raw_name: spec
        .name
        .as_ref()
        .map_or_else(|| ident.clone(), |n| Ident::new(&n.value(), n.span())),
      env: if spec.no_env {
        MergeConfigArgEnv::Disabled
      } else {
        spec
          .env_literal
          .map_or(MergeConfigArgEnv::Auto, MergeConfigArgEnv::Literal)
      },
      short: spec.short,
      default: spec.default,
      required: spec.required,
      parse: spec.parse,
      cli_only: spec.cli_only,
    })))
  }
}

/// Convert the `app_name` literal into the env-var prefix.
///
/// `example-server` → `example_server`.  Lowercase per POSIX §8.1: the
/// namespace of env-var names containing lowercase letters is reserved
/// for applications.  Hyphens become underscores because POSIX env-var
/// names are restricted to letters, digits, and underscore.
fn env_prefix(app_name: &LitStr) -> String {
  app_name.value().to_lowercase().replace('-', "_")
}

/// One merged field of `CliRaw`.
fn cli_field_def(
  field: &MergeConfigFieldInfo,
  merged: &MergeConfigMergedArg,
  prefix: &str,
) -> proc_macro2::TokenStream {
  let docs = &field.doc_attrs;
  let raw_name = &merged.raw_name;
  let arg_parts: Vec<proc_macro2::TokenStream> = merged
    .short
    .arg_part()
    .into_iter()
    .chain(std::iter::once(quote! { long }))
    .chain(merged.env.arg_part(prefix, raw_name))
    .collect();
  let ty = &field.ty;
  let field_ty = if merged.parse {
    quote! { Option<String> }
  } else {
    quote! { Option<#ty> }
  };
  quote! {
    #(#docs)*
    #[arg(#(#arg_parts),*)]
    pub #raw_name: #field_ty,
  }
}

fn mc_gen_cli_raw(
  fields: &[MergeConfigFieldInfo],
  attrs: &MergeConfigStructAttrs,
) -> proc_macro2::TokenStream {
  let app_name = &attrs.app_name;
  let prefix = env_prefix(app_name);

  let field_defs: Vec<_> = fields
    .iter()
    .filter_map(|f| match &f.kind {
      MergeConfigFieldKind::Merged(m) => Some(cli_field_def(f, m, &prefix)),
      _ => None,
    })
    .collect();

  // Subcommand field is forwarded verbatim.  Required/optional follows
  // the user's field type: `Commands` requires a subcommand, while
  // `Option<Commands>` makes it optional.
  let subcommand_field = fields
    .iter()
    .find(|f| matches!(f.kind, MergeConfigFieldKind::Subcommand))
    .map(|f| {
      let ident = &f.ident;
      let ty = &f.ty;
      let docs = &f.doc_attrs;
      quote! {
        #(#docs)*
        #[command(subcommand)]
        pub #ident: #ty,
      }
    });

  let extra_field = attrs.extra_cli.as_ref().map(|extra_ty| {
    quote! {
      #[command(flatten)]
      pub extra: #extra_ty,
    }
  });

  // Common fields are inlined (rather than flattened from a shared
  // `CommonCli` struct) so each app gets per-app-prefixed env-var
  // names — `<prefix>_log_level`, etc.  A single shared struct can't
  // do that, since clap bakes the env name into the struct's own
  // attributes at the struct's compile site.
  let log_level_env = ::std::format!("{}_log_level", prefix);
  let log_format_env = ::std::format!("{}_log_format", prefix);
  let config_env = ::std::format!("{}_config", prefix);

  // NOTE: `Option<...>` is intentionally unqualified — clap's
  // `_infer_ValueParser_for` derive matches on the syntactic form
  // `Option<T>`, and a fully-qualified `::std::option::Option<T>`
  // doesn't trigger the inference path.  `Option`, `String`, and
  // `std::path::PathBuf` resolve via the user crate's prelude /
  // standard library namespace.
  quote! {
    #[derive(::std::fmt::Debug, ::clap::Parser)]
    #[command(name = #app_name, version, about)]
    pub struct CliRaw {
      /// Log level (trace, debug, info, warn, error).
      #[arg(long, env = #log_level_env)]
      pub log_level: Option<String>,
      /// Log format (text, json).
      #[arg(long, env = #log_format_env)]
      pub log_format: Option<String>,
      /// Path to configuration file.
      #[arg(short, long, env = #config_env)]
      pub config: Option<std::path::PathBuf>,
      #(#field_defs)*
      #subcommand_field
      #extra_field
    }
  }
}

fn mc_gen_config_file_raw(
  fields: &[MergeConfigFieldInfo],
  attrs: &MergeConfigStructAttrs,
) -> proc_macro2::TokenStream {
  let field_defs: Vec<_> = fields
    .iter()
    .filter_map(|f| {
      let MergeConfigFieldKind::Merged(m) = &f.kind else {
        return None;
      };
      if m.cli_only {
        return None;
      }

      let field_ty = if m.parse {
        quote! { Option<String> }
      } else {
        let ty = &f.ty;
        quote! { Option<#ty> }
      };

      let raw_name = &m.raw_name;
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

fn mc_gen_config_error(
  attrs: &MergeConfigStructAttrs,
) -> proc_macro2::TokenStream {
  let extra_variant = attrs.extra_error.as_ref().map(|ty| {
    quote! {
      #[error(transparent)]
      Extra(
        #[from]
        #ty,
      ),
    }
  });

  // The generated `ConfigError` derives thiserror's `Error` via the consuming
  // crate's own `thiserror`, not foundation's.  thiserror's derive expands to
  // bare `thiserror::...` paths that must resolve in the crate the code lands
  // in, so re-exporting the path through foundation is not enough — the
  // consumer must depend on thiserror directly.  Using `::thiserror` keeps the
  // derive and its generated references on one version (the consumer's), which
  // is also the version the consumer uses for its own error types.
  quote! {
    #[derive(
      ::std::fmt::Debug,
      ::thiserror::Error,
    )]
    pub enum ConfigError {
      #[error("Failed to load configuration file: {0}")]
      File(
        #[from]
        ::rust_template_foundation::config::ConfigFileError,
      ),
      #[error("Configuration validation failed: {0}")]
      Validation(::std::string::String),
      #extra_variant
    }
  }
}

fn mc_gen_from_cli_and_file(
  struct_name: &Ident,
  fields: &[MergeConfigFieldInfo],
  attrs: &MergeConfigStructAttrs,
) -> proc_macro2::TokenStream {
  let app_name = &attrs.app_name;

  // Find common field idents by name.
  let log_level_ident = fields
    .iter()
    .find(|f| {
      matches!(f.kind, MergeConfigFieldKind::Common) && f.ident == "log_level"
    })
    .map(|f| &f.ident);
  let log_format_ident = fields
    .iter()
    .find(|f| {
      matches!(f.kind, MergeConfigFieldKind::Common) && f.ident == "log_format"
    })
    .map(|f| &f.ident);

  let log_resolve = match (log_level_ident, log_format_ident) {
    (Some(lvl), Some(fmt)) => quote! {
      let (#lvl, #fmt) =
        ::rust_template_foundation::config::resolve_log_settings(
          cli.log_level.clone(),
          cli.log_format.clone(),
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
      if !matches!(f.kind, MergeConfigFieldKind::Skip) {
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

  // Subcommand passthrough — partial move out of `cli`.  Order
  // relative to merged stmts is irrelevant; each field moves
  // independently.
  let subcommand_stmt = fields.iter().find_map(|f| {
    if !matches!(f.kind, MergeConfigFieldKind::Subcommand) {
      return None;
    }
    let field_name = &f.ident;
    Some(quote! {
      let #field_name = cli.#field_name;
    })
  });

  // Merged field resolution (moves from cli/file).
  let merge_stmts: Vec<_> = fields
    .iter()
    .filter_map(|f| {
      let MergeConfigFieldKind::Merged(m) = &f.kind else {
        return None;
      };

      let field_name = &f.ident;
      let field_ty = &f.ty;
      let raw_name = &m.raw_name;

      let or_file = if m.cli_only {
        quote! {}
      } else {
        quote! { .or(file.#raw_name) }
      };

      // Three-arm chain (default / required / otherwise) — collapsing
      // this to map_or_else would force a nested closure picking
      // between two quote!{} blocks, which reads worse than the
      // explicit if/else if ladder.
      #[allow(clippy::option_if_let_else)]
      let unwrap = if let Some(default_expr) = &m.default {
        quote! { .unwrap_or_else(|| #default_expr) }
      } else if m.required {
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

      if m.parse {
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
            cli.config.as_deref(),
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
        #subcommand_stmt

        ::std::result::Result::Ok(#struct_name {
          #(#field_names),*
        })
      }
    }
  }
}

fn mc_gen_cli_app_impl(
  struct_name: &Ident,
  fields: &[MergeConfigFieldInfo],
  attrs: &MergeConfigStructAttrs,
) -> proc_macro2::TokenStream {
  let app_name = &attrs.app_name;

  let log_level_ident = fields
    .iter()
    .find(|f| {
      matches!(f.kind, MergeConfigFieldKind::Common) && f.ident == "log_level"
    })
    .map(|f| &f.ident);
  let log_format_ident = fields
    .iter()
    .find(|f| {
      matches!(f.kind, MergeConfigFieldKind::Common) && f.ident == "log_format"
    })
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

  let Data::Struct(DataStruct {
    fields: Fields::Named(FieldsNamed { named, .. }),
    ..
  }) = &input.data
  else {
    return Err(syn::Error::new_spanned(
      &input,
      "MergeConfig requires a struct with named fields",
    ));
  };

  let field_infos: Vec<MergeConfigFieldInfo> = named
    .iter()
    .map(mc_parse_field)
    .collect::<syn::Result<_>>()?;

  // Validate common fields.
  let common_count = field_infos
    .iter()
    .filter(|f| matches!(f.kind, MergeConfigFieldKind::Common))
    .count();
  if common_count != 2 {
    return Err(syn::Error::new_spanned(
      &input,
      "MergeConfig requires exactly two `common` fields \
       (log_level and log_format)",
    ));
  }
  let has_log_level = field_infos.iter().any(|f| {
    matches!(f.kind, MergeConfigFieldKind::Common) && f.ident == "log_level"
  });
  let has_log_format = field_infos.iter().any(|f| {
    matches!(f.kind, MergeConfigFieldKind::Common) && f.ident == "log_format"
  });
  if !has_log_level || !has_log_format {
    return Err(syn::Error::new_spanned(
      &input,
      "common fields must be named `log_level` and \
       `log_format`",
    ));
  }

  let subcommand_count = field_infos
    .iter()
    .filter(|f| matches!(f.kind, MergeConfigFieldKind::Subcommand))
    .count();
  if subcommand_count > 1 {
    return Err(syn::Error::new_spanned(
      &input,
      "MergeConfig allows at most one `subcommand` field",
    ));
  }

  let cli_raw = mc_gen_cli_raw(&field_infos, &attrs);
  let config_file_raw = mc_gen_config_file_raw(&field_infos, &attrs);
  let config_error = mc_gen_config_error(&attrs);
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
      let all_server = tuple_ty.elems.iter().all(type_is_server);
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
      .is_some_and(|seg| seg.ident == "Server"),
    Type::Tuple(tuple) => {
      // A tuple of Servers is also a server param.
      !tuple.elems.is_empty() && tuple.elems.iter().all(type_is_server)
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
  // Each arm builds a multi-line `quote!{}` block whose shape differs
  // entirely between the tuple and single-server cases — folding this
  // into Option::map_or_else would push both quote bodies inside
  // closures and obscure the structural difference between them.
  #[allow(clippy::option_if_let_else)]
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
      let __rt = match ::tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
          ::std::eprintln!("Failed to create tokio runtime: {}", e);
          return ::std::process::ExitCode::FAILURE;
        }
      };

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

      let __rt = match ::tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
          ::std::eprintln!("Failed to create tokio runtime: {}", e);
          return ::std::process::ExitCode::FAILURE;
        }
      };

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

#[cfg(test)]
mod tests {
  use super::*;
  use syn::parse::Parser;

  /// A named struct field parsed from its token form.
  fn field(tokens: proc_macro2::TokenStream) -> syn::Field {
    syn::Field::parse_named
      .parse2(tokens)
      .expect("the field tokens to parse")
  }

  /// The merged binding of a field parsed from its token form.
  fn merged(tokens: proc_macro2::TokenStream) -> Box<MergeConfigMergedArg> {
    match mc_parse_field(&field(tokens)).unwrap().kind {
      MergeConfigFieldKind::Merged(merged) => merged,
      _ => panic!("the field to be a merged field"),
    }
  }

  /// The error a field's attributes produce.
  fn error_of(tokens: proc_macro2::TokenStream) -> String {
    mc_parse_field(&field(tokens))
      .err()
      .expect("the field to be rejected")
      .to_string()
  }

  #[test]
  fn a_plain_field_reads_the_derived_variable() {
    let binding = merged(quote! {
      #[merge_config(default = "0")]
      pub port: u32
    });
    assert!(matches!(binding.env, MergeConfigArgEnv::Auto));
  }

  #[test]
  fn no_env_opts_a_field_out() {
    let binding = merged(quote! {
      #[merge_config(no_env, default = "String::new()")]
      pub token: String
    });
    assert!(matches!(binding.env, MergeConfigArgEnv::Disabled));
  }

  #[test]
  fn a_literal_env_names_its_own_variable() {
    let binding = merged(quote! {
      #[merge_config(env = "LEGACY_PORT", default = "0")]
      pub port: u32
    });
    assert!(matches!(
      binding.env,
      MergeConfigArgEnv::Literal(ref name) if name.value() == "LEGACY_PORT"
    ));
  }

  #[test]
  fn a_bare_env_is_refused_with_the_fix_named() {
    let error = error_of(quote! {
      #[merge_config(env, default = "0")]
      pub port: u32
    });
    assert!(error.contains("no_env"), "{error}");
  }

  #[test]
  fn no_env_and_a_literal_env_are_mutually_exclusive() {
    let error = error_of(quote! {
      #[merge_config(no_env, env = "PORT", default = "0")]
      pub port: u32
    });
    assert!(error.contains("mutually exclusive"), "{error}");
  }

  #[test]
  fn no_env_is_refused_on_a_subcommand_field() {
    let error = error_of(quote! {
      #[merge_config(subcommand, no_env)]
      pub command: Commands
    });
    assert!(error.contains("not allowed on a `subcommand` field"), "{error}");
  }
}
