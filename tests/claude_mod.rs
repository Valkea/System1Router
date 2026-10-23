//! The CLI contract the Claude Code mod relies on (`claude-code/system1router`).
use serde_json::Value;
use std::{io::Write, path::Path, process::{Command, Stdio}};

/// Runs the CLI; `mod_session` is the session the mod marked, if any.
fn run(arguments: &[&str], stdin: &str, state: &Path, mod_session: Option<&str>) -> Value { run_with(arguments, stdin, state, mod_session, None) }

/// `jev` is the classification the Jev provider answers with.
fn run_with(arguments: &[&str], stdin: &str, state: &Path, mod_session: Option<&str>, jev: Option<&str>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_system1router"));
    command.args(arguments).env("XDG_STATE_HOME", state).env("SYSTEM1ROUTER_TELEMETRY_DISABLED", "1").env_remove("SYSTEM1ROUTER_TELEMETRY").env_remove("SYSTEM1ROUTER_CONSENT")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    match jev { Some(response) => command.env("SYSTEM1ROUTER_JEV_RESPONSE", response), None => command.env_remove("SYSTEM1ROUTER_JEV_RESPONSE") };
    match mod_session { Some(session) => command.env("SYSTEM1ROUTER_CLAUDE_MOD", session), None => command.env_remove("SYSTEM1ROUTER_CLAUDE_MOD") };
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{arguments:?} exits 0");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn config(directory: &Path, enabled: bool) -> String {
    let path = directory.join("config.json");
    std::fs::write(&path, format!(r#"{{"routing":{{"enabled":{enabled}}}}}"#)).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn decide_answers_none_when_routing_is_off_or_input_is_bad() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path(), false);
    let payload = r#"{"session_id":"s","cwd":"/tmp","prompt":"refactor the parser","model":"claude-sonnet-5-5"}"#;
    let off = run(&["decide", "--config", &config], payload, directory.path(), Some("s"));
    assert_eq!((off["action"].as_str(), off["code"].as_str(), off["current"].as_str()), (Some("none"), Some("disabled"), Some("claude-sonnet-5-5")));
    let bad = run(&["decide", "--config", &config], "not json", directory.path(), Some("s"));
    assert_eq!((bad["action"].as_str(), bad["code"].as_str()), (Some("none"), Some("malformed_event")));
}

#[test]
fn hook_stands_aside_only_in_the_mods_session_but_still_answers_controls() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path(), true);
    // A prompt the hook held earlier: "." releases it, unless the mod routes the session.
    let hold = || {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        std::fs::create_dir_all(directory.path().join("system1router")).unwrap();
        std::fs::write(directory.path().join("system1router/claude-pending.json"), serde_json::json!({ "s": { "prompt": "refactor the parser", "suggested": "opus", "current": null, "at_ms": now } }).to_string()).unwrap();
    };
    let resume = r#"{"session_id":"s","cwd":"/tmp","prompt":".","hook_event_name":"UserPromptSubmit"}"#;
    let hook = ["hook", "--host", "claude-code", "--config", &config];
    hold();
    assert_eq!(run(&hook, resume, directory.path(), Some("s")), serde_json::json!({}), "the mod's own session");
    for nested in [Some("another-session"), None] {
        hold();
        let released = run(&hook, resume, directory.path(), nested);
        assert!(released["hookSpecificOutput"]["additionalContext"].as_str().is_some_and(|text| text.ends_with("refactor the parser")), "{nested:?}: {released}");
    }
    let control = r#"{"session_id":"s","cwd":"/tmp","prompt":"/s1router_config","hook_event_name":"UserPromptSubmit"}"#;
    assert_eq!(run(&["hook", "--host", "claude-code", "--config", &config], control, directory.path(), Some("s"))["decision"], "block");
}

#[test]
fn choices_are_remembered_and_forgotten_and_the_panel_lists_them() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path(), true);
    let recorded = run(&["choice", "--config", &config, "--current", "claude-sonnet-5-5", "--suggested", "opus", "--variant", "high", "--accept", "yes", "--remember"], "", directory.path(), Some("s"));
    assert_eq!((recorded["recorded"].as_bool(), recorded["remembered"].as_bool()), (Some(true), Some(true)));
    let panel = run(&["panel", "--config", &config], "", directory.path(), Some("s"));
    assert_eq!(panel["status"]["enabled"], true);
    assert_eq!(panel["preferences"][0]["key"], "anthropic/claude-sonnet-5-5 → anthropic/opus (high)");
    assert_eq!(panel["preferences"][0]["accept"], true);
    assert_eq!(run(&["forget", "--config", &config], "", directory.path(), Some("s"))["forgotten"], 1);
    assert_eq!(run(&["panel", "--config", &config], "", directory.path(), Some("s"))["preferences"], serde_json::json!([]));
}

