use async_trait::async_trait;
use codex_unified_core::{Provider, ProviderCapabilities, ProviderError};
use codex_unified_protocol::{CanonicalEvent, TurnEnvelope};

/// Scaffold for ordinary API-backed providers.
///
/// Provider transforms are local to this adapter. There is intentionally no
/// global normalization stage that can erase Codex-native identity for another
/// provider family.
pub struct ApiProvider;

#[async_trait]
impl Provider for ApiProvider {
    fn id(&self) -> &'static str {
        "api"
    }

    async fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tools: true,
            images: true,
            continuation: true,
        }
    }

    async fn execute(
        &self,
        turn: TurnEnvelope,
    ) -> Result<Vec<CanonicalEvent>, ProviderError> {
        let response_id = format!("api-stub-{}", turn.identity.turn_id);
        Ok(vec![
            CanonicalEvent::ResponseCreated {
                response_id: response_id.clone(),
            },
            CanonicalEvent::ResponseCompleted { response_id },
        ])
    }
}
