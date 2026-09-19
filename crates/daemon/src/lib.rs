use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use codex_unified_core::{EmptyProviderResolver, ProviderResolver, validated_event_stream};
use codex_unified_protocol::{TurnEnvelope, TurnEnvelopeError};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc};

#[derive(Clone)]
pub struct AppConfig {
    pub capability: String,
    pub providers: Arc<dyn ProviderResolver>,
}

impl AppConfig {
    pub fn foundation(capability: impl Into<String>) -> Self {
        Self {
            capability: capability.into(),
            providers: Arc::new(EmptyProviderResolver),
        }
    }
}

#[derive(Clone)]
struct AppState {
    capability: Arc<str>,
    providers: Arc<dyn ProviderResolver>,
}

pub fn app(config: AppConfig) -> Router {
    let state = AppState {
        capability: Arc::<str>::from(config.capability),
        providers: config.providers,
    };

    Router::new()
        .route("/healthz", get(health))
        .route("/_codex-unified/{capability}/v1/responses", post(responses))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({
        "service": "codex-unifiedd",
        "status": "ok",
        "protocol": 1
    }))
}

async fn responses(
    Path(capability): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    if !constant_time_equal(capability.as_bytes(), state.capability.as_bytes()) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "local_auth_required",
            "The local Codex Unified capability is invalid.",
        );
    }

    let turn = match TurnEnvelope::from_responses_request(payload) {
        Ok(turn) => turn,
        Err(error) => return turn_error_response(error),
    };

    let provider = match state.providers.resolve(&turn.requested_model) {
        Some(provider) => provider,
        None => {
            return error_response(
                StatusCode::NOT_FOUND,
                "model_not_routable",
                "No enabled provider route owns the requested model.",
            );
        }
    };

    tracing::info!(
        trace_id = %turn.trace_id,
        model = %turn.requested_model,
        provider = provider.id(),
        "responses ingress accepted"
    );

    let provider_stream = match provider.execute(turn).await {
        Ok(stream) => stream,
        Err(error) => {
            let event = error.failure_event(None);
            return sse_response(futures_util::stream::iter(vec![Ok(event)]));
        }
    };

    sse_response(validated_event_stream(provider_stream))
}

fn sse_response<S>(stream: S) -> Response
where
    S: futures_core::Stream<
            Item = Result<
                codex_unified_protocol::CanonicalEvent,
                codex_unified_core::ProviderError,
            >,
        > + Send
        + 'static,
{
    let events = stream.map(|result| {
        let event = match result {
            Ok(event) => event,
            Err(error) => error.failure_event(None),
        };
        let wire = event.to_responses_wire();
        Ok::<_, Infallible>(
            Event::default()
                .event(event.responses_event_name())
                .json_data(wire)
                .expect("canonical Responses event must serialize"),
        )
    });

    Sse::new(events)
        .keep_alive(KeepAlive::new())
        .into_response()
}

