use serde_json::{json, Value};

pub const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
];

/// Set statusLine to our forwarder (overwrites whatever is there; the prior command is
/// preserved separately in forwarder.json and chained by the forwarder at runtime).
pub fn merge_statusline(settings: &mut Value, exe: &str) {
    let obj = settings.as_object_mut().expect("settings must be an object");
    obj.insert(
        "statusLine".into(),
        json!({ "type": "command", "command": format!("{exe} statusline") }),
    );
}

/// Append a matcher for `event` that runs our forwarder; preserves existing matchers.
pub fn merge_hooks_for_event(settings: &mut Value, event: &str, exe: &str) {
    let obj = settings.as_object_mut().expect("settings must be an object");
    let hooks = obj.entry("hooks".to_string()).or_insert_with(|| json!({}));
    let arr = hooks
        .as_object_mut()
        .expect("hooks must be an object")
        .entry(event.to_string())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .expect("event list must be an array");
    arr.push(json!({
        "matcher": ".*",
        "hooks": [{ "type": "command", "command": format!("{exe} hook") }]
    }));
}

/// Remove everything this tool wrote: our statusLine (if it points at our exe) and our hook matchers.
/// User-authored entries are preserved.
pub fn remove_tool_entries(settings: &mut Value, exe: &str) {
    let obj = settings.as_object_mut().expect("settings must be an object");
    let ours_statusline = obj
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
        .map(|c| c.contains(exe))
        .unwrap_or(false);
    if ours_statusline {
        obj.remove("statusLine");
    }
    if let Some(hooks) = obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for (_ev, list) in hooks.iter_mut() {
            if let Some(arr) = list.as_array_mut() {
                arr.retain(|m| {
                    let ours = m
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
                        .any(|c| c.contains(exe));
                    !ours
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_statusline_sets_command() {
        let mut v = json!({});
        merge_statusline(&mut v, "C:/app/cc-forward.exe");
        assert_eq!(v["statusLine"]["command"], "C:/app/cc-forward.exe statusline");
    }

    #[test]
    fn merge_statusline_overwrites_existing() {
        let mut v = json!({ "statusLine": { "type": "command", "command": "old.sh" } });
        merge_statusline(&mut v, "C:/app/cc-forward.exe");
        assert_eq!(v["statusLine"]["command"], "C:/app/cc-forward.exe statusline");
    }

    #[test]
    fn merge_hooks_appends_without_clobbering() {
        let mut v = json!({ "hooks": { "Stop": [{ "matcher": ".*", "hooks": [{ "type": "command", "command": "user.sh" }] }] } });
        merge_hooks_for_event(&mut v, "Stop", "C:/app/cc-forward.exe");
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert!(stop.iter().any(|m| m["hooks"][0]["command"] == "user.sh"));
        assert!(stop.iter().any(|m| m["hooks"][0]["command"].as_str().unwrap().contains("cc-forward.exe")));
    }

    #[test]
    fn remove_tool_entries_strips_ours_keeps_user_hooks() {
        let mut v = json!({
          "statusLine": { "type": "command", "command": "C:/app/cc-forward.exe statusline" },
          "hooks": { "Stop": [
            { "matcher": ".*", "hooks": [{ "type": "command", "command": "user.sh" }] },
            { "matcher": ".*", "hooks": [{ "type": "command", "command": "C:/app/cc-forward.exe hook" }] }
          ] }
        });
        remove_tool_entries(&mut v, "C:/app/cc-forward.exe");
        assert!(v.get("statusLine").is_none());
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], "user.sh");
    }
}
