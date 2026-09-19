use async_trait::async_trait;
use codex_unified_core::ProviderError;
use codex_unified_protocol::FailureCode;
use codex_unified_provider_api::CredentialResolver;
use std::sync::Arc;

const KEYCHAIN_SCHEME: &str = "keychain://";

#[derive(Debug, Clone)]
pub struct KeyringCredentialResolver {
    service: Arc<str>,
}

impl KeyringCredentialResolver {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: Arc::from(service.into()),
        }
    }

    pub fn codex_unified() -> Self {
        Self::new("codex-unified")
    }
}

#[async_trait]
impl CredentialResolver for KeyringCredentialResolver {
    async fn resolve(&self, credential_ref: &str) -> Result<String, ProviderError> {
        let account = parse_keychain_ref(credential_ref)?;
        let service = Arc::clone(&self.service);

        let result = tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(&service, &account)
                .map_err(|_| CredentialStoreError::Unavailable)?;
            let secret = entry
                .get_password()
                .map_err(|_| CredentialStoreError::Missing)?;
            if secret.is_empty() {
                return Err(CredentialStoreError::Missing);
            }
            Ok(secret)
        })
        .await
        .map_err(|_| classified(
            FailureCode::TransportFailed,
            "credential lookup task failed",
            false,
        ))?;

        result.map_err(|error| match error {
            CredentialStoreError::Unavailable => classified(
                FailureCode::AuthRequired,
                "OS credential store is unavailable",
                false,
            ),
            CredentialStoreError::Missing => classified(
                FailureCode::AuthRequired,
                "provider credential is unavailable",
                false,
            ),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialStoreError {
    Unavailable,
    Missing,
}

fn parse_keychain_ref(credential_ref: &str) -> Result<String, ProviderError> {
    let Some(account) = credential_ref.strip_prefix(KEYCHAIN_SCHEME) else {
        return Err(classified(
            FailureCode::InvalidProviderResponse,
            "unsupported credential reference scheme",
            false,
        ));
    };

    if account.is_empty()
        || account.contains('/')
        || account.contains('?')
        || account.contains('#')
        || account.chars().any(char::is_whitespace)
    {
        return Err(classified(
            FailureCode::InvalidProviderResponse,
            "invalid keychain credential reference",
            false,
        ));
    }

    Ok(account.to_owned())
}

fn classified(
    code: FailureCode,
    message: impl Into<String>,
    retryable: bool,
) -> ProviderError {
    ProviderError::Classified {
        code,
        message: message.into(),
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bounded_keychain_reference() {
        assert_eq!(
            parse_keychain_ref("keychain://openrouter").expect("valid reference"),
            "openrouter"
        );
    }

    #[test]
    fn rejects_non_keychain_scheme() {
        let error = parse_keychain_ref("oauth://xai").expect_err("must reject");
        assert!(matches!(
            error,
            ProviderError::Classified {
                code: FailureCode::InvalidProviderResponse,
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn rejects_path_like_or_empty_account() {
        for value in [
            "keychain://",
            "keychain://provider/account",
            "keychain://provider?query",
            "keychain://provider#fragment",
            "keychain://provider name",
        ] {
            assert!(parse_keychain_ref(value).is_err(), "{value} must fail");
        }
    }
}
