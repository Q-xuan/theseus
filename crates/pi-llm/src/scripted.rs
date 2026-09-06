use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::LlmError;
use crate::seam::{ChatOutcome, ChatRequest, LlmSeam, ToolCallRequest};

/// One scripted model response for tests.
#[derive(Debug, Clone)]
pub enum ScriptedRound {
    Text(Vec<String>),
    ToolCalls {
        text: String,
        calls: Vec<ToolCallRequest>,
    },
    Fail(LlmError),
}

#[derive(Debug, Clone)]
enum ScriptKind {
    RepeatText(Vec<String>),
    AlwaysFail(LlmError),
    Sequence(Vec<ScriptedRound>),
}

/// In-process seam for tests. Never opens a network socket.
#[derive(Debug)]
pub struct ScriptedSeam {
    kind: ScriptKind,
    cursor: AtomicUsize,
}

impl ScriptedSeam {
    pub fn ok(chunks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            kind: ScriptKind::RepeatText(chunks.into_iter().map(Into::into).collect()),
            cursor: AtomicUsize::new(0),
        }
    }

    pub fn fail(err: LlmError) -> Self {
        Self {
            kind: ScriptKind::AlwaysFail(err),
            cursor: AtomicUsize::new(0),
        }
    }

    /// Consume one round per `stream_chat` call, in order.
    pub fn rounds(rounds: impl IntoIterator<Item = ScriptedRound>) -> Self {
        Self {
            kind: ScriptKind::Sequence(rounds.into_iter().collect()),
            cursor: AtomicUsize::new(0),
        }
    }
}

impl LlmSeam for ScriptedSeam {
    fn ready(&self) -> Result<(), LlmError> {
        match &self.kind {
            ScriptKind::AlwaysFail(LlmError::MissingApiKey) => Err(LlmError::MissingApiKey),
            _ => Ok(()),
        }
    }

    fn stream_chat(
        &self,
        _request: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatOutcome, LlmError> {
        match &self.kind {
            ScriptKind::AlwaysFail(err) => Err(err.clone()),
            ScriptKind::RepeatText(chunks) => {
                for chunk in chunks {
                    if !chunk.is_empty() {
                        on_delta(chunk);
                    }
                }
                Ok(ChatOutcome::text_only(chunks.concat()))
            }
            ScriptKind::Sequence(rounds) => {
                let i = self.cursor.fetch_add(1, Ordering::SeqCst);
                match rounds.get(i) {
                    None => Err(LlmError::InvalidResponse("script exhausted".into())),
                    Some(ScriptedRound::Fail(err)) => Err(err.clone()),
                    Some(ScriptedRound::Text(chunks)) => {
                        for chunk in chunks {
                            if !chunk.is_empty() {
                                on_delta(chunk);
                            }
                        }
                        Ok(ChatOutcome::text_only(chunks.concat()))
                    }
                    Some(ScriptedRound::ToolCalls { text, calls }) => {
                        if !text.is_empty() {
                            on_delta(text);
                        }
                        Ok(ChatOutcome {
                            text: text.clone(),
                            tool_calls: calls.clone(),
                        })
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::messages_from_derived;
    use pi_protocol::DerivedMessage;

    #[test]
    fn concatenates_chunks() {
        let seam = ScriptedSeam::ok(["Hel", "lo"]);
        let mut seen = Vec::new();
        let out = seam
            .stream_chat(
                &ChatRequest::new(
                    "gpt-4o-mini",
                    messages_from_derived(&[DerivedMessage::User {
                        content: "hi".into(),
                    }]),
                ),
                &mut |d| seen.push(d.to_string()),
            )
            .unwrap();
        assert_eq!(out.text, "Hello");
        assert!(out.tool_calls.is_empty());
        assert_eq!(seen, vec!["Hel", "lo"]);
    }
}
