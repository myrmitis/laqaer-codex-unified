use codex_unified_core::Provider;
use codex_unified_provider_api::{
    ApiProtocol, ApiProvider, ApiRoute, ApiRouteTable, RouteResolution,
};
use codex_unified_protocol::TurnEnvelope;
use codex_unifiedd::render_responses_sse;
use serde_json::json;

fn route(provider_id: &str, model_prefix: &str, protocol: ApiProtocol) -> ApiRoute {
    ApiRoute {
        provider_id: provider_id.into(),
        model_prefix: model_prefix.into(),
        upstream_base_url: format!("https://{provider_id}.example/v1"),
        protocol,
        credential_ref: format!("test://{provider_id}"),
    }
}

fn codex_request(model: &str, turn_id: &str) -> serde_json::Value {
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
        "input": [{
            "type": "message",
            "role": "user",
            "content": "synthetic"
        }],
        "tools": []
    })
}

async fn exercise_route(model: &str, expected_provider: &str, expected_upstream: &str) {
    let table = ApiRouteTable::new(vec![
        route(
            "openrouter",
            "openrouter/",
            ApiProtocol::OpenAiCompatibleResponses,
        ),
        route("xai", "grok-oauth/", ApiProtocol::OpenAiResponses),
    ]);

    let envelope =
        TurnEnvelope::from_responses_request(codex_request(model, "turn-e2e"))
            .expect("parse Codex request");

    assert_eq!(envelope.identity.turn_id, "turn-e2e");
    assert_eq!(envelope.identity.thread_id.as_deref(), Some("thread-e2e"));

    let RouteResolution::Matched {
        route,
        upstream_model,
    } = table.resolve(&envelope.requested_model)
    else {
        panic!("expected exactly one provider route");
    };

    assert_eq!(route.provider_id, expected_provider);
    assert_eq!(upstream_model, expected_upstream);

    let provider = ApiProvider {
        route: route.clone(),
    };
    let events = provider.execute(envelope).await.expect("provider execution");
    let sse = render_responses_sse(&events).expect("render canonical SSE");

    assert!(sse.contains("event: response.created"));
    assert!(sse.contains("event: response.completed"));
    assert!(!sse.contains("event: response.failed"));
}

#[tokio::test]
async fn codex_to_openrouter_to_completed_sse() {
    exercise_route(
        "openrouter/meta/llama",
        "openrouter",
        "meta/llama",
    )
    .await;
}

#[tokio::test]
async fn codex_to_grok_to_completed_sse() {
    exercise_route("grok-oauth/grok-4.6", "xai", "grok-4.6").await;
}
