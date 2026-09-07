use std::collections::HashSet;

use theseus_protocol::EventData;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FenceError {
    #[error("thread/meta must be the first event")]
    MetaMustBeFirst,
    #[error("thread/meta already recorded")]
    DuplicateMeta,
    #[error("turn {0} is already open")]
    TurnAlreadyOpen(u32),
    #[error("no open turn")]
    NoOpenTurn,
    #[error("open turn is {expected}, got {got}")]
    TurnMismatch { expected: u32, got: u32 },
    #[error("expected next turn {expected}, got {got}")]
    TurnSequence { expected: u32, got: u32 },
    #[error("step {0} is already open")]
    StepAlreadyOpen(u32),
    #[error("no open step")]
    NoOpenStep,
    #[error("open step is turn={expected_turn} step={expected_step}, got turn={turn} step={step}")]
    StepMismatch {
        expected_turn: u32,
        expected_step: u32,
        turn: u32,
        step: u32,
    },
    #[error("expected next step {expected} in turn {turn}, got {got}")]
    StepSequence { turn: u32, expected: u32, got: u32 },
    #[error("cannot close turn {turn} while step {step} is open")]
    CloseTurnWithOpenStep { turn: u32, step: u32 },
    #[error("assistant/chunk cannot enter the session log")]
    ChunkNotInHistory,
    #[error("user/message must be inside an open turn")]
    UserOutsideTurn,
    #[error("event must be inside an open step")]
    OutsideStep,
    #[error("tool call {0} is unknown in this step")]
    UnknownToolCall(String),
    #[error("tool call {0} already recorded in this step")]
    DuplicateToolCall(String),
    #[error("cannot fork: prefix through seq {0} ends inside an open turn")]
    ForkInsideTurn(u64),
    #[error("seq {0} is past the log end")]
    SeqOutOfRange(u64),
    #[error("turn {turn} reached max steps ({max})")]
    MaxStepsReached { turn: u32, max: u32, attempted: u32 },
}

/// Hard cap on `step/start` inside one turn. Prevents a tool-loop livelock.
pub const DEFAULT_MAX_STEPS_PER_TURN: u32 = 20;

#[derive(Debug, Clone)]
pub(crate) struct Fence {
    has_meta: bool,
    open_turn: Option<u32>,
    /// Last closed turn number (0 = none yet).
    last_turn: u32,
    open_step: Option<(u32, u32)>,
    /// Last closed step number in the current/last turn (0 = none).
    last_step_in_turn: u32,
    open_calls: HashSet<String>,
    max_steps_per_turn: u32,
}

impl Default for Fence {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_STEPS_PER_TURN)
    }
}

impl Fence {
    pub(crate) fn new(max_steps_per_turn: u32) -> Self {
        Self {
            has_meta: false,
            open_turn: None,
            last_turn: 0,
            open_step: None,
            last_step_in_turn: 0,
            open_calls: HashSet::new(),
            max_steps_per_turn: max_steps_per_turn.max(1),
        }
    }

    pub(crate) fn max_steps_per_turn(&self) -> u32 {
        self.max_steps_per_turn
    }

    pub(crate) fn check(&self, data: &EventData) -> Result<(), FenceError> {
        match data {
            EventData::AssistantChunk(_) => Err(FenceError::ChunkNotInHistory),
            EventData::ThreadMeta(_) => {
                if self.has_meta {
                    return Err(FenceError::DuplicateMeta);
                }
                if self.last_turn != 0 || self.open_turn.is_some() {
                    return Err(FenceError::MetaMustBeFirst);
                }
                Ok(())
            }
            EventData::TurnStart(t) => {
                if let Some(open) = self.open_turn {
                    return Err(FenceError::TurnAlreadyOpen(open));
                }
                let expected = self.last_turn + 1;
                if t.turn != expected {
                    return Err(FenceError::TurnSequence {
                        expected,
                        got: t.turn,
                    });
                }
                Ok(())
            }
            EventData::TurnEnd(t) => {
                let open = self.open_turn.ok_or(FenceError::NoOpenTurn)?;
                if open != t.turn {
                    return Err(FenceError::TurnMismatch {
                        expected: open,
                        got: t.turn,
                    });
                }
                if let Some((_, step)) = self.open_step {
                    return Err(FenceError::CloseTurnWithOpenStep { turn: t.turn, step });
                }
                Ok(())
            }
            EventData::StepStart(s) => {
                let open = self.open_turn.ok_or(FenceError::NoOpenTurn)?;
                if open != s.turn {
                    return Err(FenceError::TurnMismatch {
                        expected: open,
                        got: s.turn,
                    });
                }
                if let Some((_, step)) = self.open_step {
                    return Err(FenceError::StepAlreadyOpen(step));
                }
                let expected = self.last_step_in_turn + 1;
                if s.step != expected {
                    return Err(FenceError::StepSequence {
                        turn: s.turn,
                        expected,
                        got: s.step,
                    });
                }
                if s.step > self.max_steps_per_turn {
                    return Err(FenceError::MaxStepsReached {
                        turn: s.turn,
                        max: self.max_steps_per_turn,
                        attempted: s.step,
                    });
                }
                Ok(())
            }
            EventData::StepEnd(s) => {
                let (turn, step) = self.open_step.ok_or(FenceError::NoOpenStep)?;
                if turn != s.turn || step != s.step {
                    return Err(FenceError::StepMismatch {
                        expected_turn: turn,
                        expected_step: step,
                        turn: s.turn,
                        step: s.step,
                    });
                }
                Ok(())
            }
            EventData::UserMessage(m) => {
                let open = self.open_turn.ok_or(FenceError::UserOutsideTurn)?;
                if open != m.turn {
                    return Err(FenceError::TurnMismatch {
                        expected: open,
                        got: m.turn,
                    });
                }
                Ok(())
            }
            EventData::AssistantMessage(m) => self.require_open_step(m.turn, m.step),
            EventData::ToolCall(c) => {
                self.require_open_step(c.turn, c.step)?;
                if self.open_calls.contains(&c.call_id) {
                    return Err(FenceError::DuplicateToolCall(c.call_id.clone()));
                }
                Ok(())
            }
            EventData::ToolResult(r) => {
                self.require_open_step(r.turn, r.step)?;
                if !self.open_calls.contains(&r.call_id) {
                    return Err(FenceError::UnknownToolCall(r.call_id.clone()));
                }
                Ok(())
            }
        }
    }

