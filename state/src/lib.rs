use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionState {
    Starting,
    Working,
    Waiting,
    WaitingPermission,
    Error,
    Ended,
}

impl SessionState {
    pub fn color(self) -> &'static str {
        match self {
            SessionState::Starting => "gray",
            SessionState::Working => "green",
            SessionState::Waiting => "yellow",
            SessionState::WaitingPermission => "orange",
            SessionState::Error => "red",
            SessionState::Ended => "gray",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Stop,
    NotificationIdle,
    NotificationPermission,
    StatuslineBeat,
    SessionEnd,
    HeartbeatTimeout,
}

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub session_id: String,
    pub project: String,
    pub cwd: String,
    pub model: Option<String>,
    pub state: SessionState,
    pub last_stop_at: Option<Instant>,
    pub last_beat_at: Option<Instant>,
    pub started_at: Instant,
    pub error_count: u32,
}

impl SessionRecord {
    pub fn new(session_id: String, project: String, cwd: String) -> Self {
        Self {
            session_id,
            project,
            cwd,
            model: None,
            state: SessionState::Starting,
            last_stop_at: None,
            last_beat_at: None,
            started_at: Instant::now(),
            error_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mapping() {
        assert_eq!(SessionState::Working.color(), "green");
        assert_eq!(SessionState::Waiting.color(), "yellow");
        assert_eq!(SessionState::WaitingPermission.color(), "orange");
        assert_eq!(SessionState::Error.color(), "red");
        assert_eq!(SessionState::Ended.color(), "gray");
        assert_eq!(SessionState::Starting.color(), "gray");
    }

    #[test]
    fn new_record_starts_in_starting() {
        let r = SessionRecord::new("s1".into(), "proj".into(), "/p".into());
        assert_eq!(r.state, SessionState::Starting);
        assert_eq!(r.error_count, 0);
    }
}