fn turn_error_response(error: TurnEnvelopeError) -> Response {
    let code = match &error {
        TurnEnvelopeError::MissingTurnMetadata | TurnEnvelopeError::MissingTurnId => {
            "native_turn_metadata_required"
        }
        _ => "invalid_request",
    };
    error_response(StatusCode::BAD_REQUEST, code, &error.to_string())
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "type": "invalid_request_error",
                "code": code,
                "message": message
            }
        })),
    )
        .into_response()
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.iter()
        .zip(right)
        .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
    };
    use codex_unified_core::{Provider, ProviderCapabilities, ProviderError, ProviderEventStream};
    use codex_unified_protocol::{CanonicalEvent, TurnEnvelope};
    use futures_util::stream;
    use std::sync::Arc;
    use tower::ServiceExt;

    const CAPABILITY: &str = "test-capability-0123456789";

    struct TestResolver {
        provider: Option<Arc<dyn Provider>>,
    }

    impl ProviderResolver for TestResolver {
        fn resolve(&self, _model: &str) -> Option<Arc<dyn Provider>> {
            self.provider.clone()
        }
    }

    struct ScriptedProvider {
        abrupt_eof: bool,
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn id(&self) -> &'static str {
            "scripted"
        }

        async fn capabilities(&self, _model: &str) -> ProviderCapabilities {
            ProviderCapabilities {
                streaming: true,
                tools: true,
                images: false,
                continuation: true,
            }
        }

        async fn execute(&self, turn: TurnEnvelope) -> Result<ProviderEventStream, ProviderError> {
            let response_id = format!("resp-{}", turn.identity.turn_id);
            let mut events = vec![
                Ok(CanonicalEvent::ResponseCreated {
                    response_id: response_id.clone(),
                }),
                Ok(CanonicalEvent::TextDelta {
                    item_id: "msg-1".into(),
                    delta: "OK".into(),
                }),
            ];
            if !self.abrupt_eof {
                events.push(Ok(CanonicalEvent::ResponseCompleted { response_id }));
            }
            Ok(Box::pin(stream::iter(events)))
        }
    }

    fn test_app(provider: Option<Arc<dyn Provider>>) -> Router {
        app(AppConfig {
            capability: CAPABILITY.to_owned(),
            providers: Arc::new(TestResolver { provider }),
        })
    }

    fn payload() -> Value {
        let metadata = serde_json::json!({
            "thread_id": "thread-1",
            "turn_id": "turn-1",
            "request_kind": "turn"
        });
        serde_json::json!({
            "model": "test/model",
            "client_metadata": {
                "x-codex-turn-metadata": serde_json::to_string(&metadata)
                    .expect("encode metadata")
            },
            "stream": true,
            "input": []
        })
    }

    async fn post_json(
        app: Router,
        path_capability: &str,
        payload: Value,
    ) -> (StatusCode, String, String) {
        let request = Request::builder()
            .method("POST")
            .uri(format!("/_codex-unified/{path_capability}/v1/responses"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&payload).expect("serialize fixture"),
            ))
            .expect("request fixture");

        let response = app.oneshot(request).await.expect("router response");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response");
        (
            status,
            content_type,
            String::from_utf8(body.to_vec()).expect("UTF-8 body"),
        )
    }

    #[tokio::test]
    async fn rejects_wrong_local_capability() {
        let (status, _, body) = post_json(test_app(None), "wrong-capability", payload()).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let body: Value = serde_json::from_str(&body).expect("JSON response");
        assert_eq!(body["error"]["code"], "local_auth_required");
    }

    #[tokio::test]
    async fn rejects_missing_native_turn_identity() {
        let (status, _, body) = post_json(
            test_app(None),
            CAPABILITY,
            serde_json::json!({
                "model": "chatgpt-web/instant",
                "input": []
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        let body: Value = serde_json::from_str(&body).expect("JSON response");
        assert_eq!(body["error"]["code"], "native_turn_metadata_required");
    }

    #[tokio::test]
    async fn rejects_unroutable_model_before_streaming() {
        let (status, _, body) = post_json(test_app(None), CAPABILITY, payload()).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        let body: Value = serde_json::from_str(&body).expect("JSON response");
        assert_eq!(body["error"]["code"], "model_not_routable");
    }

    #[tokio::test]
    async fn streams_canonical_responses_sse() {
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider { abrupt_eof: false });
        let (status, content_type, body) =
            post_json(test_app(Some(provider)), CAPABILITY, payload()).await;

        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/event-stream"));
        assert!(body.contains("event: response.created"));
        assert!(body.contains("event: response.output_text.delta"));
        assert!(body.contains("event: response.completed"));
        assert!(body.contains("\"delta\":\"OK\""));
    }

    #[tokio::test]
    async fn abrupt_provider_eof_streams_failure_and_never_completion() {
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider { abrupt_eof: true });
        let (status, _, body) = post_json(test_app(Some(provider)), CAPABILITY, payload()).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("event: response.failed"));
        assert!(body.contains("invalid_provider_response"));
        assert!(!body.contains("event: response.completed"));
    }
}
