# Claude Code 任务状态悬浮窗 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Windows-first Tauri floating widget that shows traffic-light status (green/yellow/orange/red/gray) of multiple foreground Claude Code sessions, fed by statusline + hooks forwarded to an embedded local collector.

**Architecture:** Three Rust crates share a pure state-machine core (`cc_state`): a tiny `cc-forward.exe` forwarder that statusline/hooks invoke to POST events over HTTP, a `cc_collector` lib that runs the HTTP ingestion server + reaper, and the Tauri app (`src-tauri`) that hosts the collector, exposes session state to the UI via Tauri IPC, and renders always-on-top cards. A config installer writes the statusline/hooks into `~/.claude/settings.json` (chaining any existing statusline).

**Tech Stack:** Rust (tiny_http, ureq, serde), Tauri v2, vanilla HTML/CSS/JS frontend. No Node bundler. Windows-first.

**Spec refinements vs. design doc (intentional):**
- UI reads session state via Tauri IPC (`invoke('get_sessions')`), not HTTP polling. The HTTP server is ingestion-only (receives forwarder POSTs). Avoids CORS; keeps UI in-process.
- `cc_state` extracted as a shared lib so the state machine is fully unit-testable without Tauri.
- Forwarder **chains** a pre-existing user statusline (stored in `forwarder.json`) instead of clobbering it.

**Plan deviations found during implementation (Tasks 6–7, all committed on `feat/core-logic`):**
- `cc_forwarder` uses `minreq` instead of `ureq`+`mockito`: ureq's default TLS pulls `ring` (C/asm), which fails under the GNU toolchain (no gcc). minreq is pure-Rust; the POST-delivery test uses a `std::net::TcpListener` mock instead of mockito.
- `cc_collector` has NO `ureq` dev-dependency (the plan listed one, but the tests call `ingest_*` directly — no HTTP client needed; declaring ureq would have pulled `ring`).
- Root `Cargo.toml` is a workspace (`members = [state, forwarder, collector]`); `src-tauri` will stay out of it.

**Repo layout (locked):**
```
claude-task-state/
├─ state/                 # cc_state lib — types, transition, classify_hook (TDD core)
│   ├─ Cargo.toml
│   ├─ src/lib.rs
│   └─ tests/{transition_tests.rs, classify_tests.rs}
├─ forwarder/             # cc_forwarder bin → cc-forward.exe
│   ├─ Cargo.toml
│   └─ src/main.rs
├─ collector/             # cc_collector lib — HTTP server, reaper, view
│   ├─ Cargo.toml
│   ├─ src/lib.rs
│   └─ tests/collector_tests.rs
├─ src-tauri/             # claude-task-state Tauri app
│   ├─ Cargo.toml, build.rs, tauri.conf.json
│   └─ src/{main.rs, installer.rs}
├─ src/                   # frontend (vanilla): index.html, main.js, style.css
├─ scripts/               # recon + e2e helpers
└─ docs/superpowers/{specs,plans}/
```

---

## Execution Approach (Plan C — no local admin)

The dev/build machine is a corporate Windows box where the user is a **standard user with no local admin** (cannot install MSVC / VS Build Tools). Therefore:

- **Core (Tasks 1–7):** develop locally with the **`x86_64-pc-windows-gnu`** Rust toolchain. rustup bundles the GNU linker, so **no admin, no MSVC** is needed for the pure-Rust crates (`cc_state`, `cc_forwarder`, `cc_collector` — no C deps).
- **Tauri UI (Tasks 8–11):** requires MSVC and **cannot be cross-built**. Defer until MSVC is available via one of:
  1. **GitHub Actions `windows-latest` runner** (MSVC + WebView2 preinstalled — no admin needed; the recommended path). Add `.github/workflows/build.yml` using `tauri-apps/tauri-action`; download the resulting installer artifact and run it on Windows.
  2. IT / domain admin installing VS Build Tools locally.
- **Fallback if CI / MSVC is unavailable:** swap the UI to **Electron** (`electron-builder --win` packages Windows installers from any OS without MSVC).

> Verified reachable from the dev box: `win.rustup.com`, `static.rust-lang.org`. The `x86_64-pc-windows-gnu` toolchain links pure-Rust crates out of the box.

---

## Task 1: Install Rust toolchain — GNU target, no admin

**Why first:** `cargo` is currently missing; nothing compiles without it. We use the **GNU** target so no MSVC / no admin is required (see Execution Approach above).

**Files:** none (environment only).

- [ ] **Step 1: Download rustup-init.exe**

