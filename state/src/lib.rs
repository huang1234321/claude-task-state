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

pub fn transition(rec: &SessionRecord, event: &Event, now: Instant) -> SessionRecord {
    let mut next = rec.clone();
    match event {
        Event::SessionStart => {
            next.state = SessionState::Starting;
            next.started_at = now;
            next.last_beat_at = Some(now);
            next.error_count = 0;
        }
        Event::UserPromptSubmit | Event::PreToolUse | Event::PostToolUse => {
            next.state = SessionState::Working;
            next.last_beat_at = Some(now);
        }
        Event::StatuslineBeat => {
            next.last_beat_at = Some(now);
            match rec.state {
                SessionState::Starting
                | SessionState::Waiting
                | SessionState::WaitingPermission
                | SessionState::Error => next.state = SessionState::Working,
                _ => {}
            }
        }
        Event::Stop => {
            next.state = SessionState::Waiting;
            next.last_stop_at = Some(now);
        }
        Event::NotificationIdle => next.state = SessionState::Waiting,
        Event::NotificationPermission => next.state = SessionState::WaitingPermission,
        Event::PostToolUseFailure => {
            next.state = SessionState::Error;
            next.error_count = rec.error_count.saturating_add(1);
            next.last_beat_at = Some(now);
        }
        Event::SessionEnd => next.state = SessionState::Ended,
        Event::HeartbeatTimeout => match rec.state {
            SessionState::Working | SessionState::Waiting | SessionState::WaitingPermission => {
                next.state = SessionState::Ended;
            }
            _ => {}
        },
    }
    next
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
