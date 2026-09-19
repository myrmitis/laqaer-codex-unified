use async_trait::async_trait;
use codex_unified_protocol::{CanonicalEvent, FailureCode, TurnEnvelope};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EventSequenceError {
    #[error("provider event stream was empty")]
    Empty,
    #[error("provider event stream did not reach a terminal event")]
    MissingTerminal,
    #[error("provider event stream emitted an event after terminal")]
    EventAfterTerminal,
    #[error("provider event stream emitted multiple terminal events")]
    MultipleTerminal,
}

pub fn validate_event_sequence(events: &[CanonicalEvent]) -> Result<(), EventSequenceError> {
    if events.is_empty() {
        return Err(EventSequenceError::Empty);
    }

    let mut terminal_index = None;
    for (index, event) in events.iter().enumerate() {
        if event.is_terminal() {
            if terminal_index.is_some() {
                return Err(EventSequenceError::MultipleTerminal);
            }
            terminal_index = Some(index);
        } else if terminal_index.is_some() {
            return Err(EventSequenceError::EventAfterTerminal);
        }
    }

    match terminal_index {
        Some(index) if index == events.len() - 1 => Ok(()),
        Some(_) => Err(EventSequenceError::EventAfterTerminal),
        None => Err(EventSequenceError::MissingTerminal),
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;

    async fn capabilities(&self, model: &str) -> ProviderCapabilities;

    async fn execute(&self, turn: TurnEnvelope) -> Result<Vec<CanonicalEvent>, ProviderError>;
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

    #[test]
    fn browser_replay_stops_at_activation_boundary() {
        assert!(SubmissionState::Prepared.browser_retry_safe());
        assert!(!SubmissionState::Activated.browser_retry_safe());
        assert!(!SubmissionState::Accepted.browser_retry_safe());
    }

    #[test]
    fn event_sequence_requires_explicit_terminal() {
        let events = vec![
            CanonicalEvent::ResponseCreated {
                response_id: "r1".into(),
            },
            CanonicalEvent::TextDelta {
                item_id: "i1".into(),
                delta: "hello".into(),
            },
        ];
        assert_eq!(
            validate_event_sequence(&events),
            Err(EventSequenceError::MissingTerminal)
        );
    }

    #[test]
    fn event_sequence_accepts_one_final_terminal() {
        let events = vec![
            CanonicalEvent::ResponseCreated {
                response_id: "r1".into(),
            },
            CanonicalEvent::ResponseCompleted {
                response_id: "r1".into(),
            },
        ];
        assert_eq!(validate_event_sequence(&events), Ok(()));
    }
}
