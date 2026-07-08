use cc_collector::{ingest_hook, ingest_statusline, reap, view, AppState};
use std::time::{Duration, Instant};

fn statusline(id: &str, dir: &str) -> String {
    format!(r#"{{"session_id":"{id}","cwd":"{dir}","model":{{"display_name":"Opus"}}}}"#)
}
fn hook(id: &str, event: &str) -> String {
    format!(r#"{{"session_id":"{id}","hook_event_name":"{event}","cwd":"D:/code/proj"}}"#)
}

#[test]
fn statusline_then_stop_shows_working_then_waiting() {
    let s = AppState::default();
    ingest_statusline(&s.records, &statusline("s1", "D:/code/proj"));
    ingest_hook(&s.records, &hook("s1", "Stop"));
    let v = view(&s.records);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].color, "yellow");
    assert_eq!(v[0].state, "waiting");
}

#[test]
fn userprompt_beat_shows_working() {
    let s = AppState::default();
    ingest_statusline(&s.records, &statusline("s1", "D:/code/proj"));
    ingest_hook(&s.records, &hook("s1", "UserPromptSubmit"));
    let v = view(&s.records);
    assert_eq!(v[0].color, "green");
}

#[test]
fn permission_notification_is_orange() {
    let s = AppState::default();
    ingest_statusline(&s.records, &statusline("s1", "D:/code/proj"));
    ingest_hook(
        &s.records,
        r#"{"session_id":"s1","hook_event_name":"Notification","cwd":"D:/code/proj","notification_type":"permission_prompt"}"#,
    );
    let v = view(&s.records);
    assert_eq!(v[0].color, "orange");
}

#[test]
fn multiple_sessions_are_distinct() {
    let s = AppState::default();
    ingest_statusline(&s.records, &statusline("a", "D:/code/alpha"));
    ingest_statusline(&s.records, &statusline("b", "D:/code/beta"));
    assert_eq!(view(&s.records).len(), 2);
}

#[test]
fn reaper_ends_stale_and_removes_dead() {
    // Anchor "now" in the future so `now - duration` can't underflow. On Windows,
    // Instant is rooted at boot time, so `Instant::now() - large_duration` panics
    // when a fresh CI runner hasn't been up long enough. reap() takes `now` as a
    // parameter precisely so tests can use a fake clock — use it.
    let now = Instant::now() + Duration::from_secs(10000);

    let mut map = std::collections::HashMap::new();
    let mut r = cc_state::SessionRecord::new("s1".into(), "p".into(), "/p".into());
    r.state = cc_state::SessionState::Working;
    r.last_beat_at = Some(now - Duration::from_secs(120));
    map.insert("s1".into(), r);
    reap(&mut map, now, Duration::from_secs(30), Duration::from_secs(300));
    assert_eq!(map.get("s1").unwrap().state, cc_state::SessionState::Ended);

    let mut r2 = map.get_mut("s1").unwrap().clone();
    r2.last_beat_at = Some(now - Duration::from_secs(600));
    let mut m2 = std::collections::HashMap::new();
    m2.insert("s1".into(), r2);
    reap(&mut m2, now, Duration::from_secs(30), Duration::from_secs(300));
    assert!(m2.get("s1").is_none());
}
