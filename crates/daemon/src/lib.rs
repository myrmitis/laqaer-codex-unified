use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use codex_unified_protocol::{TurnEnvelope, TurnEnvelopeError};
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
        .route(
            "/_codex-unified/{capability}/v1/responses",
            post(responses),
        )
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

fn turn_error_response(error: TurnEnvelopeError) -> Response {
    let code = match error {
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
            .uri(format!(
                "/_codex-unified/{path_capability}/v1/responses"
            ))
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
        assert_eq!(
            body["error"]["code"],
            "native_turn_metadata_required"
        );
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
