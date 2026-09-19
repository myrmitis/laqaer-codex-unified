use async_trait::async_trait;
use codex_unified_core::{Provider, ProviderCapabilities, ProviderError, ProviderEventStream};
use codex_unified_protocol::{CanonicalEvent, TurnEnvelope};
use futures_util::stream;

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

    async fn execute(&self, turn: TurnEnvelope) -> Result<ProviderEventStream, ProviderError> {
        let response_id = format!("api-stub-{}", turn.identity.turn_id);
        Ok(Box::pin(stream::iter(vec![
            Ok(CanonicalEvent::ResponseCreated {
                response_id: response_id.clone(),
            }),
            Ok(CanonicalEvent::ResponseCompleted { response_id }),
        ])))
    }
}