From https://rustup.rs (or directly https://win.rustup.com/x86_64). Save anywhere, e.g. `Downloads`. User-level install — no admin, no UAC.

- [ ] **Step 2: Install the GNU toolchain**

In a regular (non-admin) PowerShell, in the download folder:
```
.\rustup-init.exe -y --default-toolchain stable-x86_64-pc-windows-gnu --profile minimal
```
If it prints a warning about Visual Studio / MSVC, ignore it — the GNU target does not need MSVC.

- [ ] **Step 3: New terminal, set default, verify**
```
rustup default stable-x86_64-pc-windows-gnu
cargo --version
rustc --version
```

- [ ] **Step 4: Link self-check (must produce an .exe)**
```
cargo new --bin _linkcheck
cd _linkcheck
cargo build
cd ..
Remove-Item -Recurse -Force _linkcheck
```
`cargo --version` alone can mask a missing linker. This step passes only if the GNU-bundled linker works. (If it fails to download crates, suspect a corp proxy — surface the error.)

> WebView2 / MSVC are NOT needed for Tasks 2–7. They are needed only for the Tauri UI (Tasks 8–11), built separately via GitHub Actions or IT — see Execution Approach.

---

## Task 2: Reconnaissance — capture real statusline/hooks JSON

**Why:** Field names in the design doc are documentation-sourced; the installed CC version may differ. Capture ground-truth payloads BEFORE locking the forwarder/collector contract. These become fixtures and validate Tasks 5 + 7.

**Files:** Create `scripts/recon-capture.ps1`; outputs `state/fixtures/*.json` + `state/fixtures/NOTES.md`.

- [ ] **Step 1: Write the capturing script**

`scripts/recon-capture.ps1`:
```powershell
param([string]$Kind = "statusline")
$ErrorActionPreference = "SilentlyContinue"
$dir = Join-Path $env:USERPROFILE ".claude\cc-recon"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$raw = [Console]::In.ReadToEnd()
$stamp = (Get-Date).ToString("yyyyMMdd-HHmmssfff")
if ($Kind -eq "hook") {
    try { $obj = $raw | ConvertFrom-Json; $tag = $obj.hook_event_name } catch { $tag = "unknown" }
    $file = Join-Path $dir ("hook-" + $tag + "-" + $stamp + ".json")
} else {
    $file = Join-Path $dir ("statusline-" + $stamp + ".json")
}
[IO.File]::WriteAllText($file, $raw, [Text.Encoding]::UTF8)
Write-Output "recon-capture"
```

- [ ] **Step 2: Temporarily install the capturing config**

Back up `~/.claude/settings.json` first. Then merge in:
```json
{
  "statusLine": { "type": "command", "command": "powershell -NoProfile -File D:/code/explore/claude-task-state/scripts/recon-capture.ps1 statusline" },
  "hooks": {
    "SessionStart":    [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File D:/code/explore/claude-task-state/scripts/recon-capture.ps1 hook" }] }],
    "SessionEnd":      [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File D:/code/explore/claude-task-state/scripts/recon-capture.ps1 hook" }] }],
    "UserPromptSubmit":[{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File D:/code/explore/claude-task-state/scripts/recon-capture.ps1 hook" }] }],
    "PreToolUse":      [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File D:/code/explore/claude-task-state/scripts/recon-capture.ps1 hook" }] }],
    "PostToolUse":     [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File D:/code/explore/claude-task-state/scripts/recon-capture.ps1 hook" }] }],
    "Notification":    [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File D:/code/explore/claude-task-state/scripts/recon-capture.ps1 hook" }] }],
    "Stop":            [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File D:/code/explore/claude-task-state/scripts/recon-capture.ps1 hook" }] }]
  }
}
```

- [ ] **Step 3: Exercise a real session**

Open a terminal in any project, run `claude`, and capture each: send a prompt; trigger a tool that needs approval; let it finish a turn; cause a tool failure (read a non-existent file); `/exit`.

- [ ] **Step 4: Inspect + copy fixtures**
```
ls ~/.claude/cc-recon
mkdir -p state/fixtures
cp ~/.claude/cc-recon/statusline-*.json        state/fixtures/statusline.sample.json
cp ~/.claude/cc-recon/hook-UserPromptSubmit-*  state/fixtures/hook_userpromptsubmit.json
cp ~/.claude/cc-recon/hook-Stop-*              state/fixtures/hook_stop.json
cp ~/.claude/cc-recon/hook-Notification-*      state/fixtures/hook_notification.json
cp ~/.claude/cc-recon/hook-SessionStart-*      state/fixtures/hook_sessionstart.json
```
Record actual field names in `state/fixtures/NOTES.md`: confirm `session_id`, `cwd`, `hook_event_name`, the Notification subtype field (`notification_type` vs `notification.type`), and how tool errors are flagged (`tool_response.is_error`? top-level `error`?). **Reconcile Tasks 5 + 7 with these notes.**

