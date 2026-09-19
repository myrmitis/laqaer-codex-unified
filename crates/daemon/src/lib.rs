use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use codex_unified_core::{EventSequenceError, validate_event_sequence};
use codex_unified_protocol::{CanonicalEvent, FailureCode, TurnEnvelope, TurnEnvelopeError};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub capability: String,
}

#[derive(Clone)]
struct AppState {
    capability: Arc<str>,
}

pub fn app(config: AppConfig) -> Router {
    let state = AppState {
        capability: Arc::<str>::from(config.capability),
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

    tracing::info!(
        trace_id = %turn.trace_id,
        model = %turn.requested_model,
        "responses ingress accepted"
    );

    error_response(
        StatusCode::NOT_IMPLEMENTED,
        "provider_not_implemented",
        "Codex Unified ingress is active, but provider execution is not implemented in this foundation slice.",
    )
}

pub fn render_responses_sse(events: &[CanonicalEvent]) -> Result<String, EventSequenceError> {
    validate_event_sequence(events)?;

    let mut output = String::new();
    for event in events {
        let name = event_name(event);
        let payload = event_payload(event);
        output.push_str("event: ");
        output.push_str(name);
        output.push('\n');
        output.push_str("data: ");
        output.push_str(&payload.to_string());
        output.push_str("\n\n");
    }
    Ok(output)
}

fn event_name(event: &CanonicalEvent) -> &'static str {
    match event {
        CanonicalEvent::ResponseCreated { .. } => "response.created",
        CanonicalEvent::OutputItemAdded { .. } => "response.output_item.added",
        CanonicalEvent::TextDelta { .. } => "response.output_text.delta",
        CanonicalEvent::FunctionCall { .. } => "response.function_call_arguments.done",
        CanonicalEvent::OutputItemDone { .. } => "response.output_item.done",
        CanonicalEvent::ResponseCompleted { .. } => "response.completed",
        CanonicalEvent::ResponseFailed { .. } => "response.failed",
        CanonicalEvent::ResponseIncomplete { .. } => "response.incomplete",
    }
}

fn event_payload(event: &CanonicalEvent) -> Value {
    match event {
        CanonicalEvent::ResponseCreated { response_id } => json!({
            "type": "response.created",
            "response": {"id": response_id, "status": "in_progress"}
        }),
        CanonicalEvent::OutputItemAdded { item_id, item_type } => json!({
            "type": "response.output_item.added",
            "item": {"id": item_id, "type": item_type}
        }),
        CanonicalEvent::TextDelta { item_id, delta } => json!({
            "type": "response.output_text.delta",
            "item_id": item_id,
            "delta": delta
        }),
        CanonicalEvent::FunctionCall {
            item_id,
            call_id,
            name,
            arguments,
        } => json!({
            "type": "response.function_call_arguments.done",
            "item_id": item_id,
            "call_id": call_id,
            "name": name,
            "arguments": arguments
        }),
        CanonicalEvent::OutputItemDone { item_id } => json!({
            "type": "response.output_item.done",
            "item": {"id": item_id}
        }),
        CanonicalEvent::ResponseCompleted { response_id } => json!({
            "type": "response.completed",
            "response": {"id": response_id, "status": "completed"}
        }),
        CanonicalEvent::ResponseFailed {
            response_id,
            code,
            message,
            retryable,
        } => json!({
            "type": "response.failed",
            "response": {
                "id": response_id,
                "status": "failed",
                "error": {
                    "code": failure_code(code),
                    "message": message,
                    "retryable": retryable
                }
            }
        }),
        CanonicalEvent::ResponseIncomplete {
            response_id,
            reason,
        } => json!({
            "type": "response.incomplete",
            "response": {
                "id": response_id,
                "status": "incomplete",
                "incomplete_details": {"reason": reason}
            }
        }),
    }
}

