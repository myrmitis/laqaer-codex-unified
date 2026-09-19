use async_trait::async_trait;
use codex_unified_protocol::{CanonicalEvent, FailureCode, TurnEnvelope};
use futures_core::Stream;
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::{pin::Pin, sync::Arc};
use thiserror::Error;

pub type ProviderEventStream =
    Pin<Box<dyn Stream<Item = Result<CanonicalEvent, ProviderError>> + Send + 'static>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tools: bool,
    pub images: bool,
    pub continuation: bool,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("{code:?}: {message}")]
    Classified {
        code: FailureCode,
        message: String,
        retryable: bool,
    },
}

impl ProviderError {
    pub fn failure_event(self, response_id: Option<String>) -> CanonicalEvent {
        match self {
            Self::Classified {
                code,
                message,
                retryable,
            } => CanonicalEvent::ResponseFailed {
                response_id,
                code,
                message,
                retryable,
            },
        }
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;

    async fn capabilities(&self, model: &str) -> ProviderCapabilities;

    async fn execute(&self, turn: TurnEnvelope) -> Result<ProviderEventStream, ProviderError>;
}

pub trait ProviderResolver: Send + Sync {
    fn resolve(&self, model: &str) -> Option<Arc<dyn Provider>>;
}

pub struct EmptyProviderResolver;

impl ProviderResolver for EmptyProviderResolver {
    fn resolve(&self, _model: &str) -> Option<Arc<dyn Provider>> {
        None
    }
}

pub fn validated_event_stream(source: ProviderEventStream) -> ProviderEventStream {
    Box::pin(stream::unfold(
        ValidatorState {
            source,
            response_id: None,
            terminal_seen: false,
        },
        |mut state| async move {
            if state.terminal_seen {
                return None;
            }

            match state.source.next().await {
                Some(Ok(event)) => {
                    if let Some(response_id) = event.response_id() {
                        state.response_id = Some(response_id.to_owned());
                    }
                    if event.is_terminal() {
                        state.terminal_seen = true;
                    }

                    Some((Ok(event), state))
                }
                Some(Err(error)) => {
                    state.terminal_seen = true;
                    Some((Ok(error.failure_event(state.response_id.clone())), state))
                }
                None => {
                    state.terminal_seen = true;
                    Some((
                        Ok(CanonicalEvent::ResponseFailed {
                            response_id: state.response_id.clone(),
                            code: FailureCode::InvalidProviderResponse,
                            message: "provider stream ended before a terminal Responses event"
                                .into(),
                            retryable: false,
                        }),
                        state,
                    ))
                }
            }
        },
    ))
}

struct ValidatorState {
    source: ProviderEventStream,
    response_id: Option<String>,
    terminal_seen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionState {
    NotStarted,
    Prepared,
    Activated,
    Accepted,
    Terminal,
}

impl SubmissionState {
    pub fn browser_retry_safe(self) -> bool {
        matches!(self, Self::NotStarted | Self::Prepared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[test]
    fn browser_replay_stops_at_activation_boundary() {
        assert!(SubmissionState::Prepared.browser_retry_safe());
        assert!(!SubmissionState::Activated.browser_retry_safe());
        assert!(!SubmissionState::Accepted.browser_retry_safe());
    }

    #[tokio::test]
    async fn abrupt_provider_eof_becomes_failure_not_completion() {
        let source: ProviderEventStream =
            Box::pin(stream::iter(vec![Ok(CanonicalEvent::ResponseCreated {
                response_id: "resp-eof".into(),
            })]));

        let events: Vec<_> = validated_event_stream(source)
            .map(|item| item.expect("validator output"))
            .collect()
            .await;

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[1],
            CanonicalEvent::ResponseFailed {
                response_id: Some(id),
                code: FailureCode::InvalidProviderResponse,
                ..
            } if id == "resp-eof"
        ));
    }

    #[tokio::test]
    async fn explicit_terminal_event_is_preserved_exactly_once() {
        let source: ProviderEventStream = Box::pin(stream::iter(vec![
            Ok(CanonicalEvent::ResponseCreated {
                response_id: "resp-ok".into(),
            }),
            Ok(CanonicalEvent::ResponseCompleted {
                response_id: "resp-ok".into(),
            }),
        ]));

        let events: Vec<_> = validated_event_stream(source)
            .map(|item| item.expect("validator output"))
            .collect()
            .await;

        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[1],
            CanonicalEvent::ResponseCompleted { .. }
        ));
    }
}
