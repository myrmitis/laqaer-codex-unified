use async_trait::async_trait;
use codex_unified_core::{
    Provider, ProviderCapabilities, ProviderError, ProviderEventStream, ProviderResolver,
};
use codex_unified_protocol::{CanonicalEvent, FailureCode, TurnEnvelope};
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProtocol {
    OpenAiResponses,
    OpenAiCompatibleResponses,
    AnthropicMessages,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiRoute {
    pub provider_id: String,
    pub model_prefix: String,
    pub upstream_base_url: String,
    pub protocol: ApiProtocol,
    pub credential_ref: String,
}

impl ApiRoute {
    pub fn matches(&self, model: &str) -> bool {
        model.starts_with(&self.model_prefix)
    }

    pub fn upstream_model<'a>(&self, model: &'a str) -> Option<&'a str> {
        model.strip_prefix(&self.model_prefix)
    }

    pub fn responses_endpoint(&self) -> String {
        format!("{}/responses", self.upstream_base_url.trim_end_matches('/'))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteResolution<'a> {
    Matched {
        route: &'a ApiRoute,
        upstream_model: &'a str,
    },
    NoMatch,
    Ambiguous,
}

#[derive(Debug, Clone, Default)]
pub struct ApiRouteTable {
    routes: Vec<ApiRoute>,
}

impl ApiRouteTable {
    pub fn new(routes: Vec<ApiRoute>) -> Self {
        Self { routes }
    }

    pub fn resolve<'a>(&'a self, model: &'a str) -> RouteResolution<'a> {
        let mut matches = self.routes.iter().filter_map(|route| {
            route
                .upstream_model(model)
                .map(|upstream| (route, upstream))
        });

        let Some((route, upstream_model)) = matches.next() else {
            return RouteResolution::NoMatch;
        };

        if matches.next().is_some() {
            return RouteResolution::Ambiguous;
        }

        RouteResolution::Matched {
            route,
            upstream_model,
        }
    }
}

/// Deterministic provider used by transport-only tests.
///
/// It never performs network traffic. Production routing should use
/// `HttpApiProviderResolver`.
#[derive(Debug, Clone)]
pub struct ApiProvider {
    pub route: ApiRoute,
}

#[async_trait]
impl Provider for ApiProvider {
    fn id(&self) -> &'static str {
        "api"
    }

    async fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        default_capabilities()
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

#[derive(Debug, Clone)]
pub struct ApiProviderResolver {
    routes: ApiRouteTable,
}

impl ApiProviderResolver {
    pub fn new(routes: ApiRouteTable) -> Self {
        Self { routes }
    }
}

impl ProviderResolver for ApiProviderResolver {
    fn resolve(&self, model: &str) -> Option<Arc<dyn Provider>> {
        match self.routes.resolve(model) {
            RouteResolution::Matched { route, .. } => Some(Arc::new(ApiProvider {
                route: route.clone(),
            })),
            RouteResolution::NoMatch | RouteResolution::Ambiguous => None,
        }
    }
}

#[async_trait]
pub trait CredentialResolver: Send + Sync {
    async fn resolve(&self, credential_ref: &str) -> Result<String, ProviderError>;
}

pub struct MissingCredentialResolver;

#[async_trait]
impl CredentialResolver for MissingCredentialResolver {
    async fn resolve(&self, _credential_ref: &str) -> Result<String, ProviderError> {
        Err(classified(
            FailureCode::AuthRequired,
            "provider credential is unavailable",
            false,
        ))
    }
}

pub struct HttpApiProvider {
    route: ApiRoute,
    upstream_model: String,
    client: reqwest::Client,
    credentials: Arc<dyn CredentialResolver>,
}

impl HttpApiProvider {
    pub fn new(
        route: ApiRoute,
        upstream_model: impl Into<String>,
        client: reqwest::Client,
        credentials: Arc<dyn CredentialResolver>,
    ) -> Self {
        Self {
            route,
            upstream_model: upstream_model.into(),
            client,
            credentials,
        }
    }

