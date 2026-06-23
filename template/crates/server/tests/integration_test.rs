// Integration tests under tests/ may use the panicking variants (unwrap,
// expect, panic) freely — see llms.org's "No unwrap or expect" test exemption.
// clippy's is_in_test heuristic does not recognize tests/ integration tests as
// test code, so the workspace-level denials reach them and must be allowed at
// the file level.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

// ── config tests ────────────────────────────────────────────────────

#[tokio::test]
async fn test_config_no_oidc() {
  use rust_template_server::config::{CliRaw, Config, OidcCliFields};

  let cli = CliRaw {
    log_level: None,
    log_format: None,
    config: None,
    listen: None,
    frontend_path: None,
    base_url: Some("https://example.com".to_string()),
    extra: OidcCliFields {
      oidc_issuer: None,
      oidc_client_id: None,
      oidc_client_secret_file: None,
    },
  };

  let config = Config::from_cli_and_file(cli).unwrap();
  assert!(config.oidc.is_none());
}

#[tokio::test]
async fn test_config_full_oidc() {
  use rust_template_server::config::{CliRaw, Config, OidcCliFields};

  let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/oidc-client-secret");

  let cli = CliRaw {
    log_level: None,
    log_format: None,
    config: None,
    listen: None,
    frontend_path: None,
    base_url: Some("https://example.com".to_string()),
    extra: OidcCliFields {
      oidc_issuer: Some("https://sso.example.com".to_string()),
      oidc_client_id: Some("my-client".to_string()),
      oidc_client_secret_file: Some(fixture),
    },
  };

  let config = Config::from_cli_and_file(cli).unwrap();
  let oidc = config.oidc.expect("OIDC config should be Some");
  assert_eq!(oidc.issuer, "https://sso.example.com");
  assert_eq!(oidc.client_id, "my-client");
  assert_eq!(oidc.client_secret, "test-secret-not-for-production");
}

#[tokio::test]
async fn test_config_partial_oidc_errors() {
  use rust_template_server::config::{CliRaw, Config, OidcCliFields};

  let cli = CliRaw {
    log_level: None,
    log_format: None,
    config: None,
    listen: None,
    frontend_path: None,
    base_url: Some("https://example.com".to_string()),
    extra: OidcCliFields {
      oidc_issuer: Some("https://sso.example.com".to_string()),
      oidc_client_id: None,
      oidc_client_secret_file: None,
    },
  };

  let err = Config::from_cli_and_file(cli).unwrap_err();
  let msg = err.to_string();
  assert!(
    msg.contains("partial OIDC") && msg.contains("missing"),
    "error should describe partial OIDC config, got: {msg}"
  );
}
