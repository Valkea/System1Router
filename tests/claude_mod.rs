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

    // Claude Code keeps its cache across effort changes, so even a long conversation takes a lower effort.
    let transcript = directory.path().join("s.jsonl");
    std::fs::write(&transcript, serde_json::json!({ "type": "assistant", "message": { "model": "claude-opus-5-5", "content": [], "usage": { "input_tokens": 10, "cache_read_input_tokens": 90_000, "cache_creation_input_tokens": 0 } } }).to_string()).unwrap();
    let mut long = payload("high");
    long["transcript_path"] = serde_json::json!(transcript);
    let lower = decide(long, "question_simple");
    assert_eq!((lower["action"].as_str(), lower["effort"].as_str()), (Some("ask"), Some("low")), "{lower}");

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

#[test]
fn a_trial_logs_what_jev_saw_and_the_overrides_and_the_hook_stays_out() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.json");
    std::fs::write(&config, r#"{"routing":{"enabled":true,"swap":"effort","classSelection":"upper"},"consent":{"mode":"always-accept"},"telemetry":{"capturePrompts":true},"providers":{"selected":"jev","jev":{"baseUrl":"http://127.0.0.1:9","credentialEnv":null}}}"#).unwrap();
    let response = serde_json::json!({ "model": "jev-1.14.0", "answers": {
        "routing_class": { "choice": "code_simple", "confidence": 0.45, "probabilities": { "code_simple": 0.45, "code_moderate": 0.35, "code_hard": 0.2 } },
        "task_type": { "choice": "debugging", "confidence": 0.9 },
        "context": { "score": 2.0, "confidence": 0.9 } } }).to_string();
    let cli = |arguments: &[&str], stdin: &str, mod_session: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_system1router"));
        command.args(arguments).env("XDG_STATE_HOME", directory.path()).env_remove("SYSTEM1ROUTER_TELEMETRY").env_remove("SYSTEM1ROUTER_TELEMETRY_DISABLED").env_remove("SYSTEM1ROUTER_CONSENT")
            .env("SYSTEM1ROUTER_JEV_RESPONSE", &response).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        match mod_session { Some(session) => command.env("SYSTEM1ROUTER_CLAUDE_MOD", session), None => command.env_remove("SYSTEM1ROUTER_CLAUDE_MOD") };
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    };
    let config = config.to_str().unwrap();
    let transcript = directory.path().join("s.jsonl");
    std::fs::write(&transcript, serde_json::json!({ "type": "assistant", "message": { "model": "claude-opus-5-5", "content": [{ "type": "text", "text": "CI hangs in 3 of 50 runs: a child waits on stdin while the parent waits on stdout." }] } }).to_string()).unwrap();
    let payload = serde_json::json!({ "session_id": "s", "cwd": "/tmp", "prompt": "fix it", "model": "claude-opus-5-5", "effort": "medium", "transcript_path": transcript, "hook_event_name": "UserPromptSubmit" }).to_string();
    let decided: Value = serde_json::from_str(&cli(&["decide", "--config", config], &payload, Some("s"))).unwrap();
    assert_eq!((decided["action"].as_str(), decided["effort"].as_str(), decided["routingClass"].as_str()), (Some("switch"), Some("xhigh"), Some("code_hard")), "a fifth of the mass on code_hard: {decided}");
    let events = || std::fs::read_to_string(directory.path().join("system1router/events.jsonl")).unwrap_or_default();
    let logged: Value = serde_json::from_str(events().lines().last().unwrap()).unwrap();
    assert_eq!(logged["correlationId"], decided["correlationId"]);
    assert_eq!((logged["providerModel"].as_str(), logged["probabilities"]["code_hard"].as_f64().map(|value| (value * 100.0).round())), (Some("jev-1.14.0"), Some(20.0)));
    assert_eq!((logged["capture"]["query"].as_str(), logged["capture"]["currentEffort"].as_str()), (Some("fix it"), Some("medium")));
    assert!(logged["capture"]["context"].as_str().is_some_and(|context| context.contains("CI hangs")), "{logged}");

    assert_eq!(serde_json::from_str::<Value>(&cli(&["override", "--config", config, "--session", "s", "--model", "claude-opus-5-5", "--effort", "high", "--from", "xhigh"], "", None)).unwrap()["recorded"], true);
    assert!(cli(&["stats", "--config", config], "", None).contains("Manual effort overrides: 1."));

    // Without the mod, the hook cannot change effort, so it does not classify.
    let before = events().lines().count();
    assert_eq!(cli(&["hook", "--host", "claude-code", "--config", config], &payload, None).trim(), "{}");
    assert_eq!(events().lines().count(), before);
}

#[test]
fn ask_below_applies_confident_efforts_and_the_style_reaches_the_mod() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.json");
    std::fs::write(&config, r#"{"routing":{"enabled":true,"swap":"effort","classSelection":"argmax"},"consent":{"mode":"interactive","style":"menu","askBelow":0.8},"providers":{"selected":"jev","jev":{"baseUrl":"http://127.0.0.1:9","credentialEnv":null}}}"#).unwrap();
    let config = config.to_str().unwrap();
    let payload = r#"{"session_id":"s","cwd":"/tmp","prompt":"rename tmp","model":"claude-opus-5-5","effort":"high","hook_event_name":"UserPromptSubmit"}"#;
    let jev = |confidence: f64| serde_json::json!({ "answers": {
        "routing_class": { "choice": "code_simple", "confidence": confidence },
        "task_type": { "choice": "code-generation", "confidence": 0.9 },
        "context": { "score": 0.0, "confidence": 0.9 } } }).to_string();
    let sure = run_with(&["decide", "--config", config], payload, directory.path(), Some("s"), Some(&jev(0.9)));
    assert_eq!((sure["action"].as_str(), sure["effort"].as_str(), sure["style"].as_str()), (Some("switch"), Some("medium"), Some("menu")), "{sure}");
    let unsure = run_with(&["decide", "--config", config], payload, directory.path(), Some("s"), Some(&jev(0.6)));
    assert_eq!(unsure["action"], "ask", "{unsure}");
    assert!(unsure["correlationId"].as_str().is_some_and(|id| id.starts_with("s1r-")));
}

#[test]
fn consent_command_sets_the_dialog_and_the_panel_shows_it() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path(), true);
    let say = |arguments: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_system1router")).args(arguments).env("XDG_STATE_HOME", directory.path()).output().unwrap();
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(say(&["consent", "--config", &config, "menu", "--ask-below", "80%"]).trim(), "System1Router consent menu: pick the effort in a menu, asked only below 80% confidence.");
    let panel = run(&["panel", "--config", &config], "", directory.path(), None);
    assert_eq!((panel["status"]["consentMode"].as_str(), panel["status"]["consentStyle"].as_str(), panel["status"]["askBelow"].as_f64()), (Some("interactive"), Some("menu"), Some(0.8_f32 as f64)));
    assert!(say(&["consent", "--config", &config, "--ask-below", "nope"]).contains("--ask-below takes"));
    assert!(say(&["consent", "--config", &config]).contains("consent menu"));
}
