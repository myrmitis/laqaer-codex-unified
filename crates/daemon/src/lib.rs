use axum::{
    Json, Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use codex_unified_core::{EmptyProviderResolver, ProviderResolver, validated_event_stream};
use codex_unified_protocol::{CanonicalEvent, TurnEnvelope, TurnEnvelopeError};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc};

pub const RESPONSES_WEBSOCKET_BETA: &str = "responses_websockets=2026-02-06";

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
        .route(
            "/_codex-unified/{capability}/v1/responses",
            post(responses).get(responses_websocket),
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
    if !authorized_capability(&capability, &state) {
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
        transport = "sse",
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

async fn responses_websocket(
    Path(capability): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !authorized_capability(&capability, &state) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "local_auth_required",
            "The local Codex Unified capability is invalid.",
        );
    }

    if !has_websocket_beta(&headers) {
        return error_response(
            StatusCode::UPGRADE_REQUIRED,
            "websocket_beta_required",
            "Responses WebSocket requires the responses_websockets=2026-02-06 beta.",
        );
    }

    ws.on_upgrade(move |socket| websocket_loop(socket, state))
        .into_response()
}

async fn websocket_loop(mut socket: WebSocket, state: AppState) {
    while let Some(message) = socket.next().await {
        match message {
            Ok(Message::Text(text)) => {
                let payload = match serde_json::from_str::<Value>(text.as_str()) {
                    Ok(payload) => payload,
                    Err(_) => {
                        if !send_ws_error(
                            &mut socket,
                            "invalid_request",
                            "WebSocket request frame must contain valid JSON.",
                        )
                        .await
                        {
                            break;
                        }
                        continue;
                    }
                };

                if !process_websocket_request(&mut socket, &state, payload).await {
                    break;
                }
            }
            Ok(Message::Ping(payload)) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {
                if !send_ws_error(
                    &mut socket,
                    "unsupported_frame",
                    "Responses WebSocket accepts JSON text request frames.",
                )
                .await
                {
                    break;
                }
            }
        }
    }
}

async fn process_websocket_request(
    socket: &mut WebSocket,
    state: &AppState,
    payload: Value,
) -> bool {
    if payload.get("type").and_then(Value::as_str) != Some("response.create") {
        return send_ws_error(
            socket,
            "invalid_request_type",
            "Responses WebSocket request type must be response.create.",
        )
        .await;
    }

    let turn = match TurnEnvelope::from_responses_request(payload) {
        Ok(turn) => turn,
        Err(error) => {
            let code = turn_error_code(&error);
            return send_ws_error(socket, code, &error.to_string()).await;
        }
    };

    let provider = match state.providers.resolve(&turn.requested_model) {
        Some(provider) => provider,
        None => {
            return send_ws_error(
                socket,
                "model_not_routable",
                "No enabled provider route owns the requested model.",
            )
            .await;
        }
    };

    tracing::info!(
        trace_id = %turn.trace_id,
        model = %turn.requested_model,
        provider = provider.id(),
        transport = "websocket",
        "responses ingress accepted"
    );

    let source = match provider.execute(turn).await {
        Ok(stream) => stream,
        Err(error) => {
            return send_ws_canonical(socket, error.failure_event(None)).await;
        }
    };

    let mut stream = validated_event_stream(source);
    while let Some(result) = stream.next().await {
        let event = match result {
            Ok(event) => event,
            Err(error) => error.failure_event(None),
        };
        if !send_ws_canonical(socket, event).await {
            return false;
        }
    }

    true
}

async fn send_ws_canonical(socket: &mut WebSocket, event: CanonicalEvent) -> bool {
    let encoded = match serde_json::to_string(&event.to_responses_wire()) {
        Ok(encoded) => encoded,
        Err(_) => return false,
    };
    socket.send(Message::Text(encoded.into())).await.is_ok()
}

async fn send_ws_error(socket: &mut WebSocket, code: &str, message: &str) -> bool {
    let encoded = json!({
        "type": "error",
        "error": {
            "type": "invalid_request_error",
            "code": code,
            "message": message
        }
    })
    .to_string();

    socket.send(Message::Text(encoded.into())).await.is_ok()
}

