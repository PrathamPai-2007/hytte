//! Talking to the user's own Ollama (`[ai]`): which models it has, a streamed answer to one
//! question, and pulling a model. Nothing here leaves the PC: only loopback hosts are accepted,
//! and the request goes through System32's `curl.exe` (as in `net.rs`) with every proxy off.
//!
//! The question travels on curl's stdin, so it never shows in a process list. Everything here
//! blocks: call it from a worker thread.
#![cfg_attr(not(windows), allow(dead_code))]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use std::time::Duration;

pub const DEFAULT_HOST: &str = "127.0.0.1:11434";
/// The model Hytte suggests: small (1.4 GB), Apache 2.0, answers well enough to explain an error.
pub const DEFAULT_MODEL: &str = "qwen3:1.7b";

/// Whether `host` ("127.0.0.1:11434", "localhost", "[::1]:11434") is this PC. Anything else is
/// refused, so the text asked about can never be sent to another machine.
pub fn host_ok(host: &str) -> bool {
    let h = host.trim();
    let (name, port) = match h.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((a, b)) => (a, b.strip_prefix(':')),
            None => return false,
        },
        None => match h.rsplit_once(':') {
            Some((a, b)) => (a, Some(b)),
            None => (h, None),
        },
    };
    let port_ok =
        port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok
        && matches!(
            name.to_ascii_lowercase().as_str(),
            "127.0.0.1" | "localhost" | "::1"
        )
}

fn url(host: &str, path: &str) -> String {
    format!("http://{}{path}", host.trim())
}

/// One installed model, as `/api/tags` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub name: String,
    pub bytes: u64,
}

/// What a question asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub model: String,
    /// Tells the model how to answer (short, plain text).
    pub system: String,
    pub prompt: String,
    /// Most tokens in the answer.
    pub max_tokens: u32,
    /// How long Ollama keeps the model in memory afterwards ("2m").
    pub keep_alive: String,
}

/// The JSON body of a chat request: one user message, thinking off, a short answer.
pub fn chat_body(q: &Question) -> String {
    json!({
        "model": q.model,
        "stream": true,
        "think": false,
        "keep_alive": q.keep_alive,
        "options": { "num_predict": q.max_tokens, "num_ctx": 4096, "temperature": 0.3 },
        "messages": [
            { "role": "system", "content": q.system },
            { "role": "user", "content": q.prompt },
        ],
    })
    .to_string()
}

/// One line of a streamed answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    Text(String),
    Done,
    Error(String),
}

/// A line of `/api/chat`'s stream. Empty or unrecognised lines say nothing.
pub fn parse_chat_line(line: &str) -> Option<Chunk> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Some(Chunk::Error(e.to_string()));
    }
    if v.get("done").and_then(Value::as_bool) == Some(true) {
        return Some(Chunk::Done);
    }
    let t = v.pointer("/message/content").and_then(Value::as_str)?;
    (!t.is_empty()).then(|| Chunk::Text(t.to_string()))
}

/// One line of `/api/pull`'s stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Pull {
    /// "pulling manifest", "verifying sha256 digest" ...
    Status(String),
    /// Bytes of the layer being fetched.
    Progress {
        done: u64,
        total: u64,
    },
    Done,
    Error(String),
}

pub fn parse_pull_line(line: &str) -> Option<Pull> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Some(Pull::Error(e.to_string()));
    }
    let status = v.get("status").and_then(Value::as_str)?;
    if status == "success" {
        return Some(Pull::Done);
    }
    // `completed` is missing until a layer has started: no progress to report yet.
    match (
        v.get("total").and_then(Value::as_u64),
        v.get("completed").and_then(Value::as_u64),
    ) {
        (Some(total), done) if total > 0 => Some(Pull::Progress {
            done: done.unwrap_or(0).min(total),
            total,
        }),
        _ => Some(Pull::Status(status.to_string())),
    }
}