    pub(crate) fn apply(&mut self, data: &EventData) {
        match data {
            EventData::AssistantChunk(_) => {}
            EventData::ThreadMeta(_) => self.has_meta = true,
            EventData::TurnStart(t) => {
                self.open_turn = Some(t.turn);
                self.last_step_in_turn = 0;
                self.open_calls.clear();
            }
            EventData::TurnEnd(t) => {
                self.open_turn = None;
                self.last_turn = t.turn;
                self.last_step_in_turn = 0;
                self.open_calls.clear();
            }
            EventData::StepStart(s) => {
                self.open_step = Some((s.turn, s.step));
                self.open_calls.clear();
            }
            EventData::StepEnd(s) => {
                self.open_step = None;
                self.last_step_in_turn = s.step;
                self.open_calls.clear();
            }
            EventData::UserMessage(_) | EventData::AssistantMessage(_) => {}
            EventData::ToolCall(c) => {
                self.open_calls.insert(c.call_id.clone());
            }
            EventData::ToolResult(r) => {
                self.open_calls.remove(&r.call_id);
            }
        }
    }

    pub(crate) fn accept(&mut self, data: &EventData) -> Result<(), FenceError> {
        self.check(data)?;
        self.apply(data);
        Ok(())
    }

    pub(crate) fn open_turn(&self) -> Option<u32> {
        self.open_turn
    }

    pub(crate) fn open_step(&self) -> Option<(u32, u32)> {
        self.open_step
    }

    pub(crate) fn next_turn(&self) -> u32 {
        self.last_turn + 1
    }

    pub(crate) fn next_step(&self) -> u32 {
        self.last_step_in_turn + 1
    }

    fn require_open_step(&self, turn: u32, step: u32) -> Result<(), FenceError> {
        let (open_turn, open_step) = self.open_step.ok_or(FenceError::OutsideStep)?;
        if open_turn != turn || open_step != step {
            return Err(FenceError::StepMismatch {
                expected_turn: open_turn,
                expected_step: open_step,
                turn,
                step,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use theseus_protocol::{TurnEnd, TurnEndReason, TurnStart};

    #[test]
    fn rejects_turn_start_while_open() {
        let mut fence = Fence::default();
        fence
            .accept(&EventData::TurnStart(TurnStart { turn: 1 }))
            .unwrap();
        let err = fence
            .check(&EventData::TurnStart(TurnStart { turn: 2 }))
            .unwrap_err();
        assert_eq!(err, FenceError::TurnAlreadyOpen(1));
    }

    #[test]
    fn rejects_turn_end_with_open_step() {
        let mut fence = Fence::default();
        fence
            .accept(&EventData::TurnStart(TurnStart { turn: 1 }))
            .unwrap();
        fence
            .accept(&EventData::StepStart(theseus_protocol::StepStart {
                turn: 1,
                step: 1,
            }))
            .unwrap();
        let err = fence
            .check(&EventData::TurnEnd(TurnEnd {
                turn: 1,
                reason: TurnEndReason::Completed,
            }))
            .unwrap_err();
        assert_eq!(err, FenceError::CloseTurnWithOpenStep { turn: 1, step: 1 });
    }

    #[test]
    fn rejects_step_past_max() {
        let mut fence = Fence::new(1);
        fence
            .accept(&EventData::TurnStart(TurnStart { turn: 1 }))
            .unwrap();
        fence
            .accept(&EventData::StepStart(theseus_protocol::StepStart {
                turn: 1,
                step: 1,
            }))
            .unwrap();
        fence
            .accept(&EventData::StepEnd(theseus_protocol::StepEnd {
                turn: 1,
                step: 1,
            }))
            .unwrap();
        let err = fence
            .check(&EventData::StepStart(theseus_protocol::StepStart {
                turn: 1,
                step: 2,
            }))
            .unwrap_err();
        assert_eq!(
            err,
            FenceError::MaxStepsReached {
                turn: 1,
                max: 1,
                attempted: 2
            }
        );
    }
}
