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

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;

    async fn capabilities(&self, model: &str) -> ProviderCapabilities;

    async fn execute(
        &self,
        turn: TurnEnvelope,
    ) -> Result<Vec<CanonicalEvent>, ProviderError>;
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
}