    fn upstream_body(&self, turn: &TurnEnvelope) -> Result<Value, ProviderError> {
        let mut body = turn.raw_request.clone();
        let object = body.as_object_mut().ok_or_else(|| {
            classified(
                FailureCode::InvalidProviderResponse,
                "Codex Responses request must be a JSON object",
                false,
            )
        })?;

        object.insert("model".into(), Value::String(self.upstream_model.clone()));
        object.insert("stream".into(), Value::Bool(true));

        // Native Codex metadata is preserved in TurnEnvelope but is not forwarded
        // to external API providers that do not own the local Codex protocol.
        object.remove("client_metadata");
        object.remove("type");

        Ok(body)
    }
}

#[async_trait]
impl Provider for HttpApiProvider {
    fn id(&self) -> &'static str {
        "api-http"
    }

    async fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        default_capabilities()
    }

    async fn execute(&self, turn: TurnEnvelope) -> Result<ProviderEventStream, ProviderError> {
        match self.route.protocol {
            ApiProtocol::OpenAiResponses | ApiProtocol::OpenAiCompatibleResponses => {}
            ApiProtocol::AnthropicMessages => {
                return Err(classified(
                    FailureCode::ModelUnavailable,
                    "Anthropic Messages is not implemented by the Responses HTTP adapter",
                    false,
                ));
            }
        }

        let credential = self.credentials.resolve(&self.route.credential_ref).await?;
        let body = self.upstream_body(&turn)?;

        let response = self
            .client
            .post(self.route.responses_endpoint())
            .bearer_auth(credential)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                classified(
                    FailureCode::TransportFailed,
                    format!("provider request transport failed: {error}"),
                    true,
                )
            })?;

        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }

        let stream = response
            .bytes_stream()
            .eventsource()
            .filter_map(|item| async move {
                match item {
                    Ok(event) => match decode_responses_event(&event.data) {
                        Ok(Some(event)) => Some(Ok(event)),
                        Ok(None) => None,
                        Err(error) => Some(Err(error)),
                    },
                    Err(error) => Some(Err(classified(
                        FailureCode::TransportFailed,
                        format!("provider SSE decode failed: {error}"),
                        true,
                    ))),
                }
            });

        Ok(Box::pin(stream))
    }
}

pub struct HttpApiProviderResolver {
    routes: ApiRouteTable,
    client: reqwest::Client,
    credentials: Arc<dyn CredentialResolver>,
}

impl HttpApiProviderResolver {
    pub fn new(routes: ApiRouteTable, credentials: Arc<dyn CredentialResolver>) -> Self {
        Self {
            routes,
            client: reqwest::Client::new(),
            credentials,
        }
    }

    pub fn with_client(
        routes: ApiRouteTable,
        client: reqwest::Client,
        credentials: Arc<dyn CredentialResolver>,
    ) -> Self {
        Self {
            routes,
            client,
            credentials,
        }
    }
}

impl ProviderResolver for HttpApiProviderResolver {
    fn resolve(&self, model: &str) -> Option<Arc<dyn Provider>> {
        match self.routes.resolve(model) {
            RouteResolution::Matched {
                route,
                upstream_model,
            } => Some(Arc::new(HttpApiProvider::new(
                route.clone(),
                upstream_model,
                self.client.clone(),
                Arc::clone(&self.credentials),
            ))),
            RouteResolution::NoMatch | RouteResolution::Ambiguous => None,
        }
    }
}

fn default_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        streaming: true,
        tools: true,
        images: true,
        continuation: true,
    }
}

fn classified(code: FailureCode, message: impl Into<String>, retryable: bool) -> ProviderError {
    ProviderError::Classified {
        code,
        message: message.into(),
        retryable,
    }
}