fn has_websocket_beta(headers: &HeaderMap) -> bool {
    headers
        .get("openai-beta")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|token| token == RESPONSES_WEBSOCKET_BETA)
        })
}

fn authorized_capability(capability: &str, state: &AppState) -> bool {
    constant_time_equal(capability.as_bytes(), state.capability.as_bytes())
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

fn turn_error_code(error: &TurnEnvelopeError) -> &'static str {
    match error {
        TurnEnvelopeError::MissingTurnMetadata | TurnEnvelopeError::MissingTurnId => {
            "native_turn_metadata_required"
        }
        _ => "invalid_request",
    }
}

fn turn_error_response(error: TurnEnvelopeError) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        turn_error_code(&error),
        &error.to_string(),
    )
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
    use futures_util::{SinkExt, StreamExt, stream};
    use std::{net::SocketAddr, sync::Arc};
    use tokio::task::JoinHandle;
    use tokio_tungstenite::{
        MaybeTlsStream, WebSocketStream, connect_async,
        tungstenite::{
            Message as TungsteniteMessage, client::IntoClientRequest, http::HeaderValue,
        },
    };
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
            "type": "response.create",
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

    async fn spawn(app: Router) -> (SocketAddr, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test router");
        });
        (address, task)
    }

    async fn connect_ws(
        address: SocketAddr,
        capability: &str,
    ) -> WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>> {
        let url = format!("ws://{address}/_codex-unified/{capability}/v1/responses");
        let mut request = url.into_client_request().expect("WebSocket request");
        request.headers_mut().insert(
            "openai-beta",
            HeaderValue::from_static(RESPONSES_WEBSOCKET_BETA),
        );
        let (socket, response) = connect_async(request).await.expect("connect websocket");
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        socket
    }

    async fn next_ws_json(
        socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Value {
        loop {
            let message = socket
                .next()
                .await
                .expect("WebSocket message")
                .expect("WebSocket frame");
            if let TungsteniteMessage::Text(text) = message {
                return serde_json::from_str(text.as_str()).expect("JSON WebSocket frame");
            }
        }
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

    #[tokio::test]
    async fn websocket_relays_the_same_canonical_events() {
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider { abrupt_eof: false });
        let (address, task) = spawn(test_app(Some(provider))).await;
        let mut socket = connect_ws(address, CAPABILITY).await;

        socket
            .send(TungsteniteMessage::Text(payload().to_string().into()))
            .await
            .expect("send response.create");

        assert_eq!(next_ws_json(&mut socket).await["type"], "response.created");
        assert_eq!(
            next_ws_json(&mut socket).await["type"],
            "response.output_text.delta"
        );
        assert_eq!(
            next_ws_json(&mut socket).await["type"],
            "response.completed"
        );

        socket.close(None).await.expect("close websocket");
        task.abort();
    }

    #[tokio::test]
    async fn websocket_abrupt_eof_is_failure_not_completion() {
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider { abrupt_eof: true });
        let (address, task) = spawn(test_app(Some(provider))).await;
        let mut socket = connect_ws(address, CAPABILITY).await;

        socket
            .send(TungsteniteMessage::Text(payload().to_string().into()))
            .await
            .expect("send response.create");

        assert_eq!(next_ws_json(&mut socket).await["type"], "response.created");
        assert_eq!(
            next_ws_json(&mut socket).await["type"],
            "response.output_text.delta"
        );
        let terminal = next_ws_json(&mut socket).await;
        assert_eq!(terminal["type"], "response.failed");
        assert_eq!(
            terminal["response"]["error"]["code"],
            "invalid_provider_response"
        );

        socket.close(None).await.expect("close websocket");
        task.abort();
    }

    #[tokio::test]
    async fn websocket_requires_beta_before_upgrade() {
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider { abrupt_eof: false });
        let (address, task) = spawn(test_app(Some(provider))).await;
        let url = format!("ws://{address}/_codex-unified/{CAPABILITY}/v1/responses");

        let error = connect_async(url)
            .await
            .expect_err("missing beta must reject upgrade");

        match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
            }
            other => panic!("unexpected WebSocket error: {other}"),
        }

        task.abort();
    }
}
