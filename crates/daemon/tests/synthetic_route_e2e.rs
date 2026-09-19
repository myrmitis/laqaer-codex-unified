use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use codex_unified_provider_api::{ApiProtocol, ApiProviderResolver, ApiRoute, ApiRouteTable};
use codex_unifiedd::{AppConfig, app};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

const CAPABILITY: &str = "test-capability-e2e-0123456789";

fn route(provider_id: &str, model_prefix: &str, protocol: ApiProtocol) -> ApiRoute {
    ApiRoute {
        provider_id: provider_id.into(),
        model_prefix: model_prefix.into(),
        upstream_base_url: format!("https://{provider_id}.example/v1"),
        protocol,
        credential_ref: format!("test://{provider_id}"),
    }
}

fn test_app() -> axum::Router {
    let routes = ApiRouteTable::new(vec![
        route(
            "openrouter",
            "openrouter/",
            ApiProtocol::OpenAiCompatibleResponses,
        ),
        route("xai", "grok-oauth/", ApiProtocol::OpenAiResponses),
    ]);

    app(AppConfig {
        capability: CAPABILITY.into(),
        providers: Arc::new(ApiProviderResolver::new(routes)),
    })
}

fn request_payload(model: &str, turn_id: &str) -> Value {
    let metadata = json!({
        "thread_id": "thread-e2e",
        "turn_id": turn_id,
        "request_kind": "turn"
    });

    json!({
        "model": model,
        "client_metadata": {
            "x-codex-turn-metadata": serde_json::to_string(&metadata)
                .expect("encode metadata")
        },
        "stream": true,
        "input": [{
            "type": "message",
            "role": "user",
            "content": "synthetic"
        }],
        "tools": []
    })
}

async fn exercise(model: &str, turn_id: &str) {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/_codex-unified/{CAPABILITY}/v1/responses"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&request_payload(model, turn_id)).expect("serialize request"),
        ))
        .expect("build request");

    let response = test_app().oneshot(request).await.expect("router response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
    );

    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read SSE body");
    let text = String::from_utf8(body.to_vec()).expect("UTF-8 SSE");

    assert!(text.contains("event: response.created"));
    assert!(text.contains("event: response.completed"));
    assert!(!text.contains("event: response.failed"));
    assert!(text.contains(turn_id));
}

#[tokio::test]
async fn codex_to_openrouter_route_to_sse() {
    exercise("openrouter/meta/llama", "turn-openrouter").await;
}

#[tokio::test]
async fn codex_to_grok_route_to_sse() {
    exercise("grok-oauth/grok-4.6", "turn-grok").await;
}