fn failure_code(code: &FailureCode) -> &'static str {
    match code {
        FailureCode::AuthRequired => "auth_required",
        FailureCode::RateLimited => "rate_limited",
        FailureCode::QuotaExhausted => "quota_exhausted",
        FailureCode::SecurityChallenge => "security_challenge",
        FailureCode::ModelUnavailable => "model_unavailable",
        FailureCode::DomContractChanged => "dom_contract_changed",
        FailureCode::ProviderTimeout => "provider_timeout",
        FailureCode::TransportFailed => "transport_failed",
        FailureCode::ContinuationMissing => "continuation_missing",
        FailureCode::InvalidProviderResponse => "invalid_provider_response",
        FailureCode::Cancelled => "cancelled",
    }
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
    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
    };
    use tower::ServiceExt;

    const CAPABILITY: &str = "test-capability-0123456789";

    fn test_app() -> Router {
        app(AppConfig {
            capability: CAPABILITY.to_owned(),
        })
    }

    async fn post(path_capability: &str, payload: Value) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("POST")
            .uri(format!("/_codex-unified/{path_capability}/v1/responses"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&payload).expect("serialize fixture"),
            ))
            .expect("request fixture");

        let response = test_app().oneshot(request).await.expect("router response");
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response");
        let value = serde_json::from_slice(&body).expect("JSON response");
        (status, value)
    }

    #[test]
    fn renderer_refuses_eof_without_terminal_event() {
        let result = render_responses_sse(&[CanonicalEvent::ResponseCreated {
            response_id: "r1".into(),
        }]);
        assert_eq!(result, Err(EventSequenceError::MissingTerminal));
    }

    #[test]
    fn renderer_emits_completion_only_from_explicit_completion() {
        let body = render_responses_sse(&[
            CanonicalEvent::ResponseCreated {
                response_id: "r1".into(),
            },
            CanonicalEvent::TextDelta {
                item_id: "i1".into(),
                delta: "OK".into(),
            },
            CanonicalEvent::ResponseCompleted {
                response_id: "r1".into(),
            },
        ])
        .expect("valid event sequence");

        assert!(body.contains("event: response.created"));
        assert!(body.contains("event: response.output_text.delta"));
        assert!(body.contains("event: response.completed"));
        assert!(!body.contains("response.failed"));
    }

    #[test]
    fn renderer_preserves_typed_failure_terminal() {
        let body = render_responses_sse(&[
            CanonicalEvent::ResponseCreated {
                response_id: "r1".into(),
            },
            CanonicalEvent::ResponseFailed {
                response_id: Some("r1".into()),
                code: FailureCode::TransportFailed,
                message: "stream closed before terminal".into(),
                retryable: false,
            },
        ])
        .expect("valid failure sequence");

        assert!(body.contains("event: response.failed"));
        assert!(body.contains("\"code\":\"transport_failed\""));
        assert!(!body.contains("event: response.completed"));
    }

    #[tokio::test]
    async fn rejects_wrong_local_capability() {
        let (status, body) = post(
            "wrong-capability",
            serde_json::json!({"model": "api/model"}),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "local_auth_required");
    }

    #[tokio::test]
    async fn rejects_missing_native_turn_identity() {
        let (status, body) = post(
            CAPABILITY,
            serde_json::json!({
                "model": "chatgpt-web/instant",
                "input": []
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "native_turn_metadata_required");
    }

    #[tokio::test]
    async fn accepts_codex_shaped_request_without_claiming_provider_success() {
        let metadata = serde_json::json!({
            "thread_id": "thread-1",
            "turn_id": "turn-1",
            "request_kind": "turn"
        });
        let (status, body) = post(
            CAPABILITY,
            serde_json::json!({
                "model": "chatgpt-web/pro",
                "client_metadata": {
                    "x-codex-turn-metadata": serde_json::to_string(&metadata)
                        .expect("encode metadata")
                },
                "input": []
            }),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["error"]["code"], "provider_not_implemented");
    }
}