- [ ] **Step 5: Restore settings.json** from your Step-2 backup.

- [ ] **Step 6: Commit**
```
git add scripts state/fixtures
git commit -m "chore: capture real statusline/hook fixtures for contract"
```

---

## Task 3: `cc_state` crate — core types

**Files:** Create `state/Cargo.toml`, `state/src/lib.rs`.

- [ ] **Step 1: Scaffold**
```
cargo new --lib state
```

- [ ] **Step 2: Write `state/Cargo.toml`**
```toml
[package]
name = "cc_state"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }

[dev-dependencies]
serde_json = "1"
```

- [ ] **Step 3: Write `state/src/lib.rs`**
```rust
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
```

- [ ] **Step 4: Run tests**
```
cargo test -p cc_state
```
Expected: 2 passed.

- [ ] **Step 5: Commit**
```
git add state
git commit -m "feat(state): core types — SessionState, Event, SessionRecord"
```

---

## Task 4: `cc_state` — `transition` state machine (TDD)

**Files:** Modify `state/src/lib.rs` (add `transition`); test `state/tests/transition_tests.rs`.

- [ ] **Step 1: Write the failing tests**

`state/tests/transition_tests.rs`:
```rust
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
```

- [ ] **Step 2: Run to verify failure**
```
cargo test -p cc_state --test transition_tests
```
Expected: FAIL (`transition` not found).

- [ ] **Step 3: Implement `transition`** — append to `state/src/lib.rs` (before the `#[cfg(test)]` module):
```rust
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
```

- [ ] **Step 4: Run tests**
```
cargo test -p cc_state
```
Expected: all pass.

- [ ] **Step 5: Commit**
```
git add state
git commit -m "feat(state): transition state machine with full transitions"
```

---

## Task 5: `cc_state` — `classify_hook` (TDD)

**Files:** Modify `state/src/lib.rs` (add `classify_hook`); test `state/tests/classify_tests.rs`.

- [ ] **Step 1: Write the failing tests**

`state/tests/classify_tests.rs`:
```rust
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
```
> Reconcile `notification_type` values and the error-flag source with `state/fixtures/NOTES.md`. If the installed CC differs, update these tests AND the `Notification` arm below together.

- [ ] **Step 2: Run to verify failure**
```
cargo test -p cc_state --test classify_tests
```
Expected: FAIL (`classify_hook` not found).

- [ ] **Step 3: Implement** — append to `state/src/lib.rs` (before the `#[cfg(test)]` module):
```rust
/// Map a raw hook payload to a domain Event.
/// `notification_type`: Notification hook subtype (e.g. "idle_prompt").
/// `tool_error`: true when a PostToolUse payload indicates failure.
pub fn classify_hook(hook_event_name: &str, notification_type: Option<&str>, tool_error: bool) -> Option<Event> {
    match hook_event_name {
        "SessionStart" => Some(Event::SessionStart),
        "SessionEnd" => Some(Event::SessionEnd),
        "UserPromptSubmit" => Some(Event::UserPromptSubmit),
        "PreToolUse" => Some(Event::PreToolUse),
        "Stop" => Some(Event::Stop),
        "PostToolUse" if tool_error => Some(Event::PostToolUseFailure),
        "PostToolUse" => Some(Event::PostToolUse),
        "Notification" => match notification_type {
            Some("idle_prompt") => Some(Event::NotificationIdle),
            Some("permission_prompt") => Some(Event::NotificationPermission),
            _ => None,
        },
        _ => None,
    }
}
```

- [ ] **Step 4: Run tests**
```
cargo test -p cc_state
```
Expected: all pass.

- [ ] **Step 5: Commit**
```
git add state
git commit -m "feat(state): classify_hook maps raw hook payloads to events"
```

---

## Task 6: `cc_forwarder` crate (TDD)

**Files:** Create `forwarder/Cargo.toml`, `forwarder/src/main.rs`.

- [ ] **Step 1: Scaffold**
```
cargo new forwarder
```

- [ ] **Step 2: Write `forwarder/Cargo.toml`**
```toml
[package]
name = "cc_forwarder"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "cc-forward"
path = "src/main.rs"

[dependencies]
ureq = { version = "2", features = ["json"] }
serde_json = "1"

[dev-dependencies]
mockito = "1"
```

