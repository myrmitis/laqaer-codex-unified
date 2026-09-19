use async_trait::async_trait;
use codex_unified_core::{Provider, ProviderCapabilities, ProviderError, ProviderEventStream};
use codex_unified_protocol::{CanonicalEvent, FailureCode, TurnEnvelope};
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use zeroize::Zeroizing;

const OPENROUTER_PROVIDER_ID: &str = "openrouter";
const GROK_API_PROVIDER_ID: &str = "grok-api";
const OPENROUTER_RESPONSES_URL: &str = "https://openrouter.ai/api/v1/responses";
const XAI_RESPONSES_URL: &str = "https://api.x.ai/v1/responses";

pub trait CredentialSource: Send + Sync {
    fn load(&self) -> Result<Zeroizing<String>, ProviderError>;
}

pub struct EnvironmentCredential {
    variable: &'static str,
}

impl EnvironmentCredential {
    pub const fn new(variable: &'static str) -> Self {
        Self { variable }
    }
}

impl CredentialSource for EnvironmentCredential {
    fn load(&self) -> Result<Zeroizing<String>, ProviderError> {
        let token = std::env::var(self.variable)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                classified(
                    FailureCode::AuthRequired,
                    format!("{} is not configured", self.variable),
                    false,
                )
            })?;
        Ok(Zeroizing::new(token))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSchemaPolicy {
    Preserve,
    BreakRecursiveLocalRefs,
}

#[derive(Clone)]
pub struct ResponsesHttpRoute {
    provider_id: &'static str,
    public_model: String,
    upstream_model: String,
    endpoint: Url,
    credential: Arc<dyn CredentialSource>,
    tool_schema_policy: ToolSchemaPolicy,
    preserve_previous_response_id: bool,
}

impl ResponsesHttpRoute {
    pub fn openrouter(
        public_model: impl Into<String>,
        upstream_model: impl Into<String>,
        credential: Arc<dyn CredentialSource>,
    ) -> Self {
        Self::openrouter_with_endpoint(
            public_model,
            upstream_model,
            Url::parse(OPENROUTER_RESPONSES_URL).expect("static OpenRouter URL"),
            credential,
        )
    }

    pub fn xai_api(
        public_model: impl Into<String>,
        upstream_model: impl Into<String>,
        credential: Arc<dyn CredentialSource>,
    ) -> Self {
        Self::xai_api_with_endpoint(
            public_model,
            upstream_model,
            Url::parse(XAI_RESPONSES_URL).expect("static xAI URL"),
            credential,
        )
    }

    pub fn public_model(&self) -> &str {
        &self.public_model
    }

    pub fn openrouter_with_endpoint(
        public_model: impl Into<String>,
        upstream_model: impl Into<String>,
        endpoint: Url,
        credential: Arc<dyn CredentialSource>,
    ) -> Self {
        let upstream_model = upstream_model.into();
        let tool_schema_policy = if upstream_model.starts_with("meta/") {
            ToolSchemaPolicy::BreakRecursiveLocalRefs
        } else {
            ToolSchemaPolicy::Preserve
        };
        Self {
            provider_id: OPENROUTER_PROVIDER_ID,
            public_model: public_model.into(),
            upstream_model,
            endpoint,
            credential,
            tool_schema_policy,
            preserve_previous_response_id: true,
        }
    }

    pub fn xai_api_with_endpoint(
        public_model: impl Into<String>,
        upstream_model: impl Into<String>,
        endpoint: Url,
        credential: Arc<dyn CredentialSource>,
    ) -> Self {
        Self {
            provider_id: GROK_API_PROVIDER_ID,
            public_model: public_model.into(),
            upstream_model: upstream_model.into(),
            endpoint,
            credential,
            tool_schema_policy: ToolSchemaPolicy::Preserve,
            preserve_previous_response_id: true,
        }
    }
}

pub struct ResponsesHttpProvider {
    route: ResponsesHttpRoute,
    client: Client,
}

