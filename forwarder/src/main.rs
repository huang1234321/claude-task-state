use std::io::Read;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    let _ = minreq::post(&url)
        .with_timeout(200)
        .with_header("Content-Type", "application/json")
        .with_body(body)
        .send();
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
    fn post_swallows_connection_failure() {
        post(65535, "statusline", b"{}");
    }

    #[test]
    fn post_delivers_body_to_server() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = br#"{"session_id":"x"}"#.to_vec();
        let handle = std::thread::spawn(move || post(port, "statusline", &body));
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        // headers + small body arrive together on localhost; read until end of headers
        for _ in 0..8 {
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let req = String::from_utf8_lossy(&buf);
        assert!(
            req.starts_with("POST /statusline HTTP/"),
            "bad request line: {}",
            &req[..req.len().min(50)]
        );
        assert!(req.contains("Content-Type: application/json"), "missing content-type");
        assert!(req.contains(r#""session_id":"x""#), "body not delivered");
        // respond so minreq::send() completes promptly instead of hitting its 200ms timeout
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").unwrap();
        handle.join().unwrap();
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
