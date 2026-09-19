use async_trait::async_trait;
use codex_unified_core::{Provider, ProviderCapabilities, ProviderError};
use codex_unified_protocol::{CanonicalEvent, FailureCode, TurnEnvelope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserSessionState {
    LoggedOut,
    Authenticating,
    Healthy,
    Challenged,
    NeedsReauth,
}

pub struct WebProvider {
    pub session: BrowserSessionState,
}

impl WebProvider {
    pub fn preflight(&self) -> Result<(), ProviderError> {
        match self.session {
            BrowserSessionState::Healthy => Ok(()),
            BrowserSessionState::Challenged => Err(ProviderError::Classified {
                code: FailureCode::SecurityChallenge,
                message: "ChatGPT browser session is blocked by a security challenge".into(),
                retryable: false,
            }),
            _ => Err(ProviderError::Classified {
                code: FailureCode::AuthRequired,
                message: "ChatGPT browser session requires explicit sign-in".into(),
                retryable: false,
            }),
        }
    }
}

#[async_trait]
impl Provider for WebProvider {
    fn id(&self) -> &'static str {
        "chatgpt-web"
    }

    async fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tools: true,
            images: true,
            continuation: true,
        }
    }

    async fn execute(&self, turn: TurnEnvelope) -> Result<Vec<CanonicalEvent>, ProviderError> {
        self.preflight()?;

        // Browser RPC lands in Phase 2. The provider already enforces the
        // critical invariant: an unhealthy session never starts physical work.
        let response_id = format!("web-stub-{}", turn.identity.turn_id);
        Ok(vec![
            CanonicalEvent::ResponseCreated {
                response_id: response_id.clone(),
            },
            CanonicalEvent::ResponseCompleted { response_id },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_auth_never_enters_a_turn() {
        let provider = WebProvider {
            session: BrowserSessionState::NeedsReauth,
        };

        let error = provider.preflight().expect_err("must fail closed");
        assert!(matches!(
            error,
            ProviderError::Classified {
                code: FailureCode::AuthRequired,
                retryable: false,
                ..
            }
        ));
    }
}