- [ ] **Step 3: Write `forwarder/src/main.rs`**
```rust
use std::io::Read;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn read_stdin() -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut buf);
    buf
}

// Config file location: env override (for tests) else %APPDATA%\cc-task-state\forwarder.json
fn config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CC_TASK_STATE_CONFIG") {
        return Some(PathBuf::from(p));
    }
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(&appdata).join("cc-task-state").join("forwarder.json"))
}

fn load_config() -> serde_json::Value {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn resolve_port(cli_port: Option<u16>) -> u16 {
    if let Some(p) = cli_port {
        return p;
    }
    if let Ok(s) = std::env::var("CC_TASK_STATE_PORT") {
        if let Ok(n) = s.parse::<u16>() {
            return n;
        }
    }
    load_config()
        .get("port")
        .and_then(|x| x.as_u64())
        .map(|n| n as u16)
        .unwrap_or(7331)
}

fn chain_command() -> Option<String> {
    load_config()
        .get("chain_command")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

fn post(port: u16, endpoint: &str, body: &[u8]) {
    let url = format!("http://127.0.0.1:{port}/{endpoint}");
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(200))
        .build();
    let _ = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_bytes(body);
}

fn derived_status_line(body: &[u8]) -> String {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    let model = v
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(|x| x.as_str())
        .unwrap_or("claude");
    let dir = v
        .get("cwd")
        .or_else(|| v.get("workspace").and_then(|w| w.get("current_dir")))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let base = Path::new(dir).file_name().and_then(|s| s.to_str()).unwrap_or(dir);
    format!("{model} | {base}")
}

// Run a pre-existing user statusline command (chain), piping the same stdin; return its stdout.
fn run_chain(cmd: &str, body: &[u8]) -> Option<String> {
    let mut child = Command::new("cmd")
        .arg("/C")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body);
    }
    let out = child.wait_with_output().ok()?;
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn status_line(body: &[u8]) -> String {
    if let Some(cmd) = chain_command() {
        if let Some(line) = run_chain(&cmd, body) {
            return line;
        }
    }
    derived_status_line(body)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("statusline");
    let cli_port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u16>().ok());
    let body = read_stdin();
    let port = resolve_port(cli_port);
    let endpoint = if mode == "hook" { "hook" } else { "statusline" };
    post(port, endpoint, &body);
    if mode != "hook" {
        println!("{}", status_line(&body));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    fn unique_config(json: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let p = std::env::temp_dir().join(format!("cc-fwd-cfg-{pid}-{n}.json"));
        std::fs::write(&p, json).unwrap();
        p
    }

    #[test]
    fn post_hits_server() {
        let mut server = mockito::Server::new();
        let m = server.mock("POST", "/statusline").with_status(200).create();
        let port: u16 = server.url().rsplit(':').next().unwrap().parse().unwrap();
        post(port, "statusline", b"{}");
        m.assert();
    }

    #[test]
    fn post_swallows_connection_failure() {
        post(65535, "statusline", b"{}");
    }

    #[test]
    fn derived_status_line_extracts_model_and_dir() {
        let body = br#"{"model":{"display_name":"Opus"},"workspace":{"current_dir":"D:/code/foo"}}"#;
        assert_eq!(derived_status_line(body), "Opus | foo");
    }

    #[test]
    fn resolve_port_prefers_cli_then_env() {
        assert_eq!(resolve_port(Some(9999)), 9999);
        std::env::set_var("CC_TASK_STATE_PORT", "8080");
        assert_eq!(resolve_port(None), 8080);
        std::env::remove_var("CC_TASK_STATE_PORT");
    }

    #[test]
    fn resolve_port_reads_config_file() {
        let cfg = unique_config(r#"{"port":7799}"#);
        std::env::set_var("CC_TASK_STATE_CONFIG", &cfg);
        assert_eq!(resolve_port(None), 7799);
        std::env::remove_var("CC_TASK_STATE_CONFIG");
    }

    #[test]
    fn chain_command_runs_and_returns_output() {
        let cfg = unique_config(r#"{"chain_command":"echo CHAINED-LINE"}"#);
        std::env::set_var("CC_TASK_STATE_CONFIG", &cfg);
        assert_eq!(status_line(b"{}"), "CHAINED-LINE");
        std::env::remove_var("CC_TASK_STATE_CONFIG");
    }

    #[test]
    fn status_line_falls_back_when_no_chain() {
        std::env::remove_var("CC_TASK_STATE_CONFIG");
        let body = br#"{"model":{"display_name":"Sonnet"}}"#;
        assert_eq!(status_line(body), "Sonnet | ");
    }
}
```
> The env-var tests mutate shared environment. Run them single-threaded (Step 4).

