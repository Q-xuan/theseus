//! Thin OpenAI-compatible streaming seam.
//!
//! The API key is read only from [`ENV_API_KEY`] (`PI_LLM_API_KEY`). It is never
//! written to the session log, stdout traces, or this crate's `Debug` output.

mod error;
mod messages;
mod openai;
mod scripted;
mod seam;

pub use error::{
    default_base_url, default_model, LlmError, DEFAULT_BASE_URL, DEFAULT_MODEL, ENV_API_KEY,
    ENV_BASE_URL, ENV_MODEL,
};
pub use messages::{messages_from_derived, OpenAiChatMessage, OpenAiTool};
pub use openai::OpenAiChatSeam;
pub use scripted::{ScriptedRound, ScriptedSeam};
pub use seam::{ChatOutcome, ChatRequest, LlmSeam, ToolCallRequest};
