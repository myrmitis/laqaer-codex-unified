use async_trait::async_trait;
use browser_rpc::{
    BrowserFailureCode, BrowserRpcBackend, BrowserRpcError, BrowserSessionState,
    BrowserTurnIdentity, BrowserTurnRequest, BrowserTurnResult, WebMode,
};
use codex_unified_core::{Provider, ProviderCapabilities, ProviderError, ProviderEventStream};
use codex_unified_protocol::{CanonicalEvent, FailureCode, TurnEnvelope};
use futures_util::stream;
use std::sync::Arc;

pub mod browser_rpc;

pub struct WebProvider {
    rpc: Arc<dyn BrowserRpcBackend>,
}

impl WebProvider {
    pub fn new(rpc: Arc<dyn BrowserRpcBackend>) -> Self {
        Self { rpc }
    }

    async fn preflight(&self) -> Result<(), ProviderError> {
        let snapshot = self.rpc.session().await.map_err(map_rpc_error)?;
        match snapshot.state {
            BrowserSessionState::Healthy => Ok(()),
            BrowserSessionState::Challenged => Err(ProviderError::Classified {
                code: FailureCode::SecurityChallenge,
                message: "ChatGPT browser session is blocked by a security challenge".into(),
                retryable: false,
            }),
            BrowserSessionState::LoggedOut
            | BrowserSessionState::Authenticating
            | BrowserSessionState::NeedsReauth => Err(ProviderError::Classified {
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
            tools: false,
            images: false,
            continuation: false,
        }
    }

    async fn execute(&self, turn: TurnEnvelope) -> Result<ProviderEventStream, ProviderError> {
        self.preflight().await?;

        let mode = web_mode_for_model(&turn.requested_model)?;
        let request = BrowserTurnRequest::new(
            turn.trace_id.to_string(),
            BrowserTurnIdentity {
                thread_id: turn.identity.thread_id.clone(),
                turn_id: turn.identity.turn_id.clone(),
                request_kind: turn.identity.request_kind.clone(),
            },
            mode,
            compile_browser_prompt(&turn),
            turn.previous_response_id.clone(),
        );

        let result = self.rpc.execute_turn(&request).await.map_err(map_rpc_error)?;
        match result {
            BrowserTurnResult::Success { response_id, text } => {
                let item_id = format!("{response_id}-text");
                let mut events = vec![Ok(CanonicalEvent::ResponseCreated {
                    response_id: response_id.clone(),
                })];
                if !text.is_empty() {
                    events.push(Ok(CanonicalEvent::TextDelta {
                        item_id,
                        delta: text,
                    }));
                }
                events.push(Ok(CanonicalEvent::ResponseCompleted { response_id }));
                Ok(Box::pin(stream::iter(events)))
            }
            BrowserTurnResult::Failure {
                code,
                message,
                retryable: _,
            } => Err(map_browser_failure(code, message)),
        }
    }
}

pub fn web_mode_for_model(model: &str) -> Result<WebMode, ProviderError> {
    match model {
        "chatgpt-web/instant" => Ok(WebMode::Instant),
        "chatgpt-web/medium" => Ok(WebMode::Medium),
        "chatgpt-web/high" => Ok(WebMode::High),
        "chatgpt-web/extra-high" => Ok(WebMode::ExtraHigh),
        "chatgpt-web/pro" => Ok(WebMode::Pro),
        _ => Err(ProviderError::Classified {
            code: FailureCode::ModelUnavailable,
            message: "ChatGPT Web model route is unavailable".into(),
            retryable: false,
        }),
    }
}

fn compile_browser_prompt(turn: &TurnEnvelope) -> String {
    let input = serde_json::to_string(&turn.input).unwrap_or_else(|_| "null".into());
    format!(
        "The following is the canonical Codex Responses input JSON. Follow its messages and instructions in order.\n\n{input}"
    )
}

fn map_browser_failure(code: BrowserFailureCode, message: String) -> ProviderError {
    let code = match code {
        BrowserFailureCode::WebAuthRequired => FailureCode::AuthRequired,
        BrowserFailureCode::WebSecurityChallenge => FailureCode::SecurityChallenge,
        BrowserFailureCode::WebModelUnavailable => FailureCode::ModelUnavailable,
        BrowserFailureCode::WebDomContractChanged => FailureCode::DomContractChanged,
        BrowserFailureCode::WebContinuationMissing => FailureCode::ContinuationMissing,
        BrowserFailureCode::WebTransportFailed => FailureCode::TransportFailed,
    };

    ProviderError::Classified {
        code,
        message,
        // Browser replay is fail-closed until the worker reports an explicit
        // pre-Send submission phase. Do not trust a generic retryable bit.
        retryable: false,
    }
}

fn map_rpc_error(error: BrowserRpcError) -> ProviderError {
    let code = match error {
        BrowserRpcError::Unauthorized => FailureCode::AuthRequired,
        BrowserRpcError::HttpStatus(429) => FailureCode::RateLimited,
        BrowserRpcError::NonLoopbackOrigin
        | BrowserRpcError::InvalidCapability
        | BrowserRpcError::ProtocolMismatch { .. }
        | BrowserRpcError::HttpStatus(_)
        | BrowserRpcError::InvalidResponse
        | BrowserRpcError::Transport(_) => FailureCode::TransportFailed,
    };

    ProviderError::Classified {
        code,
        message: error.to_string(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use browser_rpc::{BrowserSessionSnapshot, BrowserTurnResult};
    use futures_util::StreamExt;
    use std::sync::{Arc, Mutex};

    struct MockBackend {
        session: BrowserSessionSnapshot,
        result: BrowserTurnResult,
        requests: Mutex<Vec<BrowserTurnRequest>>,
    }

    impl MockBackend {
        fn new(session: BrowserSessionState, result: BrowserTurnResult) -> Self {
            Self {
                session: BrowserSessionSnapshot {
                    state: session,
                    revision: 1,
                },
                result,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<BrowserTurnRequest> {
            self.requests.lock().expect("requests lock").clone()
        }
    }

    #[async_trait]
    impl BrowserRpcBackend for MockBackend {
        async fn session(&self) -> Result<BrowserSessionSnapshot, BrowserRpcError> {
            Ok(self.session.clone())
        }

        async fn execute_turn(
            &self,
            request: &BrowserTurnRequest,
        ) -> Result<BrowserTurnResult, BrowserRpcError> {
            self.requests
                .lock()
                .expect("requests lock")
                .push(request.clone());
            Ok(self.result.clone())
        }
    }

    fn turn(model: &str) -> TurnEnvelope {
        let metadata = serde_json::json!({
            "thread_id": "thread-1",
            "turn_id": "turn-1",
            "request_kind": "turn"
        });
        TurnEnvelope::from_responses_request(serde_json::json!({
            "model": model,
            "client_metadata": {
                "x-codex-turn-metadata": serde_json::to_string(&metadata)
                    .expect("metadata")
            },
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }]
        }))
        .expect("turn fixture")
    }

    #[test]
    fn pro_and_extra_high_are_distinct_routes() {
        assert_eq!(
            web_mode_for_model("chatgpt-web/extra-high").expect("extra high"),
            WebMode::ExtraHigh
        );
        assert_eq!(
            web_mode_for_model("chatgpt-web/pro").expect("pro"),
            WebMode::Pro
        );
    }

    #[tokio::test]
    async fn unhealthy_session_fails_before_turn_execution() {
        let backend = Arc::new(MockBackend::new(
            BrowserSessionState::NeedsReauth,
            BrowserTurnResult::Success {
                response_id: "should-not-run".into(),
                text: "no".into(),
            },
        ));
        let provider = WebProvider::new(backend.clone());

        let error = provider
            .execute(turn("chatgpt-web/instant"))
            .await
            .expect_err("preflight must fail");

        assert!(matches!(
            error,
            ProviderError::Classified {
                code: FailureCode::AuthRequired,
                retryable: false,
                ..
            }
        ));
        assert!(backend.requests().is_empty());
    }

    #[tokio::test]
    async fn pro_success_maps_to_canonical_responses_events() {
        let backend = Arc::new(MockBackend::new(
            BrowserSessionState::Healthy,
            BrowserTurnResult::Success {
                response_id: "response-pro".into(),
                text: "PRO OK".into(),
            },
        ));
        let provider = WebProvider::new(backend.clone());

        let events: Vec<_> = provider
            .execute(turn("chatgpt-web/pro"))
            .await
            .expect("provider stream")
            .map(|event| event.expect("canonical event"))
            .collect()
            .await;

        let requests = backend.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].mode, WebMode::Pro);
        assert!(requests[0].prompt.contains("hello"));

        assert!(matches!(
            &events[0],
            CanonicalEvent::ResponseCreated { response_id }
                if response_id == "response-pro"
        ));
        assert!(matches!(
            &events[1],
            CanonicalEvent::TextDelta { delta, .. } if delta == "PRO OK"
        ));
        assert!(matches!(
            &events[2],
            CanonicalEvent::ResponseCompleted { response_id }
                if response_id == "response-pro"
        ));
    }

    #[tokio::test]
    async fn browser_auth_failure_is_not_retried() {
        let backend = Arc::new(MockBackend::new(
            BrowserSessionState::Healthy,
            BrowserTurnResult::Failure {
                code: BrowserFailureCode::WebAuthRequired,
                message: "reauthenticate".into(),
                retryable: true,
            },
        ));
        let provider = WebProvider::new(backend);

        let error = provider
            .execute(turn("chatgpt-web/high"))
            .await
            .expect_err("browser failure");

        assert!(matches!(
            error,
            ProviderError::Classified {
                code: FailureCode::AuthRequired,
                retryable: false,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn capabilities_do_not_claim_unimplemented_tool_or_image_support() {
        let backend = Arc::new(MockBackend::new(
            BrowserSessionState::Healthy,
            BrowserTurnResult::Success {
                response_id: "r".into(),
                text: String::new(),
            },
        ));
        let capabilities = WebProvider::new(backend)
            .capabilities("chatgpt-web/instant")
            .await;

        assert!(capabilities.streaming);
        assert!(!capabilities.tools);
        assert!(!capabilities.images);
        assert!(!capabilities.continuation);
    }
}
