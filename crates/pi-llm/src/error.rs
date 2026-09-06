use thiserror::Error;

pub const ENV_API_KEY: &str = "PI_LLM_API_KEY";
pub const ENV_BASE_URL: &str = "PI_LLM_BASE_URL";
pub const ENV_MODEL: &str = "PI_LLM_MODEL";
pub const DEFAULT_BASE_URL: &str = "https://ai.aruyx.com/";
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LlmError {
    #[error("{ENV_API_KEY} is not set")]
    MissingApiKey,
    #[error("llm http {status}: {message}")]
    Http { status: u16, message: String },
    #[error("llm transport: {0}")]
    Transport(String),
    #[error("llm invalid response: {0}")]
    InvalidResponse(String),
}

/// Replace any occurrence of `secret` so errors never echo the API key.
pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "[redacted]")
}

pub fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

pub fn default_model() -> String {
    std::env::var(ENV_MODEL)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

pub fn default_base_url() -> String {
    std::env::var(ENV_BASE_URL)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_strips_secret() {
        assert_eq!(
            redact("Bearer dummy-test-token exploded", "dummy-test-token"),
            "Bearer [redacted] exploded"
        );
        assert_eq!(redact("nothing", ""), "nothing");
    }
}