- [ ] **Step 4: Run tests (single-threaded for env-dependent tests)**
```
cargo test -p cc_forwarder -- --test-threads=1
```
Expected: 7 passed.

- [ ] **Step 5: Build the binary**
```
cargo build -p cc_forwarder --release
```
Expected: produces `target/release/cc-forward.exe`.

- [ ] **Step 6: Commit**
```
git add forwarder
git commit -m "feat(forwarder): cc-forward.exe ingests statusline/hooks, chains existing statusline"
```

---

## Task 7: `cc_collector` crate — HTTP ingestion + reaper (TDD)

**Files:** Create `collector/Cargo.toml`, `collector/src/lib.rs`; test `collector/tests/collector_tests.rs`.

- [ ] **Step 1: Scaffold**
```
cargo new --lib collector
```

- [ ] **Step 2: Write `collector/Cargo.toml`**
```toml
[package]
name = "cc_collector"
version = "0.1.0"
edition = "2021"

[dependencies]
cc_state = { path = "../state" }
tiny_http = "0.12"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[dev-dependencies]
cc_state = { path = "../state" }
ureq = { version = "2", features = ["json"] }
```

- [ ] **Step 3: Write `collector/src/lib.rs`**
```rust
use cc_state::{classify_hook, transition, Event, SessionRecord, SessionState};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
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
```

- [ ] **Step 4: Write the integration tests**

`collector/tests/collector_tests.rs`:
```rust
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
    let mut map = std::collections::HashMap::new();
    let mut r = cc_state::SessionRecord::new("s1".into(), "p".into(), "/p".into());
    r.state = cc_state::SessionState::Working;
    r.last_beat_at = Some(Instant::now() - Duration::from_secs(120));
    map.insert("s1".into(), r);
    reap(&mut map, Instant::now(), Duration::from_secs(30), Duration::from_secs(300));
    assert_eq!(map.get("s1").unwrap().state, cc_state::SessionState::Ended);

    let mut r2 = map.get_mut("s1").unwrap().clone();
    r2.last_beat_at = Some(Instant::now() - Duration::from_secs(600));
    let mut m2 = std::collections::HashMap::new();
    m2.insert("s1".into(), r2);
    reap(&mut m2, Instant::now(), Duration::from_secs(30), Duration::from_secs(300));
    assert!(m2.get("s1").is_none());
}
```

- [ ] **Step 5: Run tests**
```
cargo test -p cc_collector
```
Expected: 5 passed.

- [ ] **Step 6: Commit**
```
git add collector
git commit -m "feat(collector): HTTP ingestion, view, and reaper"
```

---

## Task 8: Tauri app shell — window, tray, collector, IPC

**Files:** Create `src-tauri/{Cargo.toml, build.rs, tauri.conf.json, src/main.rs}`; placeholder `src/index.html`.

- [ ] **Step 1: `src-tauri/Cargo.toml`**
```toml
[package]
name = "claude-task-state"
version = "0.1.0"
edition = "2021"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
cc_state = { path = "../state" }
cc_collector = { path = "../collector" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 2: `src-tauri/build.rs`**
```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 3: `src-tauri/tauri.conf.json`**
```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Claude Task State",
  "version": "0.1.0",
  "identifier": "com.huangjun1.claude-task-state",
  "build": { "frontendDist": "../src" },
  "app": {
    "withGlobalTauri": true,
    "windows": [{
      "label": "main",
      "title": "Claude Task State",
      "width": 240,
      "height": 320,
      "resizable": false,
      "decorations": false,
      "transparent": true,
      "alwaysOnTop": true,
      "skipTaskbar": true
    }],
    "security": { "csp": null }
  },
  "bundle": { "active": true, "targets": ["msi", "nsis"] }
}
```
> Production bundling of `cc-forward.exe` as a Tauri sidecar (target-triple-suffixed name + `externalBin`) is deferred; for dev/run we reference the built exe by path (Task 10).

- [ ] **Step 4: Placeholder frontend**

`src/index.html`:
```html
<!doctype html>
<html><head><meta charset="utf-8"><title>loading</title></head>
<body><div id="app">starting…</div></body></html>
```

- [ ] **Step 5: `src-tauri/src/main.rs`** (Tauri v2 tray API)
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cc_collector::{start_server, view, AppState};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

