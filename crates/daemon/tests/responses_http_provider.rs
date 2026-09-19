use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::post,
};
use codex_unified_core::{Provider, StaticProviderResolver};
use codex_unified_provider_api::{
    CredentialSource, ResponsesHttpProvider, ResponsesHttpRoute,
};
use codex_unifiedd::{AppConfig, app};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use zeroize::Zeroizing;

const CAPABILITY: &str = "integration-capability-0123456789";

struct StaticCredential;

impl CredentialSource for StaticCredential {
    fn load(
        &self,
    ) -> Result<Zeroizing<String>, codex_unified_core::ProviderError> {
        Ok(Zeroizing::new("integration-secret".to_owned()))
    }
}

#[derive(Default)]
struct Capture {
    authorization: Mutex<Option<String>>,
    payload: Mutex<Option<Value>>,
}

async fn upstream(
    State(capture): State<Arc<Capture>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    *capture.authorization.lock().expect("authorization lock") = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    *capture.payload.lock().expect("payload lock") = Some(payload);

    let body = [
        "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-integration\"}}\n\n",
        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg-1\",\"delta\":\"UNIFIED OK\"}\n\n",
        "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-integration\",\"status\":\"completed\"}}\n\n",
    ]
    .concat();

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        body,
    )
}

async fn mock_upstream() -> (String, Arc<Capture>, tokio::task::JoinHandle<()>) {
    let capture = Arc::new(Capture::default());
    let router = Router::new()
        .route("/v1/responses", post(upstream))
        .with_state(Arc::clone(&capture));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind upstream fixture");
    let address = listener.local_addr().expect("fixture address");
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve upstream fixture");
    });
    (
        format!("http://{address}/v1/responses"),
        capture,
        task,
    )
}

#[tokio::test]
async fn codex_request_routes_through_openrouter_responses_contract() {
    let (endpoint, capture, upstream_task) = mock_upstream().await;

    let route = ResponsesHttpRoute::openrouter_with_endpoint(
        "openrouter/grok-4.6",
        "x-ai/grok-4.6",
        endpoint.parse().expect("fixture URL"),
        Arc::new(StaticCredential),
    );
    let provider: Arc<dyn Provider> = Arc::new(ResponsesHttpProvider::new(route));
    let resolver = StaticProviderResolver::new()
        .with_route("openrouter/grok-4.6", provider);

    let router = app(AppConfig {
        capability: CAPABILITY.to_owned(),
        providers: Arc::new(resolver),
    });

    let turn_metadata = json!({
        "thread_id": "thread-integration",
        "turn_id": "turn-integration",
        "request_kind": "turn"
    });
    let request_body = json!({
        "model": "openrouter/grok-4.6",
        "stream": true,
        "client_metadata": {
            "x-codex-turn-metadata": serde_json::to_string(&turn_metadata)
                .expect("encode turn metadata")
        },
        "previous_response_id": "resp-prior",
        "input": [{"role": "user", "content": "reply exactly"}]
    });

    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!(
            "/_codex-unified/{CAPABILITY}/v1/responses"
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(
            serde_json::to_vec(&request_body).expect("request body"),
        ))
        .expect("Codex request");

    let response = router.oneshot(request).await.expect("daemon response");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read streamed response");
    let body = String::from_utf8(bytes.to_vec()).expect("UTF-8 SSE");
    assert!(body.contains("event: response.created"));
    assert!(body.contains("UNIFIED OK"));
    assert!(body.contains("event: response.completed"));

    assert_eq!(
        capture
            .authorization
            .lock()
            .expect("authorization lock")
            .as_deref(),
        Some("Bearer integration-secret")
    );

    let payload_guard = capture.payload.lock().expect("payload lock");
    let payload = payload_guard.as_ref().expect("captured upstream payload");
    assert_eq!(payload["model"], "x-ai/grok-4.6");
    assert_eq!(payload["previous_response_id"], "resp-prior");
    assert!(payload.get("client_metadata").is_none());

    upstream_task.abort();
}
