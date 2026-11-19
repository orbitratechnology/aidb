//! Running one model request with the app's guarantees: the request is checked first, it stops
//! the moment it is cancelled, and it never runs past its time limit. Stopping drops the
//! request, which closes the connection and ends generation on the provider's side.

use std::future::Future;
use std::time::Duration;

use crate::adapters::ai_providers::{AiProvider, ProviderError, ProviderErrorKind};
use crate::domain::ai::{ChatEvent, ChatRequest, ChatResponse};

/// Sends `request` to `provider`, reporting progress to `on_event`, until it finishes, `cancel`
/// completes, or `time_limit` passes.
///
/// # Errors
///
/// [`ProviderErrorKind::InvalidRequest`] for a malformed request (nothing is sent),
/// [`ProviderErrorKind::Cancelled`], [`ProviderErrorKind::Timeout`], or the provider's error.
pub async fn chat<P: AiProvider>(
    provider: &P,
    request: &ChatRequest,
    time_limit: Duration,
    cancel: impl Future<Output = ()>,
    on_event: &mut (dyn FnMut(ChatEvent) + Send),
) -> Result<ChatResponse, ProviderError> {
    request.validate().map_err(|error| {
        ProviderError::new(
            ProviderErrorKind::InvalidRequest,
            format!("The request to the model is malformed: {error}."),
        )
    })?;
    tokio::select! {
        biased;
        () = cancel => Err(ProviderError::cancelled()),
        result = tokio::time::timeout(time_limit, provider.chat(request, on_event)) => {
            result.unwrap_or_else(|_| Err(ProviderError::timeout()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::domain::ai::{
        Message, ModelInfo, OutputFormat, ProviderCapabilities, ProviderKind, Role, StopReason,
        StructuredOutput, Usage,
    };

    /// Answers after `delay`, and records whether its request was dropped before finishing.
    #[derive(Debug, Default)]
    struct SlowProvider {
        delay: Duration,
        dropped: Arc<AtomicBool>,
    }

    struct DropFlag(Arc<AtomicBool>, bool);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            if !self.1 {
                self.0.store(true, Ordering::SeqCst);
            }
        }
    }

    impl AiProvider for SlowProvider {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                kind: ProviderKind::OpenAiCompatible,
                streaming: true,
                tool_calls: true,
                structured_output: StructuredOutput::JsonSchema,
                cancellation: true,
            }
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            Ok(Vec::new())
        }

        async fn chat(
            &self,
            _request: &ChatRequest,
            on_event: &mut (dyn FnMut(ChatEvent) + Send),
        ) -> Result<ChatResponse, ProviderError> {
            let mut flag = DropFlag(Arc::clone(&self.dropped), false);
            on_event(ChatEvent::Text("Working".into()));
            tokio::time::sleep(self.delay).await;
            flag.1 = true;
            Ok(ChatResponse {
                message: Message {
                    role: Role::Assistant,
                    parts: Vec::new(),
                },
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                model: None,
            })
        }
    }

    fn request() -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            system: None,
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            output: OutputFormat::Text,
            max_output_tokens: None,
        }
    }

    #[tokio::test]
    async fn finishes_within_its_limits() {
        let provider = SlowProvider::default();
        let mut events = Vec::new();
        let response = chat(
            &provider,
            &request(),
            Duration::from_secs(5),
            std::future::pending(),
            &mut |e| events.push(e),
        )
        .await
        .expect("response");
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(events, vec![ChatEvent::Text("Working".into())]);
    }

    #[tokio::test]
    async fn cancelling_drops_the_request() {
        let provider = SlowProvider {
            delay: Duration::from_secs(30),
            ..SlowProvider::default()
        };
        let error = chat(
            &provider,
            &request(),
            Duration::from_secs(60),
            tokio::time::sleep(Duration::from_millis(20)),
            &mut |_| {},
        )
        .await
        .expect_err("cancelled");
        assert_eq!(error.kind, ProviderErrorKind::Cancelled);
        assert!(provider.dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn the_time_limit_drops_the_request() {
        let provider = SlowProvider {
            delay: Duration::from_secs(30),
            ..SlowProvider::default()
        };
        let error = chat(
            &provider,
            &request(),
            Duration::from_millis(20),
            std::future::pending(),
            &mut |_| {},
        )
        .await
        .expect_err("timed out");
        assert_eq!(error.kind, ProviderErrorKind::Timeout);
        assert!(provider.dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn malformed_requests_are_not_sent() {
        let provider = SlowProvider::default();
        let mut bad = request();
        bad.messages.clear();
        let error = chat(
            &provider,
            &bad,
            Duration::from_secs(5),
            std::future::pending(),
            &mut |_| {},
        )
        .await
        .expect_err("invalid");
        assert_eq!(error.kind, ProviderErrorKind::InvalidRequest);
        assert!(!provider.dropped.load(Ordering::SeqCst));
    }
}
