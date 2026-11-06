//! Translator that runs the user's installed Claude Code CLI (`claude -p`), so it uses
//! the Claude subscription the user is logged in with (`claude auth login`) and draws
//! from that plan's usage limits. Port of `claude-code.js`, which used the Agent SDK;
//! here the CLI is spawned directly in print mode. Each message starts a short
//! process, so it is slower than an API key and runs are capped to a small concurrency.
//! No stored API key is ever passed to it.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::prompt::{parse_llm_reply, system_prompt};
use super::ModelSource;
use crate::core::pipeline::{Translation, Translator};

pub const CLAUDE_CODE_MODELS: [&str; 3] = ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"];
pub const CONCURRENCY: usize = 2;
pub const TIMEOUT: Duration = Duration::from_secs(30);
pub const NOT_INSTALLED: &str = "Claude Code no está instalado en este PC (instálalo y ejecuta `claude auth login`)";

/// Arguments for a tool-less, settings-less, single-shot print-mode query. The chat
/// text goes on stdin, so a message starting with `-` is never read as a flag.
pub fn claude_args(model: &str, system_prompt: &str) -> Vec<String> {
    [
        "-p",
        "--model",
        model,
        "--system-prompt",
        system_prompt,
        // Pure translation: no file, shell or web access (and so a single turn).
        "--tools",
        "",
        // Ignore the user's settings, hooks and CLAUDE.md...
        "--setting-sources",
        "",
        // ...their MCP servers, and their skills / commands.
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--output-format",
        "json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Reads `--output-format json` output: one result object (or, with `--verbose`, an
/// array of messages whose result is the last `type == "result"`).
pub fn parse_claude_result(stdout: &str) -> Result<Translation, String> {
    let value: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    let result = match &value {
        Value::Array(messages) => messages.iter().rev().find(|m| m["type"] == "result"),
        Value::Object(_) if value["type"] == "result" => Some(&value),
        _ => None,
    };
    let Some(result) = result else { return Err("Claude Code: no result".into()) };
    if result["subtype"] != "success" || result["is_error"] != false {
        let detail = result["result"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| result["subtype"].as_str())
            .unwrap_or("no result");
        return Err(format!("Claude Code: {detail}"));
    }
    let parsed = parse_llm_reply(result["result"].as_str().unwrap_or(""));
    if parsed.text.is_empty() {
        return Err("Claude Code: empty reply".into());
    }
    Ok(parsed)
}

/// What a finished process printed.
#[derive(Clone, Debug, PartialEq)]
pub struct CliOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a program to completion; `Err` when it cannot start or exceeds `timeout`
/// (then it has been killed).
pub trait CliRunner: Send + Sync {
    fn run(&self, exe: &Path, args: &[String], stdin: &str, timeout: Duration) -> Result<CliOutput, String>;
}

/// Real runner: a hidden child process, killed when it does not finish in time.
pub struct ProcessRunner;

fn read_all(mut pipe: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
    });
    rx
}

impl CliRunner for ProcessRunner {
    fn run(&self, exe: &Path, args: &[String], stdin: &str, timeout: Duration) -> Result<CliOutput, String> {
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("no se pudo ejecutar {}: {e}", exe.display()))?;
        let stdout = read_all(child.stdout.take().expect("piped stdout"));
        let stderr = read_all(child.stderr.take().expect("piped stderr"));
        if let Some(mut pipe) = child.stdin.take() {
            let input = stdin.as_bytes().to_vec();
            // Written from a thread so a child that never reads cannot block us.
            std::thread::spawn(move || {
                let _ = pipe.write_all(&input);
            });
        }
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("sin respuesta tras {} s (time out)", timeout.as_secs()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(15)),
                Err(e) => {
                    let _ = child.kill();
                    return Err(e.to_string());
                }
            }
        };
        // The pipes close when the process exits; a grandchild holding them open
        // must not hang us, so wait only briefly.
        let grace = Duration::from_secs(2);
        Ok(CliOutput {
            success: status.success(),
            stdout: stdout.recv_timeout(grace).unwrap_or_default(),
            stderr: stderr.recv_timeout(grace).unwrap_or_default(),
        })
    }
}

/// At most `max` tasks run at once. A finishing task hands its slot straight to the
/// oldest waiter, so a new caller can never slip in between and exceed `max`.
struct Limiter {
    max: usize,
    state: Mutex<LimiterState>,
    wake: Condvar,
}

#[derive(Default)]
struct LimiterState {
    active: usize,
    next_ticket: u64,
    waiting: VecDeque<u64>,
    granted: HashSet<u64>,
}

struct Slot<'a>(&'a Limiter);

impl Limiter {
    fn new(max: usize) -> Self {
        Self { max: max.max(1), state: Mutex::new(LimiterState::default()), wake: Condvar::new() }
    }

    fn acquire(&self) -> Slot<'_> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.active < self.max && s.waiting.is_empty() {
            s.active += 1;
            return Slot(self);
        }
        let ticket = s.next_ticket;
        s.next_ticket += 1;
        s.waiting.push_back(ticket);
        while !s.granted.remove(&ticket) {
            s = self.wake.wait(s).unwrap_or_else(|e| e.into_inner());
        }
        Slot(self)
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        match s.waiting.pop_front() {
            Some(next) => {
                s.granted.insert(next);
                self.0.wake.notify_all();
            }
            None => s.active -= 1,
        }
    }
}

pub struct ClaudeCodeTranslator {
    exe: PathBuf,
    model: String,
    runner: Arc<dyn CliRunner>,
    limiter: Limiter,
    timeout: Duration,
}

impl ClaudeCodeTranslator {
    pub fn new(exe: PathBuf, model: &str, runner: Arc<dyn CliRunner>) -> Self {
        Self { exe, model: model.to_string(), runner, limiter: Limiter::new(CONCURRENCY), timeout: TIMEOUT }
    }
}

impl Translator for ClaudeCodeTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        let _slot = self.limiter.acquire();
        let args = claude_args(&self.model, &system_prompt(target));
        let out = self.runner.run(&self.exe, &args, text, self.timeout).map_err(|e| format!("Claude Code: {e}"))?;
        match parse_claude_result(&out.stdout) {
            Err(e) if !out.success && e == "Claude Code: no result" && !out.stderr.trim().is_empty() => {
                Err(format!("Claude Code: {}", super::http::snippet(out.stderr.trim(), 200)))
            }
            other => other,
        }
    }
}

impl ModelSource for ClaudeCodeTranslator {
    fn list_models(&self) -> Result<Vec<String>, String> {
        Ok(CLAUDE_CODE_MODELS.iter().map(|m| m.to_string()).collect())
    }
}

/// The user's installed `claude`: found with `where`, falling back to the native
/// installer's default location.
pub fn find_claude_executable() -> Option<PathBuf> {
    let mut cmd = Command::new("where");
    cmd.arg("claude").stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    if let Ok(out) = cmd.output() {
        let text = String::from_utf8_lossy(&out.stdout);
        let found = text.lines().map(str::trim).find(|l| {
            let lower = l.to_ascii_lowercase();
            lower.ends_with(".exe") || lower.ends_with(".cmd")
        });
        if let Some(path) = found {
            return Some(PathBuf::from(path));
        }
    }
    let home = std::env::var_os("USERPROFILE")?;
    let fallback = PathBuf::from(home).join(".local").join("bin").join("claude.exe");
    fallback.exists().then_some(fallback)
}