/// The installed models out of a `/api/tags` reply.
pub fn parse_tags(body: &str) -> Vec<Installed> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    v.get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(Installed {
                name: m.get("name")?.as_str()?.to_string(),
                bytes: m.get("size").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect()
}

fn curl() -> Command {
    let exe = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(r"System32\curl.exe");
    let mut c = Command::new(exe);
    // Silent but with errors; no proxy ever (the text must not leave this PC); plain http, since
    // this is loopback; never follow a redirect to somewhere else.
    c.args(["-sS", "--noproxy", "*", "--proto", "=http"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Why a curl run failed, in words for the user.
fn curl_error(code: Option<i32>, stderr: &str) -> String {
    match code {
        Some(7) => "Ollama isn't running".into(),
        Some(28) => "Ollama took too long".into(),
        _ => {
            let s = stderr.trim();
            if s.is_empty() {
                "couldn't talk to Ollama".into()
            } else {
                s.to_string()
            }
        }
    }
}

/// GET `path`, the body as text.
fn get(host: &str, path: &str, secs: u32) -> Result<String, String> {
    if !host_ok(host) {
        return Err("Ollama's address must be this PC (127.0.0.1 or localhost)".into());
    }
    let o = curl()
        .args(["--max-time", &secs.to_string(), &url(host, path)])
        .output()
        .map_err(|e| format!("couldn't run curl.exe: {e}"))?;
    if !o.status.success() {
        return Err(curl_error(
            o.status.code(),
            &String::from_utf8_lossy(&o.stderr),
        ));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Ollama's version, or why it can't be reached.
pub fn version(host: &str) -> Result<String, String> {
    let body = get(host, "/api/version", 3)?;
    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("version")?.as_str().map(str::to_string))
        .ok_or_else(|| "that isn't Ollama".into())
}

/// The models Ollama has.
pub fn models(host: &str) -> Result<Vec<Installed>, String> {
    Ok(parse_tags(&get(host, "/api/tags", 5)?))
}

/// What the pill knows about Ollama, checked when the Ask pane opens.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Status {
    /// Not looked yet.
    #[default]
    Unknown,
    Checking,
    /// Nothing answers on the address.
    NotRunning,
    /// Running, without the chosen model.
    NoModel,
    Ready,
    /// The address is not this PC, or something else went wrong.
    Problem(String),
}

/// Whether `model` is among `installed`. A name without a tag means `:latest`.
pub fn has_model(installed: &[Installed], model: &str) -> bool {
    let want = if model.contains(':') {
        model.to_string()
    } else {
        format!("{model}:latest")
    };
    installed.iter().any(|m| m.name == want)
}

/// Look at Ollama: is it there, and does it have `model`.
pub fn check(host: &str, model: &str) -> Status {
    if !host_ok(host) {
        return Status::Problem("Ollama's address must be this PC (127.0.0.1 or localhost)".into());
    }
    if let Err(e) = version(host) {
        return if e.contains("isn't running") {
            Status::NotRunning
        } else {
            Status::Problem(e)
        };
    }
    match models(host) {
        Ok(m) if has_model(&m, model) => Status::Ready,
        Ok(_) => Status::NoModel,
        Err(e) => Status::Problem(e),
    }
}

/// POST `body` to `path` and hand each line of the reply to `on_line`, as it arrives, until it
/// returns false, `cancel` is set, or the reply ends. A cancelled run kills curl at once.
fn stream(
    host: &str,
    path: &str,
    body: &str,
    secs: u32,
    cancel: &AtomicBool,
    mut on_line: impl FnMut(&str) -> bool,
) -> Result<(), String> {
    if !host_ok(host) {
        return Err("Ollama's address must be this PC (127.0.0.1 or localhost)".into());
    }
    let mut child = curl()
        .args([
            "-N",
            "--max-time",
            &secs.to_string(),
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
            &url(host, path),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run curl.exe: {e}"))?;
    if let Some(mut si) = child.stdin.take() {
        // A thread, so a large body can't stall us if curl isn't reading yet.
        let body = body.to_string();
        std::thread::spawn(move || {
            let _ = si.write_all(body.as_bytes());
        });
    }
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
    }
    let mut stderr = child.stderr.take();
    let stop = loop {
        if cancel.load(Ordering::Relaxed) {
            break true;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(l) => {
                if !on_line(&l) {
                    break true;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break false,
        }
    };
    if stop {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(());
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        return Ok(());
    }
    let mut err = String::new();
    if let Some(s) = stderr.as_mut() {
        let _ = std::io::Read::read_to_string(s, &mut err);
    }
    Err(curl_error(status.code(), &err))
}

/// Ask `q`; each piece of the answer goes to `on_text` as it is made. Returns when the answer is
/// complete or `cancel` is set. An error Ollama reports (no such model ...) comes back as `Err`.
pub fn ask(
    host: &str,
    q: &Question,
    cancel: &AtomicBool,
    mut on_text: impl FnMut(&str),
) -> Result<(), String> {
    let mut failed: Option<String> = None;
    let mut done = false;
    stream(host, "/api/chat", &chat_body(q), 300, cancel, |l| {
        match parse_chat_line(l) {
            Some(Chunk::Text(t)) => on_text(&t),
            Some(Chunk::Done) => {
                done = true;
                return false;
            }
            Some(Chunk::Error(e)) => {
                failed = Some(e);
                return false;
            }
            None => {}
        }
        true
    })?;
    match failed {
        Some(e) => Err(e),
        None if done || cancel.load(Ordering::Relaxed) => Ok(()),
        None => Err("Ollama stopped answering".into()),
    }
}

/// Load `model` into memory without asking anything (a `/api/generate` with no prompt), so the
/// first real question skips the load. Ollama frees it `keep_alive` after the last request.
pub fn warm(host: &str, model: &str, keep_alive: &str) -> Result<(), String> {
    let body = json!({ "model": model, "keep_alive": keep_alive, "stream": false }).to_string();
    let mut r: Option<Result<(), String>> = None;
    stream(
        host,
        "/api/generate",
        &body,
        300,
        &AtomicBool::new(false),
        |l| {
            let v: Value = serde_json::from_str(l.trim()).unwrap_or(Value::Null);
            r = match v.get("error").and_then(Value::as_str) {
                Some(e) => Some(Err(e.to_string())),
                None if v.get("done").and_then(Value::as_bool) == Some(true) => Some(Ok(())),
                None => None,
            };
            r.is_none()
        },
    )?;
    r.unwrap_or_else(|| Err("Ollama stopped before the model was loaded".into()))
}

/// Download `model` through Ollama; `on_progress` gets (bytes done, bytes total) of the layer being
/// fetched. Returns when it is complete or `cancel` is set (Ollama keeps what it has).
pub fn pull(
    host: &str,
    model: &str,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(&Pull),
) -> Result<(), String> {
    let body = json!({ "model": model, "stream": true }).to_string();
    let mut failed: Option<String> = None;
    let mut done = false;
    stream(host, "/api/pull", &body, 3600, cancel, |l| {
        match parse_pull_line(l) {
            Some(Pull::Done) => {
                done = true;
                return false;
            }
            Some(Pull::Error(e)) => {
                failed = Some(e);
                return false;
            }
            Some(p) => on_progress(&p),
            None => {}
        }
        true
    })?;
    match failed {
        Some(e) => Err(e),
        None if done || cancel.load(Ordering::Relaxed) => Ok(()),
        None => Err("Ollama stopped before the download finished".into()),
    }
}

/// Overall progress of a pull as a fraction 0..1 across layers: Ollama reports one layer at a
/// time, so the bar follows the biggest layer (the model weights) and ignores the small ones.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PullBar {
    /// Total of the biggest layer seen, and how far it is.
    pub total: u64,
    pub done: u64,
    pub status: String,
}

impl PullBar {
    pub fn update(&mut self, p: &Pull) {
        match p {
            Pull::Progress { done, total } => {
                if *total >= self.total {
                    self.total = *total;
                    self.done = *done;
                }
            }
            Pull::Status(s) => self.status = s.clone(),
            Pull::Done | Pull::Error(_) => {}
        }
    }

    /// 0..1, or None before any layer started.
    pub fn fraction(&self) -> Option<f32> {
        (self.total > 0).then(|| self.done as f32 / self.total as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn only_this_pc_is_allowed() {
        for ok in [
            "127.0.0.1",
            "127.0.0.1:11434",
            "localhost:11434",
            "LOCALHOST",
            "[::1]:11434",
        ] {
            assert!(host_ok(ok), "{ok}");
        }
        for bad in [
            "",
            "example.com",
            "192.168.1.5:11434",
            "127.0.0.1.evil.com",
            "localhost.evil.com:11434",
            "127.0.0.1:",
            "127.0.0.1:abc",
            "user@127.0.0.1",
            "[::1",
            "0.0.0.0:11434",
            "127.0.0.1/api",
        ] {
            assert!(!host_ok(bad), "{bad}");
        }
    }

    #[test]
    fn chat_request_turns_thinking_off_and_caps_the_answer() {
        let q = Question {
            model: "qwen3:1.7b".into(),
            system: "Be brief.".into(),
            prompt: "why?".into(),
            max_tokens: 300,
            keep_alive: "2m".into(),
        };
        let v: Value = serde_json::from_str(&chat_body(&q)).unwrap();
        assert_eq!(v["model"], "qwen3:1.7b");
        assert_eq!(v["think"], false);
        assert_eq!(v["stream"], true);
        assert_eq!(v["keep_alive"], "2m");
        assert_eq!(v["options"]["num_predict"], 300);
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][1]["content"], "why?");
    }

    #[test]
    fn chat_lines_parse() {
        let t = r#"{"model":"m","message":{"role":"assistant","content":"Hel"},"done":false}"#;
        assert_eq!(parse_chat_line(t), Some(Chunk::Text("Hel".into())));
        let empty = r#"{"message":{"role":"assistant","content":""},"done":false}"#;
        assert_eq!(parse_chat_line(empty), None);
        let done = r#"{"done":true,"done_reason":"stop","eval_count":5}"#;
        assert_eq!(parse_chat_line(done), Some(Chunk::Done));
        let err = r#"{"error":"model 'x' not found"}"#;
        assert_eq!(
            parse_chat_line(err),
            Some(Chunk::Error("model 'x' not found".into()))
        );
        assert_eq!(parse_chat_line(""), None);
        assert_eq!(parse_chat_line("not json"), None);
    }

    #[test]
    fn pull_lines_parse_with_and_without_progress() {
        assert_eq!(
            parse_pull_line(r#"{"status":"pulling manifest"}"#),
            Some(Pull::Status("pulling manifest".into()))
        );
        // `completed` is missing before a layer starts.
        assert_eq!(
            parse_pull_line(r#"{"status":"pulling abc","digest":"abc","total":1000}"#),
            Some(Pull::Progress {
                done: 0,
                total: 1000
            })
        );
        assert_eq!(
            parse_pull_line(r#"{"status":"pulling abc","total":1000,"completed":250}"#),
            Some(Pull::Progress {
                done: 250,
                total: 1000
            })
        );
        // More than the total is clamped.
        assert_eq!(
            parse_pull_line(r#"{"status":"x","total":10,"completed":99}"#),
            Some(Pull::Progress {
                done: 10,
                total: 10
            })
        );
        assert_eq!(
            parse_pull_line(r#"{"status":"verifying sha256 digest"}"#),
            Some(Pull::Status("verifying sha256 digest".into()))
        );
        assert_eq!(parse_pull_line(r#"{"status":"success"}"#), Some(Pull::Done));
        assert_eq!(
            parse_pull_line(r#"{"error":"pull model manifest: file does not exist"}"#),
            Some(Pull::Error(
                "pull model manifest: file does not exist".into()
            ))
        );
    }

    #[test]
    fn the_bar_follows_the_biggest_layer() {
        let mut b = PullBar::default();
        assert_eq!(b.fraction(), None);
        b.update(&Pull::Progress {
            done: 10,
            total: 1_400_000_000,
        });
        b.update(&Pull::Progress {
            done: 100,
            total: 500,
        }); // a small layer: ignored
        b.update(&Pull::Progress {
            done: 700_000_000,
            total: 1_400_000_000,
        });
        assert_eq!(b.fraction(), Some(0.5));
    }

    #[test]
    fn tags_and_model_names() {
        let body = r#"{"models":[{"name":"qwen3:1.7b","size":1400000000},{"name":"llama3.2:latest","size":2019393189}]}"#;
        let m = parse_tags(body);
        assert_eq!(m.len(), 2);
        assert!(has_model(&m, "qwen3:1.7b"));
        assert!(has_model(&m, "llama3.2"));
        assert!(!has_model(&m, "qwen3:0.6b"));
        assert!(parse_tags("garbage").is_empty());
        assert!(parse_tags(r#"{"models":null}"#).is_empty());
    }

    /// A one-shot HTTP server on a free loopback port that replies with `lines` (one per
    /// `gap`) and records the request it got.
    fn serve(lines: Vec<String>, gap: Duration) -> (String, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = l.local_addr().unwrap().to_string();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let mut got = String::new();
            // Read the headers and the (small) body.
            while !got.contains("\r\n\r\n")
                || got.split("\r\n\r\n").nth(1).is_none_or(str::is_empty)
            {
                let n = s.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n");
            for line in lines {
                if s.write_all(format!("{line}\n").as_bytes()).is_err() {
                    break;
                }
                let _ = s.flush();
                std::thread::sleep(gap);
            }
            got
        });
        (host, h)
    }

    fn q() -> Question {
        Question {
            model: "m".into(),
            system: "s".into(),
            prompt: "secret question".into(),
            max_tokens: 50,
            keep_alive: "1m".into(),
        }
    }

    #[test]
    #[cfg(windows)]
    fn an_answer_streams_in_pieces_and_the_question_goes_in_the_body() {
        let chunk = |t: &str| {
            format!(r#"{{"message":{{"role":"assistant","content":"{t}"}},"done":false}}"#)
        };
        let lines = vec![chunk("Hel"), chunk("lo"), r#"{"done":true}"#.to_string()];
        let (host, server) = serve(lines, Duration::from_millis(5));
        let mut text = String::new();
        let r = ask(&host, &q(), &AtomicBool::new(false), |t| text.push_str(t));
        assert_eq!(r, Ok(()));
        assert_eq!(text, "Hello");
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /api/chat"));
        assert!(req.contains("secret question"));
        // Not on the command line: the request line carries no question.
        assert!(!req.lines().next().unwrap().contains("secret"));
    }

    #[test]
    #[cfg(windows)]
    fn an_error_line_is_an_error() {
        let (host, _s) = serve(
            vec![r#"{"error":"model 'm' not found"}"#.to_string()],
            Duration::from_millis(1),
        );
        let r = ask(&host, &q(), &AtomicBool::new(false), |_| {});
        assert_eq!(r, Err("model 'm' not found".into()));
    }

    #[test]
    #[cfg(windows)]
    fn stop_ends_a_slow_answer_quickly() {
        let chunk = r#"{"message":{"role":"assistant","content":"x"},"done":false}"#.to_string();
        let (host, _s) = serve(vec![chunk; 200], Duration::from_millis(100));
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.store(true, Ordering::Relaxed);
        });
        let t = Instant::now();
        let mut n = 0;
        let r = ask(&host, &q(), &cancel, |_| n += 1);
        assert_eq!(r, Ok(()));
        assert!((1..20).contains(&n), "got {n} pieces");
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "took {:?}",
            t.elapsed()
        );
    }

    #[test]
    #[cfg(windows)]
    fn a_pull_reports_progress_and_ends_on_success() {
        let lines = vec![
            r#"{"status":"pulling manifest"}"#.to_string(),
            r#"{"status":"pulling abc","digest":"abc","total":1000,"completed":400}"#.to_string(),
            r#"{"status":"pulling abc","digest":"abc","total":1000,"completed":1000}"#.to_string(),
            r#"{"status":"success"}"#.to_string(),
        ];
        let (host, server) = serve(lines, Duration::from_millis(2));
        let mut bar = PullBar::default();
        let r = pull(&host, "qwen3:1.7b", &AtomicBool::new(false), |p| {
            bar.update(p)
        });
        assert_eq!(r, Ok(()));
        assert_eq!(bar.fraction(), Some(1.0));
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /api/pull"));
        assert!(req.contains(r#""model":"qwen3:1.7b""#));
    }

    #[test]
    #[cfg(windows)]
    fn a_pull_error_is_reported_and_a_cut_stream_is_not_success() {
        let (host, _s) = serve(
            vec![r#"{"error":"pull model manifest: file does not exist"}"#.to_string()],
            Duration::from_millis(1),
        );
        let r = pull(&host, "nope", &AtomicBool::new(false), |_| {});
        assert_eq!(r, Err("pull model manifest: file does not exist".into()));
        // The server hangs up before "success": not a finished download.
        let (host, _s) = serve(
            vec![r#"{"status":"pulling manifest"}"#.to_string()],
            Duration::from_millis(1),
        );
        assert!(pull(&host, "m", &AtomicBool::new(false), |_| {}).is_err());
    }
    #[test]
    #[cfg(windows)]
    fn warming_loads_with_no_prompt_and_reports_errors() {
        let (host, server) = serve(
            vec![r#"{"model":"m","done":true,"done_reason":"load"}"#.to_string()],
            Duration::from_millis(1),
        );
        assert_eq!(warm(&host, "m", "2m"), Ok(()));
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /api/generate"));
        assert!(req.contains(r#""keep_alive":"2m""#) && !req.contains("prompt"));
        let (host, _s) = serve(
            vec![r#"{"error":"model 'm' not found"}"#.to_string()],
            Duration::from_millis(1),
        );
        assert_eq!(warm(&host, "m", "2m"), Err("model 'm' not found".into()));
    }

    #[test]
    #[cfg(windows)]
    fn nothing_listening_means_not_running() {
        // A port that was just free and is closed again.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = l.local_addr().unwrap().to_string();
        drop(l);
        assert_eq!(check(&host, "m"), Status::NotRunning);
    }

    #[test]
    fn a_remote_host_is_refused_before_anything_is_sent() {
        let r = ask("example.com:11434", &q(), &AtomicBool::new(false), |_| {});
        assert!(r.is_err());
        assert!(matches!(check("10.0.0.2:11434", "m"), Status::Problem(_)));
    }
}

/// Against a real Ollama on this PC: `HYTTE_OLLAMA_MODEL=<an installed model> cargo test live_ollama
/// -- --ignored --nocapture`.
#[cfg(all(test, windows))]
#[test]
#[ignore]
fn live_ollama() {
    let model = std::env::var("HYTTE_OLLAMA_MODEL").expect("set HYTTE_OLLAMA_MODEL");
    println!("version: {:?}", version(DEFAULT_HOST));
    let installed = models(DEFAULT_HOST).unwrap();
    println!("models: {installed:?}");
    println!("check: {:?}", check(DEFAULT_HOST, &model));
    assert_eq!(check(DEFAULT_HOST, &model), Status::Ready);
    let q = Question {
        model,
        system: "Reply in plain text, briefly.".into(),
        prompt: "In one sentence, why do we say a borrow checker error in Rust?".into(),
        max_tokens: 80,
        keep_alive: "30s".into(),
    };
    let mut pieces = 0;
    let mut text = String::new();
    let t = std::time::Instant::now();
    let mut first = None;
    let r = ask(DEFAULT_HOST, &q, &AtomicBool::new(false), |p| {
        first.get_or_insert(t.elapsed());
        pieces += 1;
        text.push_str(p);
    });
    println!(
        "result: {r:?}; {pieces} pieces; first {first:?}; total {:?}",
        t.elapsed()
    );
    println!("text: {text}");
    assert!(r.is_ok());
    assert!(pieces > 1, "should stream in pieces");
    assert!(!text.contains("<think>"));
    // A model that doesn't exist is Ollama's own error.
    let mut bad = q.clone();
    bad.model = "no-such-model:1b".into();
    let e = ask(DEFAULT_HOST, &bad, &AtomicBool::new(false), |_| {}).unwrap_err();
    println!("missing model: {e}");
    assert!(e.contains("not found"), "{e}");
}
