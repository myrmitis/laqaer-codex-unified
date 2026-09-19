use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnIdentity {
    pub thread_id: Option<String>,
    pub turn_id: String,
    pub parent_thread_id: Option<String>,
    pub request_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnEnvelope {
    pub trace_id: Uuid,
    pub identity: TurnIdentity,
    pub requested_model: String,
    pub client_metadata: Value,
    pub previous_response_id: Option<String>,
    pub prompt_cache_key: Option<String>,
    pub input: Value,
    pub tools: Value,
    pub raw_request: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    AuthRequired,
    RateLimited,
    QuotaExhausted,
    SecurityChallenge,
    ModelUnavailable,
    DomContractChanged,
    ProviderTimeout,
    TransportFailed,
    ContinuationMissing,
    InvalidProviderResponse,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CanonicalEvent {
    ResponseCreated {
        response_id: String,
    },
    OutputItemAdded {
        item_id: String,
        item_type: String,
    },
    TextDelta {
        item_id: String,
        delta: String,
    },
    FunctionCall {
        item_id: String,
        call_id: String,
        name: String,
        arguments: Value,
    },
    OutputItemDone {
        item_id: String,
    },
    ResponseCompleted {
        response_id: String,
    },
    ResponseFailed {
        response_id: Option<String>,
        code: FailureCode,
        message: String,
        retryable: bool,
    },
    ResponseIncomplete {
        response_id: Option<String>,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_identity_survives_provider_payload_mutation() {
        let envelope = TurnEnvelope {
            trace_id: Uuid::nil(),
            identity: TurnIdentity {
                thread_id: Some("thread-1".into()),
                turn_id: "turn-1".into(),
                parent_thread_id: None,
                request_kind: Some("turn".into()),
            },
            requested_model: "chatgpt-web/pro".into(),
            client_metadata: serde_json::json!({
                "x-codex-turn-metadata": "{\"turn_id\":\"turn-1\"}"
            }),
            previous_response_id: None,
            prompt_cache_key: None,
            input: serde_json::json!([]),
            tools: serde_json::json!([]),
            raw_request: serde_json::json!({
                "client_metadata": {"x": 1}
            }),
        };

        let mut provider_payload = envelope.raw_request.clone();
        provider_payload
            .as_object_mut()
            .expect("fixture object")
            .remove("client_metadata");

        assert_eq!(envelope.identity.turn_id, "turn-1");
        assert!(
            envelope
                .client_metadata
                .get("x-codex-turn-metadata")
                .is_some()
        );
    }
}
