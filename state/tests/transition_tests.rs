use cc_state::{transition, Event, SessionRecord, SessionState};
use std::time::Instant;

fn rec() -> SessionRecord {
    SessionRecord::new("s1".into(), "proj".into(), "/p".into())
}

#[test]
fn user_prompt_then_stop_yields_working_then_waiting() {
    let now = Instant::now();
    let r = transition(&rec(), &Event::SessionStart, now);
    let r = transition(&r, &Event::UserPromptSubmit, now);
    assert_eq!(r.state, SessionState::Working);
    let r = transition(&r, &Event::Stop, now);
    assert_eq!(r.state, SessionState::Waiting);
    assert!(r.last_stop_at.is_some());
}

#[test]
fn beat_after_stop_means_new_turn_working() {
    let now = Instant::now();
    let r = transition(&rec(), &Event::SessionStart, now);
    let r = transition(&r, &Event::Stop, now);
    assert_eq!(r.state, SessionState::Waiting);
    let r = transition(&r, &Event::StatuslineBeat, now);
    assert_eq!(r.state, SessionState::Working);
}

#[test]
fn notification_permission_yields_orange() {
    let now = Instant::now();
    let r = transition(&rec(), &Event::SessionStart, now);
    let r = transition(&r, &Event::NotificationPermission, now);
    assert_eq!(r.state, SessionState::WaitingPermission);
}

#[test]
fn tool_failure_yields_error_then_recovers_on_stop() {
    let now = Instant::now();
    let r = transition(&rec(), &Event::SessionStart, now);
    let r = transition(&r, &Event::UserPromptSubmit, now);
    let r = transition(&r, &Event::PostToolUseFailure, now);
    assert_eq!(r.state, SessionState::Error);
    assert_eq!(r.error_count, 1);
    let r = transition(&r, &Event::Stop, now);
    assert_eq!(r.state, SessionState::Waiting);
}

#[test]
fn heartbeat_timeout_ends_live_sessions_only() {
    let now = Instant::now();
    let r = transition(&rec(), &Event::SessionStart, now);
    let r = transition(&r, &Event::UserPromptSubmit, now);
    let r = transition(&r, &Event::HeartbeatTimeout, now);
    assert_eq!(r.state, SessionState::Ended);
    let r = transition(&r, &Event::HeartbeatTimeout, now);
    assert_eq!(r.state, SessionState::Ended);
}

#[test]
fn session_end_is_terminal_until_session_start() {
    let now = Instant::now();
    let r = transition(&rec(), &Event::SessionStart, now);
    let r = transition(&r, &Event::SessionEnd, now);
    assert_eq!(r.state, SessionState::Ended);
    let r = transition(&r, &Event::StatuslineBeat, now);
    assert_eq!(r.state, SessionState::Ended);
    let r = transition(&r, &Event::SessionStart, now);
    assert_eq!(r.state, SessionState::Starting);
}
