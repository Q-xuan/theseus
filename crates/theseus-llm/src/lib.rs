//! Thin OpenAI-compatible streaming seam.
//!
//! The API key is read only from [`ENV_API_KEY`] (`THESEUS_LLM_API_KEY`), with a
//! temporary fallback to `PI_LLM_API_KEY`. It is never written to the session
//! log, stdout traces, or this crate's `Debug` output.

mod error;
mod messages;
mod openai;
mod scripted;
mod seam;

pub use error::{
    default_base_url, default_model, env_api_key, LlmError, DEFAULT_BASE_URL, DEFAULT_MODEL,
    ENV_API_KEY, ENV_API_KEY_LEGACY, ENV_BASE_URL, ENV_BASE_URL_LEGACY, ENV_MODEL,
    ENV_MODEL_LEGACY,
};
pub use messages::{messages_from_derived, OpenAiChatMessage, OpenAiTool};
pub use openai::OpenAiChatSeam;
pub use scripted::{ScriptedRound, ScriptedSeam};
pub use seam::{ChatOutcome, ChatRequest, LlmSeam, ToolCallRequest};
