//! Test helpers for foundation integration tests.

use openidconnect::{
  core::{
    CoreClient, CoreJwsSigningAlgorithm, CoreProviderMetadata,
    CoreResponseType, CoreSubjectIdentifierType,
  },
  AuthUrl, ClientId, EmptyAdditionalProviderMetadata, IssuerUrl,
  JsonWebKeySetUrl, ResponseTypes,
};
use std::sync::Arc;

/// Create a stub OIDC client for tests that need auth enabled but
/// don't perform real discovery.
pub fn stub_oidc_client() -> Arc<CoreClient> {
  let issuer = IssuerUrl::new("https://stub.invalid".to_string()).unwrap();
  let metadata = CoreProviderMetadata::new(
    issuer,
    AuthUrl::new("https://stub.invalid/authorize".to_string()).unwrap(),
    JsonWebKeySetUrl::new("https://stub.invalid/jwks".to_string()).unwrap(),
    vec![ResponseTypes::new(vec![CoreResponseType::Code])],
    vec![CoreSubjectIdentifierType::Public],
    vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
    EmptyAdditionalProviderMetadata {},
  );
  let client = CoreClient::from_provider_metadata(
    metadata,
    ClientId::new("stub-client".to_string()),
    None,
  );
  Arc::new(client)
}
