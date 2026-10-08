//! The CLI contract the Claude Code mod relies on (`claude-code/system1router`).
use serde_json::Value;
use std::{io::Write, path::Path, process::{Command, Stdio}};

/// Runs the CLI; `mod_session` is the session the mod marked, if any.
fn run(arguments: &[&str], stdin: &str, state: &Path, mod_session: Option<&str>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_system1router"));
    command.args(arguments).env("XDG_STATE_HOME", state).env("SYSTEM1ROUTER_TELEMETRY_DISABLED", "1").env_remove("SYSTEM1ROUTER_TELEMETRY")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
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
