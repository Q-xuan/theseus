use crate::error::LlmError;
use crate::messages::{OpenAiChatMessage, OpenAiTool};

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiChatMessage>,
    pub tools: Vec<OpenAiTool>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<OpenAiChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: Vec::new(),
        }
    }
}

/// One model-requested tool call (from the provider stream, not from history).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Assembled assistant turn from one LLM request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatOutcome {
    pub text: String,
    pub tool_calls: Vec<ToolCallRequest>,
}

impl ChatOutcome {
    pub fn text_only(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tool_calls: Vec::new(),
        }
    }
}

/// OpenAI-compatible streaming chat.completions.
pub trait LlmSeam: Send {
    /// `Err(MissingApiKey)` when the seam cannot call a model.
    fn ready(&self) -> Result<(), LlmError>;

    /// Stream token deltas via `on_delta`. Returns assembled text plus any tool calls.
    /// Implementations must not write to the session log.
    fn stream_chat(
        &self,
        request: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatOutcome, LlmError>;
}