impl ResponsesHttpProvider {
    pub fn new(route: ResponsesHttpRoute) -> Self {
        Self {
            route,
            client: Client::new(),
        }
    }

    pub fn route(&self) -> &ResponsesHttpRoute {
        &self.route
    }

    fn request_payload(&self, turn: &TurnEnvelope) -> Result<Value, ProviderError> {
        let mut payload = turn.raw_request.as_object().cloned().ok_or_else(|| {
            classified(
                FailureCode::InvalidProviderResponse,
                "Codex request body is not a JSON object",
                false,
            )
        })?;

        payload.insert(
            "model".into(),
            Value::String(self.route.upstream_model.clone()),
        );
        payload.insert("stream".into(), Value::Bool(true));

        // These are Codex edge semantics, not upstream provider inputs.
        payload.remove("client_metadata");
        payload.remove("type");

        if !self.route.preserve_previous_response_id {
            payload.remove("previous_response_id");
        }

        if self.route.tool_schema_policy == ToolSchemaPolicy::BreakRecursiveLocalRefs {
            break_recursive_tool_schemas(&mut payload);
        }

        Ok(Value::Object(payload))
    }
}

#[async_trait]
impl Provider for ResponsesHttpProvider {
    fn id(&self) -> &'static str {
        self.route.provider_id
    }

    async fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tools: true,
            images: true,
            continuation: self.route.preserve_previous_response_id,
        }
    }

    async fn execute(&self, turn: TurnEnvelope) -> Result<ProviderEventStream, ProviderError> {
        let payload = self.request_payload(&turn)?;
        let token = self.route.credential.load()?;

        let response = self
            .client
            .post(self.route.endpoint.clone())
            .bearer_auth(token.as_str())
            .json(&payload)
            .send()
            .await
            .map_err(|_| {
                classified(
                    FailureCode::TransportFailed,
                    format!("{} request transport failed", self.route.provider_id),
                    true,
                )
            })?;

        if !response.status().is_success() {
            return Err(classify_http_status(
                self.route.provider_id,
                response.status(),
            ));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type.starts_with("text/event-stream") {
            return Err(classified(
                FailureCode::InvalidProviderResponse,
                format!(
                    "{} returned a non-streaming Responses payload",
                    self.route.provider_id
                ),
                false,
            ));
        }

        let provider_id = self.route.provider_id;
        let mut upstream = response.bytes_stream();
        let output = async_stream::stream! {
            let mut buffer = Vec::<u8>::new();

            while let Some(chunk) = upstream.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(_) => {
                        yield Err(classified(
                            FailureCode::TransportFailed,
                            format!("{provider_id} response stream failed"),
                            true,
                        ));
                        return;
                    }
                };
                buffer.extend_from_slice(&chunk);

                while let Some(block) = take_sse_block(&mut buffer) {
                    match decode_sse_block(&block) {
                        Ok(Some(event)) => yield Ok(event),
                        Ok(None) => {}
                        Err(error) => {
                            yield Err(error);
                            return;
                        }
                    }
                }
            }

            if buffer.iter().any(|byte| !byte.is_ascii_whitespace()) {
                match decode_sse_block(&buffer) {
                    Ok(Some(event)) => yield Ok(event),
                    Ok(None) => {}
                    Err(error) => yield Err(error),
                }
            }
        };

        Ok(Box::pin(output))
    }
}

fn classified(code: FailureCode, message: impl Into<String>, retryable: bool) -> ProviderError {
    ProviderError::Classified {
        code,
        message: message.into(),
        retryable,
    }
}

fn classify_http_status(provider: &str, status: StatusCode) -> ProviderError {
    let (code, retryable) = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => (FailureCode::AuthRequired, false),
        StatusCode::PAYMENT_REQUIRED => (FailureCode::QuotaExhausted, false),
        StatusCode::NOT_FOUND => (FailureCode::ModelUnavailable, false),
        StatusCode::TOO_MANY_REQUESTS => (FailureCode::RateLimited, true),
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => (FailureCode::TransportFailed, true),
        _ if status.is_server_error() => (FailureCode::TransportFailed, true),
        _ => (FailureCode::InvalidProviderResponse, false),
    };
    classified(
        code,
        format!("{provider} returned HTTP {}", status.as_u16()),
        retryable,
    )
}

