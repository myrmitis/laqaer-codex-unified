use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TurnEnvelopeError {
    #[error("Responses request body must be a JSON object")]
    InvalidRequestRoot,
    #[error("Responses request is missing a model")]
    MissingModel,
    #[error("client_metadata must be a JSON object")]
    InvalidClientMetadata,
    #[error("native Codex turn metadata is required")]
    MissingTurnMetadata,
    #[error("native Codex turn metadata is not valid JSON")]
    InvalidTurnMetadata,
    #[error("native Codex turn metadata is missing turn_id")]
    MissingTurnId,
}

impl TurnEnvelope {
    pub fn from_responses_request(raw_request: Value) -> Result<Self, TurnEnvelopeError> {
        let object = raw_request
            .as_object()
            .ok_or(TurnEnvelopeError::InvalidRequestRoot)?;

        let requested_model = object
            .get("model")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(TurnEnvelopeError::MissingModel)?
            .to_owned();

        let client_metadata = object
            .get("client_metadata")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));

        let metadata_object = client_metadata
            .as_object()
            .ok_or(TurnEnvelopeError::InvalidClientMetadata)?;

        let encoded_turn = metadata_object
            .get("x-codex-turn-metadata")
            .ok_or(TurnEnvelopeError::MissingTurnMetadata)?;

        let turn_metadata = match encoded_turn {
            Value::String(value) => serde_json::from_str::<Value>(value)
                .map_err(|_| TurnEnvelopeError::InvalidTurnMetadata)?,
            Value::Object(_) => encoded_turn.clone(),
            _ => return Err(TurnEnvelopeError::InvalidTurnMetadata),
        };

        let turn_object = turn_metadata
            .as_object()
            .ok_or(TurnEnvelopeError::InvalidTurnMetadata)?;

        let turn_id = optional_string(turn_object, "turn_id")
            .filter(|value| !value.is_empty())
            .ok_or(TurnEnvelopeError::MissingTurnId)?;

        let parent_thread_id = optional_string(turn_object, "parent_thread_id").or_else(|| {
            metadata_object
                .get("x-codex-parent-thread-id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });

        Ok(Self {
            trace_id: Uuid::new_v4(),
            identity: TurnIdentity {
                thread_id: optional_string(turn_object, "thread_id"),
                turn_id,
                parent_thread_id,
                request_kind: optional_string(turn_object, "request_kind"),
            },
            requested_model,
            client_metadata,
            previous_response_id: optional_string(object, "previous_response_id"),
            prompt_cache_key: optional_string(object, "prompt_cache_key"),
            input: object.get("input").cloned().unwrap_or(Value::Null),
            tools: object
                .get("tools")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
            raw_request,
        })
    }
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
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

impl FailureCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuthRequired => "auth_required",
            Self::RateLimited => "rate_limited",
            Self::QuotaExhausted => "quota_exhausted",
            Self::SecurityChallenge => "security_challenge",
            Self::ModelUnavailable => "model_unavailable",
            Self::DomContractChanged => "dom_contract_changed",
            Self::ProviderTimeout => "provider_timeout",
            Self::TransportFailed => "transport_failed",
            Self::ContinuationMissing => "continuation_missing",
            Self::InvalidProviderResponse => "invalid_provider_response",
            Self::Cancelled => "cancelled",
        }
    }
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

