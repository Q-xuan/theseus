use thiserror::Error;

pub const ENV_API_KEY: &str = "THESEUS_LLM_API_KEY";
pub const ENV_API_KEY_LEGACY: &str = "PI_LLM_API_KEY";
pub const ENV_BASE_URL: &str = "THESEUS_LLM_BASE_URL";
pub const ENV_BASE_URL_LEGACY: &str = "PI_LLM_BASE_URL";
pub const ENV_MODEL: &str = "THESEUS_LLM_MODEL";
pub const ENV_MODEL_LEGACY: &str = "PI_LLM_MODEL";
pub const DEFAULT_BASE_URL: &str = "https://ai.aruyx.com/";
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

fn first_nonempty_env(names: &[&str]) -> Option<String> {
    for name in names {
        if let Ok(v) = std::env::var(name) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Canonical `THESEUS_LLM_API_KEY`, then temporary `PI_LLM_API_KEY` fallback.
pub fn env_api_key() -> Option<String> {
    first_nonempty_env(&[ENV_API_KEY, ENV_API_KEY_LEGACY])
}

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
    first_nonempty_env(&[ENV_MODEL, ENV_MODEL_LEGACY])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

pub fn default_base_url() -> String {
    first_nonempty_env(&[ENV_BASE_URL, ENV_BASE_URL_LEGACY])
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

    #[test]
    fn env_api_key_prefers_theseus_then_legacy_pi() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap();
        std::env::remove_var(ENV_API_KEY);
        std::env::remove_var(ENV_API_KEY_LEGACY);
        assert_eq!(env_api_key(), None);
        std::env::set_var(ENV_API_KEY_LEGACY, "legacy-key");
        assert_eq!(env_api_key().as_deref(), Some("legacy-key"));
        std::env::set_var(ENV_API_KEY, "new-key");
        assert_eq!(env_api_key().as_deref(), Some("new-key"));
        std::env::remove_var(ENV_API_KEY);
        std::env::remove_var(ENV_API_KEY_LEGACY);
    }
}
