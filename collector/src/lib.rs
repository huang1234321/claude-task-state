use cc_state::{classify_hook, transition, Event, SessionRecord, SessionState};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_http::{Method, Response, Server};

pub struct AppState {
    pub records: Arc<Mutex<HashMap<String, SessionRecord>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self { records: Arc::new(Mutex::new(HashMap::new())) }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SessionView {
    pub session_id: String,
    pub project: String,
    pub state: String,
    pub color: String,
}

pub fn view(records: &Arc<Mutex<HashMap<String, SessionRecord>>>) -> Vec<SessionView> {
    let map = records.lock().unwrap();
    let mut v: Vec<SessionView> = map
        .values()
        .map(|r| SessionView {
            session_id: r.session_id.clone(),
            project: r.project.clone(),
            state: format!("{:?}", r.state).to_lowercase(),
            color: r.state.color().to_string(),
        })
        .collect();
    v.sort_by(|a, b| a.project.cmp(&b.project));
    v
}

/// Pure reaper: mark live-but-stale sessions Ended, drop long-dead ones. Testable with fake clocks.
pub fn reap(
    records: &mut HashMap<String, SessionRecord>,
    now: Instant,
    ended_after: Duration,
    remove_after: Duration,
) {
    for rec in records.values_mut() {
        if let Some(beat) = rec.last_beat_at {
            if now.duration_since(beat) > ended_after
                && matches!(rec.state, SessionState::Working | SessionState::Waiting | SessionState::WaitingPermission)
            {
                *rec = transition(rec, &Event::HeartbeatTimeout, now);
            }
        }
    }
    let mut to_remove = Vec::new();
    for (id, rec) in records.iter() {
        if matches!(rec.state, SessionState::Ended) {
            if let Some(beat) = rec.last_beat_at {
                if now.duration_since(beat) > remove_after {
                    to_remove.push(id.clone());
                }
            }
        }
    }
    for id in to_remove {
        records.remove(&id);
    }
}

fn project_of(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cwd)
        .to_string()
}

pub fn ingest_statusline(records: &Arc<Mutex<HashMap<String, SessionRecord>>>, body: &str) {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return,
    };
    let session_id = match v.get("session_id").and_then(|x| x.as_str()) {
        Some(s) => s.to_string(),
        None => return,
    };
    let cwd = v
        .get("cwd")
        .or_else(|| v.get("workspace").and_then(|w| w.get("current_dir")))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let model = v
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let now = Instant::now();
    let mut map = records.lock().unwrap();
    let rec = map
        .entry(session_id.clone())
        .or_insert_with(|| SessionRecord::new(session_id.clone(), project_of(&cwd), cwd.clone()));
    if let Some(m) = model {
        rec.model = Some(m);
    }
    if !cwd.is_empty() {
        rec.cwd = cwd.clone();
        rec.project = project_of(&cwd);
    }
    *rec = transition(rec, &Event::StatuslineBeat, now);
}

pub fn ingest_hook(records: &Arc<Mutex<HashMap<String, SessionRecord>>>, body: &str) {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return,
    };
    let session_id = match v.get("session_id").and_then(|x| x.as_str()) {
        Some(s) => s.to_string(),
        None => return,
    };
    let hook_event_name = match v.get("hook_event_name").and_then(|x| x.as_str()) {
        Some(s) => s.to_string(),
        None => return,
    };
    let cwd = v.get("cwd").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let notification_type = v
        .get("notification_type")
        .or_else(|| v.get("notification").and_then(|n| n.get("type")))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let tool_error = v
        .get("tool_response")
        .and_then(|r| r.get("is_error"))
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
        || v.get("error").is_some();
    let event = match classify_hook(&hook_event_name, notification_type.as_deref(), tool_error) {
        Some(e) => e,
        None => return,
    };
    let now = Instant::now();
    let mut map = records.lock().unwrap();
    let rec = map
        .entry(session_id.clone())
        .or_insert_with(|| SessionRecord::new(session_id.clone(), project_of(&cwd), cwd.clone()));
    *rec = transition(rec, &event, now);
}

pub fn start_server(state: AppState, port: u16) -> std::thread::JoinHandle<()> {
    let records = state.records.clone();
    std::thread::spawn(move || {
        let server = match Server::http(format!("127.0.0.1:{port}")) {
            Ok(s) => s,
            Err(_) => return,
        };
        for mut req in server.incoming_requests() {
            let url = req.url().to_string();
            let method = req.method().clone();
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            match (&method, url.as_str()) {
                (Method::Post, "/statusline") => {
                    ingest_statusline(&records, &body);
                    let _ = req.respond(Response::empty(200));
                }
                (Method::Post, "/hook") => {
                    ingest_hook(&records, &body);
                    let _ = req.respond(Response::empty(200));
                }
                _ => {
                    let _ = req.respond(Response::empty(404));
                }
            }
        }
    })
}