fn take_sse_block(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let mut index = 0;
    while index + 1 < buffer.len() {
        if buffer[index] == b'\n' && buffer[index + 1] == b'\n' {
            return Some(buffer.drain(..index + 2).collect());
        }
        if index + 3 < buffer.len() && buffer[index..index + 4] == *b"\r\n\r\n" {
            return Some(buffer.drain(..index + 4).collect());
        }
        index += 1;
    }
    None
}

fn decode_sse_block(block: &[u8]) -> Result<Option<CanonicalEvent>, ProviderError> {
    let text = std::str::from_utf8(block).map_err(|_| {
        classified(
            FailureCode::InvalidProviderResponse,
            "provider returned non-UTF-8 SSE",
            false,
        )
    })?;

    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
        .collect::<Vec<_>>()
        .join("\n");

    if data.is_empty() || data == "[DONE]" {
        return Ok(None);
    }

    let value = serde_json::from_str::<Value>(&data).map_err(|_| {
        classified(
            FailureCode::InvalidProviderResponse,
            "provider returned malformed Responses SSE JSON",
            false,
        )
    })?;

    parse_wire_event(&value)
}

fn parse_wire_event(value: &Value) -> Result<Option<CanonicalEvent>, ProviderError> {
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let response = value.get("response");

    let event = match event_type {
        "response.created" => CanonicalEvent::ResponseCreated {
            response_id: required_response_id(response)?,
        },
        "response.output_item.added" => {
            let item = value
                .get("item")
                .ok_or_else(|| invalid_wire("missing output item"))?;
            CanonicalEvent::OutputItemAdded {
                item_id: required_string(item, "id")?,
                item_type: required_string(item, "type")?,
            }
        }
        "response.output_text.delta" => CanonicalEvent::TextDelta {
            item_id: required_string(value, "item_id")?,
            delta: required_string(value, "delta")?,
        },
        "response.output_item.done" => {
            let item = value.get("item").ok_or_else(|| invalid_wire("missing output item"))?;
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                let raw_arguments = item.get("arguments").cloned().unwrap_or(Value::Null);
                let arguments = match raw_arguments {
                    Value::String(text) => {
                        serde_json::from_str(&text).unwrap_or(Value::String(text))
                    }
                    other => other,
                };
                CanonicalEvent::FunctionCall {
                    item_id: required_string(item, "id")?,
                    call_id: required_string(item, "call_id")?,
                    name: required_string(item, "name")?,
                    arguments,
                }
            } else {
                CanonicalEvent::OutputItemDone {
                    item_id: required_string(item, "id")?,
                }
            }
        }
        "response.completed" => CanonicalEvent::ResponseCompleted {
            response_id: required_response_id(response)?,
        },
        "response.failed" => CanonicalEvent::ResponseFailed {
            response_id: optional_response_id(response),
            code: FailureCode::InvalidProviderResponse,
            message: bounded_wire_message(
                response
                    .and_then(|item| item.get("error"))
                    .and_then(|item| item.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("provider reported response.failed"),
            ),
            retryable: false,
        },
        "response.incomplete" => CanonicalEvent::ResponseIncomplete {
            response_id: optional_response_id(response),
            reason: bounded_wire_message(
                response
                    .and_then(|item| item.get("incomplete_details"))
                    .and_then(|item| item.get("reason"))
                    .and_then(Value::as_str)
                    .unwrap_or("provider reported response.incomplete"),
            ),
        },
        "error" => {
            return Err(classified(
                FailureCode::InvalidProviderResponse,
                bounded_wire_message(
                    value
                        .get("error")
                        .and_then(|item| item.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("provider emitted an error event"),
                ),
                false,
            ));
        }
        _ => return Ok(None),
    };

    Ok(Some(event))
}

fn required_response_id(response: Option<&Value>) -> Result<String, ProviderError> {
    response
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_wire("Responses event is missing response.id"))
}