impl CanonicalEvent {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::ResponseCompleted { .. }
                | Self::ResponseFailed { .. }
                | Self::ResponseIncomplete { .. }
        )
    }

    pub fn response_id(&self) -> Option<&str> {
        match self {
            Self::ResponseCreated { response_id } | Self::ResponseCompleted { response_id } => {
                Some(response_id)
            }
            Self::ResponseFailed { response_id, .. }
            | Self::ResponseIncomplete { response_id, .. } => response_id.as_deref(),
            _ => None,
        }
    }

    pub fn responses_event_name(&self) -> &'static str {
        match self {
            Self::ResponseCreated { .. } => "response.created",
            Self::OutputItemAdded { .. } => "response.output_item.added",
            Self::TextDelta { .. } => "response.output_text.delta",
            Self::FunctionCall { .. } => "response.output_item.done",
            Self::OutputItemDone { .. } => "response.output_item.done",
            Self::ResponseCompleted { .. } => "response.completed",
            Self::ResponseFailed { .. } => "response.failed",
            Self::ResponseIncomplete { .. } => "response.incomplete",
        }
    }

    pub fn to_responses_wire(&self) -> Value {
        match self {
            Self::ResponseCreated { response_id } => json!({
                "type": "response.created",
                "response": {
                    "id": response_id,
                    "object": "response",
                    "status": "in_progress"
                }
            }),
            Self::OutputItemAdded { item_id, item_type } => json!({
                "type": "response.output_item.added",
                "item": {
                    "id": item_id,
                    "type": item_type
                }
            }),
            Self::TextDelta { item_id, delta } => json!({
                "type": "response.output_text.delta",
                "item_id": item_id,
                "delta": delta
            }),
            Self::FunctionCall {
                item_id,
                call_id,
                name,
                arguments,
            } => json!({
                "type": "response.output_item.done",
                "item": {
                    "id": item_id,
                    "type": "function_call",
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments
                }
            }),
            Self::OutputItemDone { item_id } => json!({
                "type": "response.output_item.done",
                "item": {
                    "id": item_id
                }
            }),
            Self::ResponseCompleted { response_id } => json!({
                "type": "response.completed",
                "response": {
                    "id": response_id,
                    "object": "response",
                    "status": "completed"
                }
            }),
            Self::ResponseFailed {
                response_id,
                code,
                message,
                retryable,
            } => json!({
                "type": "response.failed",
                "response": {
                    "id": response_id,
                    "object": "response",
                    "status": "failed",
                    "error": {
                        "code": code.as_str(),
                        "message": message,
                        "retryable": retryable
                    }
                }
            }),
            Self::ResponseIncomplete {
                response_id,
                reason,
            } => json!({
                "type": "response.incomplete",
                "response": {
                    "id": response_id,
                    "object": "response",
                    "status": "incomplete",
                    "incomplete_details": {
                        "reason": reason
                    }
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(turn_metadata: Value) -> Value {
        serde_json::json!({
            "model": "chatgpt-web/pro",
            "client_metadata": {
                "x-codex-turn-metadata": turn_metadata
            },
            "previous_response_id": "response-1",
            "prompt_cache_key": "cache-1",
            "input": [{"type": "message"}],
            "tools": []
        })
    }

    #[test]
    fn canonical_identity_survives_provider_payload_mutation() {
        let turn = serde_json::json!({
            "thread_id": "thread-1",
            "turn_id": "turn-1",
            "request_kind": "turn"
        });

        let encoded = serde_json::to_string(&turn).expect("encode fixture");
        let envelope = TurnEnvelope::from_responses_request(request(Value::String(encoded)))
            .expect("parse fixture");

        let mut provider_payload = envelope.raw_request.clone();
        provider_payload
            .as_object_mut()
            .expect("fixture object")
            .remove("client_metadata");

        assert_eq!(envelope.identity.turn_id, "turn-1");
        assert_eq!(envelope.identity.thread_id.as_deref(), Some("thread-1"));
        assert!(
            envelope
                .client_metadata
                .get("x-codex-turn-metadata")
                .is_some()
        );
    }

    #[test]
    fn accepts_structured_turn_metadata_without_losing_it() {
        let turn = serde_json::json!({
            "thread_id": "thread-2",
            "turn_id": "turn-2",
            "request_kind": "turn"
        });

        let envelope =
            TurnEnvelope::from_responses_request(request(turn)).expect("parse structured metadata");

        assert_eq!(envelope.identity.turn_id, "turn-2");
        assert_eq!(envelope.previous_response_id.as_deref(), Some("response-1"));
        assert_eq!(envelope.prompt_cache_key.as_deref(), Some("cache-1"));
    }

    #[test]
    fn missing_native_identity_fails_closed() {
        let result = TurnEnvelope::from_responses_request(serde_json::json!({
            "model": "chatgpt-web/instant",
            "input": []
        }));

        assert_eq!(
            result.expect_err("must reject"),
            TurnEnvelopeError::MissingTurnMetadata
        );
    }

    #[test]
    fn wire_terminal_names_are_explicit() {
        let completed = CanonicalEvent::ResponseCompleted {
            response_id: "resp-1".into(),
        };
        assert!(completed.is_terminal());
        assert_eq!(completed.responses_event_name(), "response.completed");
        assert_eq!(
            completed.to_responses_wire()["response"]["status"],
            "completed"
        );
    }
}
