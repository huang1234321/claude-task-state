use cc_state::{classify_hook, Event};

#[test]
fn maps_simple_events() {
    assert_eq!(classify_hook("SessionStart", None, false), Some(Event::SessionStart));
    assert_eq!(classify_hook("SessionEnd", None, false), Some(Event::SessionEnd));
    assert_eq!(classify_hook("UserPromptSubmit", None, false), Some(Event::UserPromptSubmit));
    assert_eq!(classify_hook("PreToolUse", None, false), Some(Event::PreToolUse));
    assert_eq!(classify_hook("Stop", None, false), Some(Event::Stop));
}

#[test]
fn posttooluse_with_error_is_failure() {
    assert_eq!(classify_hook("PostToolUse", None, true), Some(Event::PostToolUseFailure));
    assert_eq!(classify_hook("PostToolUse", None, false), Some(Event::PostToolUse));
}

#[test]
fn notification_routes_on_type() {
    assert_eq!(classify_hook("Notification", Some("idle_prompt"), false), Some(Event::NotificationIdle));
    assert_eq!(classify_hook("Notification", Some("permission_prompt"), false), Some(Event::NotificationPermission));
    assert_eq!(classify_hook("Notification", Some("auth_success"), false), None);
}

#[test]
fn ignores_unmapped_events() {
    assert_eq!(classify_hook("SubagentStop", None, false), None);
    assert_eq!(classify_hook("PreCompact", None, false), None);
    assert_eq!(classify_hook("Whatever", None, false), None);
}