fn status_error(status: reqwest::StatusCode) -> ProviderError {
    match status.as_u16() {
        401 | 403 => classified(
            FailureCode::AuthRequired,
            format!(
                "provider authentication failed with HTTP {}",
                status.as_u16()
            ),
            false,
        ),
        429 => classified(
            FailureCode::RateLimited,
            "provider rate limit reached",
            true,
        ),
        408 | 500..=599 => classified(
            FailureCode::TransportFailed,
            format!("provider returned HTTP {}", status.as_u16()),
            true,
        ),
        _ => classified(
            FailureCode::InvalidProviderResponse,
            format!("provider returned HTTP {}", status.as_u16()),
            false,
        ),
    }
}

fn decode_responses_event(data: &str) -> Result<Option<CanonicalEvent>, ProviderError> {
    if data.trim().is_empty() || data.trim() == "[DONE]" {
        return Ok(None);
    }

    let value: Value = serde_json::from_str(data).map_err(|error| {
        classified(
            FailureCode::InvalidProviderResponse,
            format!("provider SSE data is not valid JSON: {error}"),
            false,
        )
    })?;

    let event_type = value.get("type").and_then(Value::as_str).ok_or_else(|| {
        classified(
            FailureCode::InvalidProviderResponse,
            "provider SSE event is missing type",
            false,
        )
    })?;

    match event_type {
        "response.created" => {
            let response_id = required_string(&value, &["response", "id"])?;
            Ok(Some(CanonicalEvent::ResponseCreated { response_id }))
        }
        "response.output_item.added" => {
            let item_id = required_string(&value, &["item", "id"])?;
            let item_type = required_string(&value, &["item", "type"])?;
            Ok(Some(CanonicalEvent::OutputItemAdded { item_id, item_type }))
        }
        "response.output_text.delta" => {
            let item_id = required_string(&value, &["item_id"])?;
            let delta = required_string(&value, &["delta"])?;
            Ok(Some(CanonicalEvent::TextDelta { item_id, delta }))
        }
        "response.output_item.done" => {
            let item_id = required_string(&value, &["item", "id"])?;
            let item_type = value
                .pointer("/item/type")
                .and_then(Value::as_str)
                .unwrap_or_default();

            if item_type == "function_call" {
                let call_id = required_string(&value, &["item", "call_id"])?;
                let name = required_string(&value, &["item", "name"])?;
                let arguments = value
                    .pointer("/item/arguments")
                    .cloned()
                    .unwrap_or(Value::Null);
                Ok(Some(CanonicalEvent::FunctionCall {
                    item_id,
                    call_id,
                    name,
                    arguments,
                }))
            } else {
                Ok(Some(CanonicalEvent::OutputItemDone { item_id }))
            }
        }
        "response.completed" => {
            let response_id = required_string(&value, &["response", "id"])?;
            Ok(Some(CanonicalEvent::ResponseCompleted { response_id }))
        }
        "response.failed" => {
            let response_id = optional_string_at(&value, &["response", "id"]);
            let message = optional_string_at(&value, &["response", "error", "message"])
                .unwrap_or_else(|| "provider response failed".into());
            Ok(Some(CanonicalEvent::ResponseFailed {
                response_id,
                code: FailureCode::InvalidProviderResponse,
                message,
                retryable: false,
            }))
        }
        "response.incomplete" => {
            let response_id = optional_string_at(&value, &["response", "id"]);
            let reason = optional_string_at(&value, &["response", "incomplete_details", "reason"])
                .unwrap_or_else(|| "unknown".into());
            Ok(Some(CanonicalEvent::ResponseIncomplete {
                response_id,
                reason,
            }))
        }
        _ => Ok(None),
    }
}

fn required_string(value: &Value, path: &[&str]) -> Result<String, ProviderError> {
    optional_string_at(value, path).ok_or_else(|| {
        classified(
            FailureCode::InvalidProviderResponse,
            format!("provider SSE event is missing {}", path.join(".")),
            false,
        )
    })
}

