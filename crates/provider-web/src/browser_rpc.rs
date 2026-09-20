use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BROWSER_RPC_PROTOCOL: u8 = 1;
pub const CAPABILITY_HEADER: &str = "x-codex-unified-browser-capability";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSessionState {
    LoggedOut,
    Authenticating,
    Healthy,
    Challenged,
    NeedsReauth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebMode {
    Instant,
    Medium,
    High,
    ExtraHigh,
    Pro,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTurnIdentity {
    pub thread_id: Option<String>,
    pub turn_id: String,
    pub request_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTurnRequest {
    pub protocol: u8,
    pub trace_id: String,
    pub identity: BrowserTurnIdentity,
    pub mode: WebMode,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
}

impl BrowserTurnRequest {
    pub fn new(
        trace_id: impl Into<String>,
        identity: BrowserTurnIdentity,
        mode: WebMode,
        prompt: impl Into<String>,
        previous_response_id: Option<String>,
    ) -> Self {
        Self {
            protocol: BROWSER_RPC_PROTOCOL,
            trace_id: trace_id.into(),
            identity,
            mode,
            prompt: prompt.into(),
            previous_response_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSessionSnapshot {
    pub state: BrowserSessionState,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFailureCode {
    WebAuthRequired,
    WebSecurityChallenge,
    WebModelUnavailable,
    WebDomContractChanged,
    WebContinuationMissing,
    WebTransportFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserTurnResult {
    Success {
        response_id: String,
        text: String,
    },
    Failure {
        code: BrowserFailureCode,
        message: String,
        retryable: bool,
    },
}

#[derive(Debug, Error)]
pub enum BrowserRpcError {
    #[error("browser RPC origin must be an http://127.0.0.1 URL")]
    NonLoopbackOrigin,
    #[error("browser RPC capability must contain at least 24 characters")]
    InvalidCapability,
    #[error("browser RPC rejected the local capability")]
    Unauthorized,
    #[error("browser RPC protocol mismatch: expected {expected}, received {actual}")]
    ProtocolMismatch { expected: u8, actual: u64 },
    #[error("browser RPC returned HTTP {0}")]
    HttpStatus(u16),
    #[error("browser RPC returned an invalid response")]
    InvalidResponse,
    #[error("browser RPC transport failed")]
    Transport(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct BrowserRpcClient {
    origin: Url,
    capability: String,
    http: Client,
}

impl BrowserRpcClient {
    pub fn new(origin: &str, capability: impl Into<String>) -> Result<Self, BrowserRpcError> {
        let origin = Url::parse(origin).map_err(|_| BrowserRpcError::NonLoopbackOrigin)?;
        if origin.scheme() != "http"
            || origin.host_str() != Some("127.0.0.1")
            || origin.username() != ""
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(BrowserRpcError::NonLoopbackOrigin);
        }

        let capability = capability.into();
        if capability.len() < 24 {
            return Err(BrowserRpcError::InvalidCapability);
        }

        Ok(Self {
            origin,
            capability,
            http: Client::new(),
        })
    }

    pub async fn session(&self) -> Result<BrowserSessionSnapshot, BrowserRpcError> {
        let response = self
            .http
            .get(self.endpoint("v1/session")?)
            .header(CAPABILITY_HEADER, &self.capability)
            .send()
            .await?;

        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(BrowserRpcError::Unauthorized),
            status if !status.is_success() => {
                return Err(BrowserRpcError::HttpStatus(status.as_u16()));
            }
            _ => {}
        }

        let wire = response.json::<SessionWire>().await?;
        self.require_protocol(wire.protocol)?;
        Ok(BrowserSessionSnapshot {
            state: wire.state,
            revision: wire.revision,
        })
    }

    pub async fn execute_turn(
        &self,
        request: &BrowserTurnRequest,
    ) -> Result<BrowserTurnResult, BrowserRpcError> {
        if request.protocol != BROWSER_RPC_PROTOCOL {
            return Err(BrowserRpcError::ProtocolMismatch {
                expected: BROWSER_RPC_PROTOCOL,
                actual: u64::from(request.protocol),
            });
        }

        let response = self
            .http
            .post(self.endpoint("v1/turn")?)
            .header(CAPABILITY_HEADER, &self.capability)
            .json(request)
            .send()
            .await?;

        match response.status() {
            StatusCode::UNAUTHORIZED => return Err(BrowserRpcError::Unauthorized),
            StatusCode::CONFLICT => {
                let wire = response.json::<RpcFailureWire>().await?;
                if wire.code == "rpc_protocol_mismatch" {
                    return Err(BrowserRpcError::ProtocolMismatch {
                        expected: BROWSER_RPC_PROTOCOL,
                        actual: wire.protocol.unwrap_or_default(),
                    });
                }
                return Err(BrowserRpcError::HttpStatus(StatusCode::CONFLICT.as_u16()));
            }
            status if !status.is_success() => {
                return Err(BrowserRpcError::HttpStatus(status.as_u16()));
            }
            _ => {}
        }

        let wire = response.json::<TurnResultWire>().await?;
        wire.into_result()
    }

    fn endpoint(&self, path: &str) -> Result<Url, BrowserRpcError> {
        self.origin
            .join(path)
            .map_err(|_| BrowserRpcError::InvalidResponse)
    }

    fn require_protocol(&self, protocol: u64) -> Result<(), BrowserRpcError> {
        if protocol == u64::from(BROWSER_RPC_PROTOCOL) {
            return Ok(());
        }
        Err(BrowserRpcError::ProtocolMismatch {
            expected: BROWSER_RPC_PROTOCOL,
            actual: protocol,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SessionWire {
    protocol: u64,
    state: BrowserSessionState,
    revision: u64,
}

#[derive(Debug, Deserialize)]
struct RpcFailureWire {
    code: String,
    #[serde(default)]
    protocol: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnResultWire {
    ok: bool,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    code: Option<BrowserFailureCode>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    retryable: Option<bool>,
}

impl TurnResultWire {
    fn into_result(self) -> Result<BrowserTurnResult, BrowserRpcError> {
        if self.ok {
            let response_id = self.response_id.ok_or(BrowserRpcError::InvalidResponse)?;
            let text = self.text.ok_or(BrowserRpcError::InvalidResponse)?;
            if self.code.is_some() || self.message.is_some() || self.retryable.is_some() {
                return Err(BrowserRpcError::InvalidResponse);
            }
            return Ok(BrowserTurnResult::Success { response_id, text });
        }

        let code = self.code.ok_or(BrowserRpcError::InvalidResponse)?;
        let message = self.message.ok_or(BrowserRpcError::InvalidResponse)?;
        let retryable = self.retryable.ok_or(BrowserRpcError::InvalidResponse)?;
        if self.response_id.is_some() || self.text.is_some() {
            return Err(BrowserRpcError::InvalidResponse);
        }
        Ok(BrowserTurnResult::Failure {
            code,
            message,
            retryable,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio::task::JoinHandle;

    const CAPABILITY: &str = "browser-rpc-test-capability-0123456789";

    #[derive(Clone)]
    struct MockState {
        capability: Arc<str>,
    }

    async fn start_mock() -> (String, JoinHandle<()>) {
        let state = MockState {
            capability: Arc::from(CAPABILITY),
        };
        let app = Router::new()
            .route("/v1/session", get(mock_session))
            .route("/v1/turn", post(mock_turn))
            .route("/v1/protocol-mismatch", post(mock_protocol_mismatch))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock browser RPC");
        let address = listener.local_addr().expect("mock address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve mock browser RPC");
        });
        (format!("http://127.0.0.1:{}/", address.port()), handle)
    }

    fn assert_capability(headers: &HeaderMap, expected: &str) -> bool {
        headers
            .get(CAPABILITY_HEADER)
            .and_then(|value| value.to_str().ok())
            == Some(expected)
    }

    async fn mock_session(
        State(state): State<MockState>,
        headers: HeaderMap,
    ) -> Result<Json<Value>, StatusCode> {
        if !assert_capability(&headers, &state.capability) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(Json(json!({
            "protocol": 1,
            "state": "healthy",
            "revision": 7
        })))
    }

    async fn mock_turn(
        State(state): State<MockState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, StatusCode> {
        if !assert_capability(&headers, &state.capability) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        assert_eq!(body["protocol"], 1);
        assert_eq!(body["mode"], "pro");
        assert_eq!(body["identity"]["turnId"], "turn-1");
        Ok(Json(json!({
            "ok": true,
            "responseId": "response-pro",
            "text": "PRO OK"
        })))
    }

    async fn mock_protocol_mismatch() -> (StatusCode, Json<Value>) {
        (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "code": "rpc_protocol_mismatch",
                "message": "mismatch",
                "protocol": 2
            })),
        )
    }

    #[test]
    fn constructor_rejects_non_loopback_origins() {
        assert!(matches!(
            BrowserRpcClient::new("https://example.com/", CAPABILITY),
            Err(BrowserRpcError::NonLoopbackOrigin)
        ));
        assert!(matches!(
            BrowserRpcClient::new("http://localhost:3000/", CAPABILITY),
            Err(BrowserRpcError::NonLoopbackOrigin)
        ));
    }

    #[tokio::test]
    async fn session_round_trip_preserves_state_and_revision() {
        let (origin, handle) = start_mock().await;
        let client = BrowserRpcClient::new(&origin, CAPABILITY).expect("client");

        let session = client.session().await.expect("session");
        assert_eq!(session.state, BrowserSessionState::Healthy);
        assert_eq!(session.revision, 7);

        handle.abort();
    }

    #[tokio::test]
    async fn pro_turn_round_trip_uses_private_capability_header() {
        let (origin, handle) = start_mock().await;
        let client = BrowserRpcClient::new(&origin, CAPABILITY).expect("client");
        let request = BrowserTurnRequest::new(
            "trace-1",
            BrowserTurnIdentity {
                thread_id: Some("thread-1".into()),
                turn_id: "turn-1".into(),
                request_kind: Some("turn".into()),
            },
            WebMode::Pro,
            "test",
            None,
        );

        let result = client.execute_turn(&request).await.expect("turn");
        assert_eq!(
            result,
            BrowserTurnResult::Success {
                response_id: "response-pro".into(),
                text: "PRO OK".into(),
            }
        );

        handle.abort();
    }

    #[tokio::test]
    async fn wrong_capability_is_rejected() {
        let (origin, handle) = start_mock().await;
        let client =
            BrowserRpcClient::new(&origin, "wrong-browser-capability-0123456789").expect("client");

        assert!(matches!(
            client.session().await,
            Err(BrowserRpcError::Unauthorized)
        ));

        handle.abort();
    }
}