fn optional_response_id(response: Option<&Value>) -> Option<String> {
    response
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn required_string(value: &Value, key: &str) -> Result<String, ProviderError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_wire(format!("Responses event is missing {key}")))
}

fn invalid_wire(message: impl Into<String>) -> ProviderError {
    classified(FailureCode::InvalidProviderResponse, message, false)
}

fn bounded_wire_message(message: &str) -> String {
    message
        .chars()
        .filter(|ch| *ch != '\r' && *ch != '\n')
        .take(512)
        .collect()
}

fn break_recursive_tool_schemas(payload: &mut Map<String, Value>) {
    let Some(tools) = payload.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };

    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        let Some(parameters) = tool.get_mut("parameters") else {
            continue;
        };
        *parameters = non_recursive_schema(parameters);
    }
}

fn non_recursive_schema(schema: &Value) -> Value {
    let Some(_) = schema.as_object() else {
        return schema.clone();
    };

    let mut state = HashMap::<String, u8>::new();
    let mut closing = HashSet::<String>::new();
    find_closing_refs(schema, "", &mut state, &mut closing);

    if closing.is_empty() {
        return schema.clone();
    }

    let mut repaired = schema.clone();
    for path in closing {
        let Some(node) = repaired.pointer_mut(&path).and_then(Value::as_object_mut) else {
            continue;
        };
        let reference = node
            .get("$ref")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        node.remove("$ref");

        if !node.contains_key("type") {
            if let Some(reference) = reference {
                if let Some(kind) = resolve_local_ref(schema, &reference)
                    .and_then(|target| target.get("type"))
                    .cloned()
                {
                    node.insert("type".into(), kind);
                }
            }
        }
    }

    repaired
}

fn find_closing_refs(
    root: &Value,
    path: &str,
    state: &mut HashMap<String, u8>,
    closing: &mut HashSet<String>,
) {
    if state.get(path) == Some(&2) {
        return;
    }
    state.insert(path.to_owned(), 1);

    for (target, is_reference) in schema_edges(root, path) {
        match state.get(&target).copied().unwrap_or(0) {
            1 if is_reference => {
                closing.insert(path.to_owned());
            }
            0 => find_closing_refs(root, &target, state, closing),
            _ => {}
        }
    }

    state.insert(path.to_owned(), 2);
}

fn schema_edges(root: &Value, path: &str) -> Vec<(String, bool)> {
    let Some(node) = pointer_value(root, path).and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut edges = Vec::new();

    if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        if let Some(target) = local_ref_pointer(reference) {
            if root.pointer(&target).is_some() {
                edges.push((target, true));
            }
        }
    }

    for keyword in [
        "$defs",
        "definitions",
        "properties",
        "patternProperties",
        "dependentSchemas",
    ] {
        if let Some(children) = node.get(keyword).and_then(Value::as_object) {
            for name in children.keys() {
                edges.push((
                    format!(
                        "{path}/{}/{}",
                        escape_pointer(keyword),
                        escape_pointer(name)
                    ),
                    false,
                ));
            }
        }
    }

    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(children) = node.get(keyword).and_then(Value::as_array) {
            for index in 0..children.len() {
                edges.push((format!("{path}/{keyword}/{index}"), false));
            }
        }
    }

    for keyword in [
        "additionalProperties",
        "contains",
        "contentSchema",
        "else",
        "if",
        "items",
        "not",
        "propertyNames",
        "then",
        "unevaluatedItems",
        "unevaluatedProperties",
    ] {
        if node.get(keyword).is_some_and(Value::is_object) {
            edges.push((format!("{path}/{keyword}"), false));
        }
    }

    edges
}

fn pointer_value<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        Some(root)
    } else {
        root.pointer(path)
    }
}

fn local_ref_pointer(reference: &str) -> Option<String> {
    reference.strip_prefix('#').map(ToOwned::to_owned)
}