fn jev(class: &str) -> String {
    serde_json::json!({ "answers": {
        "routing_class": { "choice": class, "confidence": 0.9 },
        "task_type": { "choice": "debugging", "confidence": 0.9 },
        "context": { "score": 1.0, "confidence": 0.9 } } }).to_string()
}

#[test]
fn decide_swaps_only_the_effort_and_waits_while_the_cache_is_warm() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.json");
    std::fs::write(&config, r#"{"routing":{"enabled":true,"swap":"effort"},"providers":{"selected":"jev","jev":{"baseUrl":"http://127.0.0.1:9","credentialEnv":null}}}"#).unwrap();
    let config = config.to_str().unwrap();
    let decide = |payload: Value, class: &str| run_with(&["decide", "--config", config], &payload.to_string(), directory.path(), Some("s"), Some(&jev(class)));
    let payload = |effort: &str| serde_json::json!({ "session_id": "s", "cwd": "/tmp", "prompt": "fix the flaky test", "model": "claude-opus-5-5", "effort": effort, "hook_event_name": "UserPromptSubmit" });

    let asked = decide(payload("medium"), "code_hard");
    assert_eq!((asked["action"].as_str(), asked["kind"].as_str(), asked["effort"].as_str(), asked["currentEffort"].as_str()), (Some("ask"), Some("effort"), Some("xhigh"), Some("medium")), "{asked}");
    assert_eq!(asked["modelIds"], serde_json::json!(["claude-opus-5-5"]));

    let same = decide(payload("xhigh"), "code_hard");
    assert_eq!((same["action"].as_str(), same["code"].as_str(), same["routingClass"].as_str()), (Some("none"), Some("effort_unchanged"), Some("code_hard")), "{same}");

    // A long conversation keeps its effort rather than re-writing its cache for a lower one.
    let transcript = directory.path().join("s.jsonl");
    std::fs::write(&transcript, serde_json::json!({ "type": "assistant", "message": { "model": "claude-opus-5-5", "content": [], "usage": { "input_tokens": 10, "cache_read_input_tokens": 90_000, "cache_creation_input_tokens": 0 } } }).to_string()).unwrap();
    let mut long = payload("high");
    long["transcript_path"] = serde_json::json!(transcript);
    let held = decide(long.clone(), "question_simple");
    assert_eq!((held["action"].as_str(), held["code"].as_str(), held["effort"].as_str()), (Some("none"), Some("cache_sticky"), Some("low")), "{held}");
    long["idle_ms"] = serde_json::json!(600_000);
    assert_eq!(decide(long, "question_simple")["action"], "ask", "the cache has expired: nothing to lose");

    let recorded = run(&["choice", "--config", config, "--current", "claude-opus-5-5", "--current-variant", "high", "--suggested", "claude-opus-5-5", "--variant", "low", "--accept", "yes", "--remember"], "", directory.path(), Some("s"));
    assert_eq!(recorded["remembered"], true);
    let panel = run(&["panel", "--config", config], "", directory.path(), Some("s"));
    assert_eq!(panel["preferences"][0]["key"], "anthropic/claude-opus-5-5 (high) → anthropic/claude-opus-5-5 (low)");
    assert_eq!((panel["status"]["swap"].as_str(), panel["status"]["efforts"]["code_hard"].as_str()), (Some("effort"), Some("xhigh")));
}

#[test]
fn an_effort_applied_without_asking_is_counted_by_effort() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.json");
    std::fs::write(&config, r#"{"routing":{"enabled":true,"swap":"effort"},"consent":{"mode":"always-accept"},"providers":{"selected":"jev","jev":{"baseUrl":"http://127.0.0.1:9","credentialEnv":null}}}"#).unwrap();
    let cli = |arguments: &[&str], stdin: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_system1router")).args(arguments).env("XDG_STATE_HOME", directory.path()).env_remove("SYSTEM1ROUTER_TELEMETRY").env_remove("SYSTEM1ROUTER_TELEMETRY_DISABLED").env_remove("SYSTEM1ROUTER_CONSENT")
            .env("SYSTEM1ROUTER_JEV_RESPONSE", jev("question_simple")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    };
    let config = config.to_str().unwrap();
    let decided: Value = serde_json::from_str(&cli(&["decide", "--config", config], r#"{"session_id":"s","cwd":"/tmp","prompt":"what is a mutex","model":"claude-opus-5-5","effort":"high","hook_event_name":"UserPromptSubmit"}"#)).unwrap();
    assert_eq!((decided["action"].as_str(), decided["effort"].as_str()), (Some("switch"), Some("low")));
    assert!(cli(&["stats", "--config", config], "").contains("By effort: low 1."));
}