struct AppStateHolder(AppState);

#[tauri::command]
fn get_sessions(state: tauri::State<'_, AppStateHolder>) -> Vec<cc_collector::SessionView> {
    view(&state.0.records)
}

fn main() {
    let app_state = AppState::default();
    let _server_handle = start_server(AppState { records: app_state.records.clone() }, 7331);

    tauri::Builder::default()
        .manage(AppStateHolder(app_state))
        .setup(|app| {
            let toggle = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &quit])?;
            TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = if w.is_visible().unwrap_or(false) { w.hide() } else { w.show() };
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_sessions])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 6: Build + run (manual verify)**
```
cargo build -p claude-task-state
cargo run -p claude-task-state
```
Expected: a small borderless, always-on-top window showing "starting…"; a tray icon with Show/Hide + Quit. Verify the server:
```
curl -X POST http://127.0.0.1:7331/statusline -d '{}'
```
→ 200.

- [ ] **Step 7: Commit**
```
git add src-tauri src/index.html
git commit -m "feat(app): Tauri v2 shell — always-on-top window, tray, collector, IPC"
```

---

## Task 9: Frontend UI — cards, polling, colors, drag

**Files:** Create `src/index.html`, `src/main.js`, `src/style.css` (replaces the placeholder).

- [ ] **Step 1: `src/index.html`**
```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Claude Task State</title>
  <link rel="stylesheet" href="style.css" />
</head>
<body>
  <div id="app" data-tauri-drag-region>
    <div id="list"></div>
  </div>
  <script src="main.js"></script>
</body>
</html>
```

- [ ] **Step 2: `src/style.css`**
```css
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { height: 100%; background: transparent; font-family: "Segoe UI", system-ui, sans-serif; }
#app {
  height: 100vh; padding: 8px;
  background: rgba(24,24,28,0.92);
  border-radius: 10px;
  border: 1px solid rgba(255,255,255,0.08);
  overflow-y: auto;
}
.card {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 10px; margin-bottom: 6px;
  background: rgba(255,255,255,0.04); border-radius: 8px;
}
.dot { width: 12px; height: 12px; border-radius: 50%; flex: 0 0 auto; box-shadow: 0 0 6px currentColor; }
.label { color:#eee; font-size:13px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.state { color:#aaa; font-size:11px; margin-left:auto; }
.dot.green  { background:#3ddc84; color:#3ddc84; }
.dot.yellow { background:#ffd24a; color:#ffd24a; }
.dot.orange { background:#ff9e3d; color:#ff9e3d; }
.dot.red    { background:#ff5c5c; color:#ff5c5c; }
.dot.gray   { background:#888;    color:#888; }
.empty { color:#888; font-size:12px; padding:12px; text-align:center; }
```

- [ ] **Step 3: `src/main.js`**
```javascript
// Resolve invoke across Tauri v2 global shapes (core.invoke / tauri.invoke / legacy).
const invoke =
  window.__TAURI__?.core?.invoke ||
  window.__TAURI__?.tauri?.invoke ||
  window.__TAURI__?.invoke;

const STATE_LABEL = {
  starting: "…",
  working: "working",
  waiting: "waiting",
  waitingpermission: "your move",
  error: "error",
  ended: "ended",
};

function escapeHtml(t) {
  return t.replace(/[&<>"']/g, (c) => ({ "&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;" }[c]));
}

function render(sessions) {
  const list = document.getElementById("list");
  if (!sessions || !sessions.length) {
    list.innerHTML = '<div class="empty">No active Claude Code sessions</div>';
    return;
  }
  list.innerHTML = sessions
    .map((s) => `
      <div class="card">
        <span class="dot ${s.color}"></span>
        <span class="label">${escapeHtml(s.project)}</span>
        <span class="state">${STATE_LABEL[s.state] || s.state}</span>
      </div>`)
    .join("");
}

async function tick() {
  if (!invoke) return;
  try { render(await invoke("get_sessions")); } catch (e) { console.error(e); }
}

tick();
setInterval(tick, 1000);
```

- [ ] **Step 4: Build + manual verify**
```
cargo run -p claude-task-state
```
In another terminal, simulate events:
```
curl -X POST http://127.0.0.1:7331/statusline -d '{"session_id":"t1","cwd":"D:/code/alpha","model":{"display_name":"Opus"}}'
curl -X POST http://127.0.0.1:7331/hook -d '{"session_id":"t1","hook_event_name":"UserPromptSubmit","cwd":"D:/code/alpha"}'
```
Expected within ~1s: a green "alpha / working" card. Then:
```
curl -X POST http://127.0.0.1:7331/hook -d '{"session_id":"t1","hook_event_name":"Stop","cwd":"D:/code/alpha"}'
```
Expected: card turns yellow / "waiting". Drag the window by its background to move it.