fn resolve_local_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    local_ref_pointer(reference)
        .as_deref()
        .and_then(|pointer| pointer_value(root, pointer))
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode as AxumStatusCode, header},
        response::IntoResponse,
        routing::post,
    };
    use codex_unified_core::validated_event_stream;
    use futures_util::StreamExt;
    use std::sync::Mutex;
    use tokio::task::JoinHandle;

    struct StaticCredential(&'static str);

    impl CredentialSource for StaticCredential {
        fn load(&self) -> Result<Zeroizing<String>, ProviderError> {
            Ok(Zeroizing::new(self.0.to_owned()))
        }
    }

    struct Capture {
        requests: Mutex<Vec<(String, Value)>>,
        status: Mutex<AxumStatusCode>,
    }

    async fn upstream(
        State(capture): State<Arc<Capture>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        capture
            .requests
            .lock()
            .expect("capture lock")
            .push((authorization, body));

        let status = *capture.status.lock().expect("status lock");
        if status != AxumStatusCode::OK {
            return (
                status,
                [(header::CONTENT_TYPE, "application/json")],
                "{}".to_owned(),
            );
        }

        let body = [
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-test\"}}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg-1\",\"delta\":\"OK\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-test\",\"status\":\"completed\"}}\n\n",
        ]
        .concat();

        (
            AxumStatusCode::OK,
            [(header::CONTENT_TYPE, "text/event-stream")],
            body,
        )
    }

    async fn mock_upstream() -> (Url, Arc<Capture>, JoinHandle<()>) {
        let capture = Arc::new(Capture {
            requests: Mutex::new(Vec::new()),
            status: Mutex::new(AxumStatusCode::OK),
        });
        let app = Router::new()
            .route("/v1/responses", post(upstream))
            .with_state(Arc::clone(&capture));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock");
        let address = listener.local_addr().expect("mock address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock");
        });
        (
            Url::parse(&format!("http://{address}/v1/responses")).expect("mock URL"),
            capture,
            task,
        )
    }

    fn turn(public_model: &str, tools: Value) -> TurnEnvelope {
        let raw_request = json!({
            "type": "response.create",
            "model": public_model,
            "stream": true,
            "client_metadata": {
                "x-codex-turn-metadata": "{\"turn_id\":\"turn-api\",\"request_kind\":\"turn\"}"
            },
            "previous_response_id": "resp-prior",
            "prompt_cache_key": "cache-key",
            "input": [{"role": "user", "content": "hello"}],
            "tools": tools
        });
        TurnEnvelope::from_responses_request(raw_request).expect("turn fixture")
    }

    fn recursive_tool() -> Value {
        json!([{
            "type": "function",
            "name": "walk",
            "description": "walk a node",
            "parameters": {
                "type": "object",
                "$defs": {
                    "node": {
                        "type": "object",
                        "properties": {
                            "child": {"$ref": "#/$defs/node"}
                        }
                    }
                },
                "properties": {
                    "root": {"$ref": "#/$defs/node"}
                }
            }
        }])
    }

    async fn execute_and_collect(
        provider: &ResponsesHttpProvider,
        turn: TurnEnvelope,
    ) -> Vec<CanonicalEvent> {
        let stream = provider.execute(turn).await.expect("execute provider");
        validated_event_stream(stream)
            .map(|item| item.expect("validated event"))
            .collect()
            .await
    }

    #[tokio::test]
    async fn openrouter_preserves_continuation_and_strips_codex_metadata() {
        let (endpoint, capture, task) = mock_upstream().await;
        let route = ResponsesHttpRoute::openrouter_with_endpoint(
            "openrouter/grok-4.6",
            "x-ai/grok-4.6",
            endpoint,
            Arc::new(StaticCredential("test-secret")),
        );
        assert_eq!(route.tool_schema_policy, ToolSchemaPolicy::Preserve);
        let provider = ResponsesHttpProvider::new(route);

        let events =
            execute_and_collect(&provider, turn("openrouter/grok-4.6", recursive_tool())).await;
        assert!(matches!(
            events.last(),
            Some(CanonicalEvent::ResponseCompleted { .. })
        ));

        let requests = capture.requests.lock().expect("capture lock");
        let (authorization, payload) = requests.last().expect("captured request");
        assert_eq!(authorization, "Bearer test-secret");
        assert_eq!(payload["model"], "x-ai/grok-4.6");
        assert_eq!(payload["previous_response_id"], "resp-prior");
        assert_eq!(payload["prompt_cache_key"], "cache-key");
        assert_eq!(payload["stream"], true);
        assert!(payload.get("client_metadata").is_none());
        assert!(payload.get("type").is_none());
        assert_eq!(
            payload["tools"][0]["parameters"]["$defs"]["node"]["properties"]["child"]["$ref"],
            "#/$defs/node"
        );

        task.abort();
    }

    #[tokio::test]
    async fn openrouter_meta_route_breaks_only_cycle_closing_ref() {
        let (endpoint, capture, task) = mock_upstream().await;
        let route = ResponsesHttpRoute::openrouter_with_endpoint(
            "openrouter/muse-spark-1.2",
            "meta/muse-spark-1.2",
            endpoint,
            Arc::new(StaticCredential("test-secret")),
        );
        assert_eq!(
            route.tool_schema_policy,
            ToolSchemaPolicy::BreakRecursiveLocalRefs
        );
        let provider = ResponsesHttpProvider::new(route);

        execute_and_collect(
            &provider,
            turn("openrouter/muse-spark-1.2", recursive_tool()),
        )
        .await;

        let requests = capture.requests.lock().expect("capture lock");
        let payload = &requests.last().expect("captured request").1;
        let parameters = &payload["tools"][0]["parameters"];
        assert_eq!(parameters["properties"]["root"]["$ref"], "#/$defs/node");
        let child = &parameters["$defs"]["node"]["properties"]["child"];
        assert!(child.get("$ref").is_none());
        assert_eq!(child["type"], "object");

        task.abort();
    }

    #[tokio::test]
    async fn xai_api_keeps_native_responses_continuation_without_meta_rewrite() {
        let (endpoint, capture, task) = mock_upstream().await;
        let route = ResponsesHttpRoute::xai_api_with_endpoint(
            "grok-api/grok-4.6",
            "grok-4.6",
            endpoint,
            Arc::new(StaticCredential("xai-secret")),
        );
        assert_eq!(route.tool_schema_policy, ToolSchemaPolicy::Preserve);
        let provider = ResponsesHttpProvider::new(route);

        execute_and_collect(&provider, turn("grok-api/grok-4.6", recursive_tool())).await;

        let requests = capture.requests.lock().expect("capture lock");
        let (authorization, payload) = requests.last().expect("captured request");
        assert_eq!(authorization, "Bearer xai-secret");
        assert_eq!(payload["model"], "grok-4.6");
        assert_eq!(payload["previous_response_id"], "resp-prior");
        assert_eq!(
            payload["tools"][0]["parameters"]["$defs"]["node"]["properties"]["child"]["$ref"],
            "#/$defs/node"
        );

        task.abort();
    }

    #[tokio::test]
    async fn upstream_rate_limit_is_classified_retryable() {
        let (endpoint, capture, task) = mock_upstream().await;
        *capture.status.lock().expect("status lock") = AxumStatusCode::TOO_MANY_REQUESTS;
        let route = ResponsesHttpRoute::xai_api_with_endpoint(
            "grok-api/grok-4.6",
            "grok-4.6",
            endpoint,
            Arc::new(StaticCredential("xai-secret")),
        );
        let provider = ResponsesHttpProvider::new(route);

        let error = match provider.execute(turn("grok-api/grok-4.6", json!([]))).await {
            Ok(_) => panic!("rate limit must fail before stream"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ProviderError::Classified {
                code: FailureCode::RateLimited,
                retryable: true,
                ..
            }
        ));

        task.abort();
    }
}
