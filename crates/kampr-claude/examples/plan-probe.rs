//! A manual probe of the billing guards against the operator's real plan login. A person runs
//! it, on the smallest model; it is never part of `make check` (guard 11):
//!
//!   cargo run -p kampr-claude --example plan-probe -- usage
//!   cargo run -p kampr-claude --example plan-probe -- init
//!   cargo run -p kampr-claude --example plan-probe -- turn --with-key
//!
//! The child is started through the guard code itself — `launch::argv` (guard 5) and
//! `launch::env` (guard 2), policy `plan`, the operator's own config dir. `--with-key` puts a
//! deliberately bogus `ANTHROPIC_API_KEY` in the parent env, to show the scrub blanks it: a
//! broken scrub would hand the child the key, which 401s without billing.
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use kampr_claude::config::Billing;
use kampr_claude::launch::{Plan, argv, env};

const BINARY: &str = "/home/dbrain/.local/bin/claude";
const FAKE_KEY: &str = "sk-ant-probe-fake-never-billed";

struct Lines {
    lines: Mutex<Vec<String>>,
    woke: Condvar,
}

impl Lines {
    fn new() -> Self {
        Self {
            lines: Mutex::new(Vec::new()),
            woke: Condvar::new(),
        }
    }

    fn push(&self, line: String) {
        let mut g = self.lines.lock().unwrap();
        g.push(line);
        self.woke.notify_all();
    }

    fn wait_for(&self, what: &str, until: Instant, take: impl Fn(&str) -> bool) -> Option<String> {
        let mut g = self.lines.lock().unwrap();
        loop {
            if let Some(i) = g.iter().position(|l| take(l)) {
                return Some(g.remove(i));
            }
            let now = Instant::now();
            if now >= until {
                eprintln!("timed out waiting for {what}; lines so far: {g:?}");
                return None;
            }
            g = self
                .woke
                .wait_timeout(g, until.saturating_duration_since(now))
                .unwrap()
                .0;
        }
    }

    fn all(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("usage");
    let with_key = args.iter().any(|a| a == "--with-key");

    if std::env::var_os("KAMPR_TEST").is_some() {
        eprintln!("refusing: KAMPR_TEST is set; this probe is for the real login");
        std::process::exit(2);
    }
    let mut inherited: Vec<(String, String)> = std::env::vars().collect();
    if with_key {
        inherited.push(("ANTHROPIC_API_KEY".into(), FAKE_KEY.into()));
    }

    let plan = Plan {
        binary: BINARY.into(),
        policy: Billing::Plan,
        ..Plan::default()
    };
    let mut cmd = Command::new(BINARY);
    for arg in argv(&plan).expect("the launch line") {
        if arg != BINARY {
            cmd.arg(arg);
        }
    }
    cmd.args(["--model", "haiku"]);
    let scrubbed = env(&plan, inherited);
    if with_key {
        let key = scrubbed.iter().find(|(k, _)| k == "ANTHROPIC_API_KEY");
        eprintln!("parent ANTHROPIC_API_KEY = {FAKE_KEY:?}; scrubbed = {key:?}");
    }
    cmd.env_clear();
    for (k, v) in scrubbed {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("the headless claude");

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let lines = Arc::new(Lines::new());
    let out = lines.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            out.push(line);
        }
    });
    let err = lines.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            eprintln!("stderr: {line}");
            err.push(format!("stderr: {line}"));
        }
    });
    let mut send = |obj: &serde_json::Value| {
        let mut line = serde_json::to_string(obj).unwrap();
        line.push('\n');
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();
    };
    let mut request = |subtype: &str| {
        send(&serde_json::json!({
            "type": "control_request",
            "request_id": subtype,
            "request": {"subtype": subtype},
        }))
    };
    let response_for = |id: &str, until: Instant| -> Option<serde_json::Value> {
        lines
            .wait_for(&format!("a control_response for {id}"), until, |l| {
                serde_json::from_str::<serde_json::Value>(l)
                    .ok()
                    .is_some_and(|v| {
                        v.get("type").and_then(|t| t.as_str()) == Some("control_response")
                            && v.pointer("/response/request_id").and_then(|r| r.as_str()) == Some(id)
                    })
            })
            .and_then(|l| serde_json::from_str(&l).ok())
    };

    let until = Instant::now() + Duration::from_secs(90);
    match mode {
        "usage" => {
            request("get_usage");
            if let Some(v) = response_for("get_usage", until) {
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            }
        }
        "init" => {
            request("initialize");
            if let Some(v) = response_for("initialize", until) {
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            }
        }
        "turn" => {
            request("initialize");
            if let Some(v) = response_for("initialize", until) {
                println!("initialize: {}", serde_json::to_string(&v).unwrap());
            }
            send(&serde_json::json!({
                "type": "user",
                "message": {"role": "user", "content": "Reply with the single word ok."},
                "parent_tool_use_id": null,
                "session_id": "",
            }));
            let init = lines.wait_for("the turn's system/init", until, |l| {
                serde_json::from_str::<serde_json::Value>(l)
                    .ok()
                    .filter(|v| {
                        v.get("type").and_then(|t| t.as_str()) == Some("system")
                            && v.get("subtype").and_then(|s| s.as_str()) == Some("init")
                    })
                    .is_some()
            });
            println!("system/init: {init:?}");
            let result = lines.wait_for("the result", until, |l| {
                serde_json::from_str::<serde_json::Value>(l)
                    .ok()
                    .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("result"))
                    .is_some()
            });
            println!("result: {result:?}");
        }
        other => {
            eprintln!("unknown mode {other}");
            std::process::exit(2);
        }
    }

    drop(stdin);
    let _ = child.wait();
    for line in lines.all() {
        if line.starts_with("stderr:") {
            continue;
        }
        eprintln!("line: {line}");
    }
}