- [ ] **Step 5: Commit**
```
git add src
git commit -m "feat(ui): traffic-light cards with 1s polling and drag"
```

---

## Task 10: Config installer — write/merge/uninstall settings.json + forwarder.json (TDD on merge)

**Files:** Create `src-tauri/src/installer.rs`; modify `src-tauri/src/main.rs`.

- [ ] **Step 1: Create `src-tauri/src/installer.rs` (pure fns + unit tests)**
```rust
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
```

- [ ] **Step 2: Run installer unit tests**
```
cargo test -p claude-task-state installer
```
Expected: 4 passed.

- [ ] **Step 3: Add disk I/O + commands to `src-tauri/src/main.rs`**

Insert at the top, after the `use` block:
```rust
mod installer;
use installer::{merge_hooks_for_event, merge_statusline, remove_tool_entries, HOOK_EVENTS};
use std::fs;
use std::path::PathBuf;

fn forwarder_exe_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("release")
        .join("cc-forward.exe")
        .to_string_lossy()
        .replace('\\', "/")
}

fn settings_path() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    PathBuf::from(home).join(".claude").join("settings.json")
}

fn forwarder_config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    PathBuf::from(appdata).join("cc-task-state").join("forwarder.json")
}

fn read_settings() -> serde_json::Value {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({}))
}

fn read_forwarder_config() -> serde_json::Value {
    fs::read_to_string(forwarder_config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({}))
}

fn write_settings(v: &serde_json::Value) -> std::io::Result<()> {
    let p = settings_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Ok(prev) = fs::read(&p) {
        let _ = fs::write(format!("{}.cc-task-state.bak", p.to_string_lossy()), prev);
    }
    fs::write(&p, serde_json::to_string_pretty(v)?)
}

fn write_forwarder_config(v: &serde_json::Value) -> std::io::Result<()> {
    let p = forwarder_config_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&p, serde_json::to_string_pretty(v)?)
}

fn install_config() -> std::io::Result<()> {
    let exe = forwarder_exe_path();
    let mut settings = read_settings();
    // Preserve a pre-existing user statusline so the forwarder can chain it.
    let existing = settings
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
        .filter(|c| !c.contains("cc-forward.exe"))
        .map(|s| s.to_string());
    write_forwarder_config(&serde_json::json!({
        "port": 7331,
        "chain_command": existing,
    }))?;
    merge_statusline(&mut settings, &exe);
    for ev in HOOK_EVENTS {
        merge_hooks_for_event(&mut settings, ev, &exe);
    }
    write_settings(&settings)
}

fn uninstall_config() -> std::io::Result<()> {
    let exe = forwarder_exe_path();
    let mut settings = read_settings();
    // Restore the user's original statusline if we chained one.
    let chain = read_forwarder_config()
        .get("chain_command")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    remove_tool_entries(&mut settings, &exe);
    if let Some(cmd) = chain {
        settings["statusLine"] = serde_json::json!({ "type": "command", "command": cmd });
    }
    write_settings(&settings)?;
    let _ = fs::remove_file(forwarder_config_path());
    Ok(())
}

#[tauri::command]
fn install_hooks() -> Result<String, String> {
    install_config().map(|_| "installed".into()).map_err(|e| e.to_string())
}

#[tauri::command]
fn uninstall_hooks() -> Result<String, String> {
    uninstall_config().map(|_| "uninstalled".into()).map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Wire the new commands + tray items**

In `src-tauri/src/main.rs`, update the `.invoke_handler(...)` line to:
```rust
.invoke_handler(tauri::generate_handler![get_sessions, install_hooks, uninstall_hooks])
```
Replace the `.setup(|app| { ... })` tray block with:
```rust
.setup(|app| {
    let toggle = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
    let install = MenuItem::with_id(app, "install", "Install CC config", true, None::<&str>)?;
    let uninstall = MenuItem::with_id(app, "uninstall", "Uninstall CC config", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &install, &uninstall, &quit])?;
    TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = if w.is_visible().unwrap_or(false) { w.hide() } else { w.show() };
                }
            }
            "install" => { let _ = install_config(); }
            "uninstall" => { let _ = uninstall_config(); }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
})
```

- [ ] **Step 5: Build + run installer tests + manual verify**
```
cargo test -p claude-task-state installer
cargo run -p claude-task-state
```
Tray → "Install CC config". Then `cat ~/.claude/settings.json`: confirm `statusLine` + `hooks` reference `cc-forward.exe`, your prior settings preserved, and `%APPDATA%\cc-task-state\forwarder.json` exists (with `chain_command` if you had a custom statusline). Tray → "Uninstall CC config": confirm the tool's entries are gone and any chained statusline restored.

- [ ] **Step 6: Commit**
```
git add src-tauri
git commit -m "feat(installer): merge/uninstall statusline+hooks; chain + restore existing statusline"
```

---

## Task 11: End-to-end replay + MVP acceptance

**Files:** Create `scripts/e2e-replay.ps1`; check off acceptance in the spec.

- [ ] **Step 1: `scripts/e2e-replay.ps1`**
```powershell
$ErrorActionPreference = "Stop"
$base = "http://127.0.0.1:7331"
function Post($path, $body) {
    Invoke-RestMethod -Method Post -Uri "$base$path" -Body $body -ContentType "application/json" | Out-Null
}
Post "/statusline" '{"session_id":"e2e","cwd":"D:/code/demo","model":{"display_name":"Opus"}}'
Start-Sleep -Milliseconds 300
Post "/hook" '{"session_id":"e2e","hook_event_name":"UserPromptSubmit","cwd":"D:/code/demo"}'
Start-Sleep -Seconds 1; Write-Host "expect: GREEN working"
Post "/hook" '{"session_id":"e2e","hook_event_name":"Notification","cwd":"D:/code/demo","notification_type":"permission_prompt"}'
Start-Sleep -Seconds 1; Write-Host "expect: ORANGE"
Post "/hook" '{"session_id":"e2e","hook_event_name":"Stop","cwd":"D:/code/demo"}'
Start-Sleep -Seconds 1; Write-Host "expect: YELLOW waiting"
Post "/hook" '{"session_id":"e2e","hook_event_name":"PostToolUse","cwd":"D:/code/demo","tool_response":{"is_error":true}}'
Start-Sleep -Seconds 1; Write-Host "expect: RED error"
Post "/hook" '{"session_id":"e2e","hook_event_name":"Stop","cwd":"D:/code/demo"}'
Start-Sleep -Seconds 1; Write-Host "expect: YELLOW (recovered)"
Post "/hook" '{"session_id":"e2e","hook_event_name":"SessionEnd","cwd":"D:/code/demo"}'
Start-Sleep -Seconds 1; Write-Host "expect: GRAY ended"
```

- [ ] **Step 2: Synthetic e2e**

Build the forwarder, then run the app, then replay:
```
cargo build -p cc_forwarder --release
cargo run -p claude-task-state
# another terminal:
powershell -NoProfile -File scripts/e2e-replay.ps1
```
Expected: the card transitions green → orange → yellow → red → yellow → gray in sync with the `expect:` lines.

- [ ] **Step 3: Real end-to-end (3 CC sessions)**

Tray → "Install CC config". Open 3 terminals in 3 different project dirs, run `claude` in each. Verify the design-doc §12 checklist:
- [ ] 3 cards appear.
- [ ] Send a prompt in one → green; finish → yellow.
- [ ] Trigger a tool approval → orange.
- [ ] Cause a tool error → red, then recovers.
- [ ] Close one terminal → gray then disappears.
- [ ] Quit the app → all CC TUIs remain responsive (statusline not laggy).
- [ ] Tray aggregate color reflects "anything needs attention."

- [ ] **Step 4: Commit**
```
git add scripts docs
git commit -m "test: e2e replay harness; MVP acceptance verified"
```

---

## Risks / notes carried into implementation

- **Tauri 2.x API drift:** Task 8/10 use `TrayIconBuilder`, `Menu::with_items`, `MenuItem::with_id`, `get_webview_window`, `withGlobalTauri`. If the resolved 2.x patch renames any, consult `https://tauri.app` for that version and adjust both tasks together.
- **Hook field reconciliation:** Task 2 fixtures are authoritative for `notification_type` values and the tool-error flag; update Task 5 tests + Task 7 extraction if they differ.
- **Forwarder must stay a native Rust bin** (never PowerShell) — it runs synchronously in the TUI statusline path. Enforced by Task 6.
- **Config agreement:** installer writes `%APPDATA%\cc-task-state\forwarder.json` (`port`, `chain_command`); the forwarder reads the same file. The two must agree — single source of truth.