fn optional_string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::post,
    };
    use codex_unified_core::validated_event_stream;
    use futures_util::StreamExt;
    use serde_json::json;
    use std::{
        net::SocketAddr,
        sync::{Arc, Mutex},
    };
    use tokio::task::JoinHandle;

    fn openrouter(base: impl Into<String>) -> ApiRoute {
        ApiRoute {
            provider_id: "openrouter".into(),
            model_prefix: "openrouter/".into(),
            upstream_base_url: base.into(),
            protocol: ApiProtocol::OpenAiCompatibleResponses,
            credential_ref: "keychain://openrouter".into(),
        }
    }

    fn grok(base: impl Into<String>) -> ApiRoute {
        ApiRoute {
            provider_id: "xai".into(),
            model_prefix: "grok-oauth/".into(),
            upstream_base_url: base.into(),
            protocol: ApiProtocol::OpenAiResponses,
            credential_ref: "oauth://xai".into(),
        }
    }

    #[test]
    fn resolves_openrouter_without_touching_grok() {
        let table = ApiRouteTable::new(vec![
            openrouter("https://openrouter.example/v1"),
            grok("https://xai.example/v1"),
        ]);
        let RouteResolution::Matched {
            route,
            upstream_model,
        } = table.resolve("openrouter/meta/llama")
        else {
            panic!("expected one route");
        };

        assert_eq!(route.provider_id, "openrouter");
        assert_eq!(upstream_model, "meta/llama");
    }

    #[test]
    fn resolves_grok_as_independent_route() {
        let table = ApiRouteTable::new(vec![
            openrouter("https://openrouter.example/v1"),
            grok("https://xai.example/v1"),
        ]);
        let RouteResolution::Matched {
            route,
            upstream_model,
        } = table.resolve("grok-oauth/grok-4.6")
        else {
            panic!("expected one route");
        };

        assert_eq!(route.provider_id, "xai");
        assert_eq!(upstream_model, "grok-4.6");
    }

    #[test]
    fn ambiguous_prefixes_fail_closed() {
        let mut duplicate = openrouter("https://one.example/v1");
        duplicate.provider_id = "other".into();
        let table = ApiRouteTable::new(vec![openrouter("https://two.example/v1"), duplicate]);

        assert_eq!(
            table.resolve("openrouter/model"),
            RouteResolution::Ambiguous
        );
    }

    #[test]
    fn resolver_refuses_ambiguous_route() {
        let mut duplicate = openrouter("https://one.example/v1");
        duplicate.provider_id = "other".into();
        let resolver = ApiProviderResolver::new(ApiRouteTable::new(vec![
            openrouter("https://two.example/v1"),
            duplicate,
        ]));
        assert!(resolver.resolve("openrouter/model").is_none());
    }

    struct FixedCredential(&'static str);

    #[async_trait]
    impl CredentialResolver for FixedCredential {
        async fn resolve(&self, _credential_ref: &str) -> Result<String, ProviderError> {
            Ok(self.0.into())
        }
    }

    #[derive(Debug, Clone)]
    struct Capture {
        authorization: Option<String>,
        body: Value,
    }

    #[derive(Clone)]
    struct MockState {
        capture: Arc<Mutex<Option<Capture>>>,
        status: StatusCode,
        sse: Arc<str>,
    }

    async fn mock_responses(
        State(state): State<MockState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        *state.capture.lock().expect("capture mutex poisoned") = Some(Capture {
            authorization: headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            body,
        });

        if !state.status.is_success() {
            return (state.status, "mock failure").into_response();
        }

        (
            [("content-type", "text/event-stream")],
            state.sse.to_string(),
        )
            .into_response()
    }

    async fn spawn_mock(
        status: StatusCode,
        sse: &str,
    ) -> (SocketAddr, Arc<Mutex<Option<Capture>>>, JoinHandle<()>) {
        let capture = Arc::new(Mutex::new(None));
        let state = MockState {
            capture: Arc::clone(&capture),
            status,
            sse: Arc::from(sse),
        };
        let app = Router::new()
            .route("/v1/responses", post(mock_responses))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock provider");
        let address = listener.local_addr().expect("mock address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve mock provider");
        });
        (address, capture, task)
    }

    fn envelope(model: &str) -> TurnEnvelope {
        let metadata = json!({
            "thread_id": "thread-http",
            "turn_id": "turn-http",
            "request_kind": "turn"
        });
        TurnEnvelope::from_responses_request(json!({
            "model": model,
            "client_metadata": {
                "x-codex-turn-metadata": serde_json::to_string(&metadata)
                    .expect("encode metadata")
            },
            "stream": true,
            "input": []
        }))
        .expect("parse envelope")
    }

    #[tokio::test]
    async fn http_provider_rewrites_model_strips_local_metadata_and_streams_completion() {
        let sse = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-1\"}}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg-1\",\"delta\":\"OK\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-1\"}}\n\n",
        );
        let (address, capture, task) = spawn_mock(StatusCode::OK, sse).await;

        let provider = HttpApiProvider::new(
            openrouter(format!("http://{address}/v1")),
            "meta/llama",
            reqwest::Client::new(),
            Arc::new(FixedCredential("secret-test-key")),
        );
        let events: Vec<_> = provider
            .execute(envelope("openrouter/meta/llama"))
            .await
            .expect("start provider")
            .map(|item| item.expect("provider event"))
            .collect()
            .await;

        assert!(matches!(
            events.as_slice(),
            [
                CanonicalEvent::ResponseCreated { .. },
                CanonicalEvent::TextDelta { .. },
                CanonicalEvent::ResponseCompleted { .. }
            ]
        ));

        let captured = capture
            .lock()
            .expect("capture mutex poisoned")
            .clone()
            .expect("captured request");
        assert_eq!(
            captured.authorization.as_deref(),
            Some("Bearer secret-test-key")
        );
        assert_eq!(captured.body["model"], "meta/llama");
        assert_eq!(captured.body["stream"], true);
        assert!(captured.body.get("client_metadata").is_none());

        task.abort();
    }

    #[tokio::test]
    async fn provider_401_maps_to_auth_required_without_leaking_body() {
        let (address, _capture, task) = spawn_mock(StatusCode::UNAUTHORIZED, "").await;
        let provider = HttpApiProvider::new(
            openrouter(format!("http://{address}/v1")),
            "meta/llama",
            reqwest::Client::new(),
            Arc::new(FixedCredential("secret-test-key")),
        );

        let error = match provider.execute(envelope("openrouter/meta/llama")).await {
            Ok(_) => panic!("401 must fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            ProviderError::Classified {
                code: FailureCode::AuthRequired,
                retryable: false,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn abrupt_http_sse_eof_becomes_canonical_failure() {
        let sse = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-eof\"}}\n\n",
        );
        let (address, _capture, task) = spawn_mock(StatusCode::OK, sse).await;
        let provider = HttpApiProvider::new(
            grok(format!("http://{address}/v1")),
            "grok-4.6",
            reqwest::Client::new(),
            Arc::new(FixedCredential("secret-test-key")),
        );

        let source = provider
            .execute(envelope("grok-oauth/grok-4.6"))
            .await
            .expect("start provider");
        let events: Vec<_> = validated_event_stream(source)
            .map(|item| item.expect("validated event"))
            .collect()
            .await;

        assert!(matches!(
            events.last(),
            Some(CanonicalEvent::ResponseFailed {
                response_id: Some(id),
                code: FailureCode::InvalidProviderResponse,
                ..
            }) if id == "resp-eof"
        ));

        task.abort();
    }
}
