//! Router controls shared by every host: toggle routing, status and usage
//! stats. OpenCode has its own TUI versions of these; Claude Code reaches
//! them through `/s1router_*` prompts intercepted by its hook.
use crate::config::{default_config_path, Config};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs, path::{Path, PathBuf}};

pub fn state_directory() -> Option<PathBuf> {
    if let Ok(base) = env::var("XDG_STATE_HOME") { return Some(PathBuf::from(base).join("system1router")); }
    #[cfg(windows)]
    { env::var("APPDATA").ok().map(|base| PathBuf::from(base).join("system1router")) }
    #[cfg(not(windows))]
    { env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local/state/system1router")) }
}

pub fn provider_log_path() -> Option<PathBuf> {
    env::var("SYSTEM1ROUTER_VERBOSE_LOG").ok().map(PathBuf::from).or_else(|| state_directory().map(|directory| directory.join("provider-debug.jsonl")))
}

/// Sets `routing.enabled` (or flips it when `enabled` is `None`) and returns
/// the new value. JSON keeps its key order; TOML is edited in place so its
/// comments and layout survive.
pub fn set_routing_enabled(path: &Path, enabled: Option<bool>) -> Result<bool, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("read {}: {error}; create the config first (see README)", path.display()))?;
    let (updated, value) = if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
        toggle_toml(&text, enabled)?
    } else {
        let mut root: Value = serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))?;
        let object = root.as_object_mut().ok_or("config root must be a JSON object")?;
        let routing = object.entry("routing").or_insert_with(|| Value::Object(Default::default())).as_object_mut().ok_or("`routing` must be an object")?;
        let value = enabled.unwrap_or_else(|| routing.get("enabled").and_then(Value::as_bool) != Some(true));
        routing.insert("enabled".into(), Value::Bool(value));
        (serde_json::to_string_pretty(&root).map_err(|error| error.to_string())? + "\n", value)
    };
    let temporary = path.with_extension(format!("s1r-{}.tmp", std::process::id()));
    fs::write(&temporary, updated).map_err(|error| format!("write {}: {error}", temporary.display()))?;
    #[cfg(unix)]
    if let Ok(metadata) = fs::metadata(path) { let _ = fs::set_permissions(&temporary, metadata.permissions()); }
    fs::rename(&temporary, path).map_err(|error| { let _ = fs::remove_file(&temporary); format!("replace {}: {error}", path.display()) })?;
    Ok(value)
}

fn toggle_toml(text: &str, enabled: Option<bool>) -> Result<(String, bool), String> {
    let parsed: toml::Value = toml::from_str(text).map_err(|error| format!("parse TOML config: {error}"))?;
    let current = parsed.get("routing").and_then(|routing| routing.get("enabled")).and_then(toml::Value::as_bool) == Some(true);
    let value = enabled.unwrap_or(!current);
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let section = lines.iter().position(|line| line.trim() == "[routing]");
    match section {
        Some(start) => {
            let end = lines.iter().skip(start + 1).position(|line| line.trim_start().starts_with('[')).map(|offset| start + 1 + offset).unwrap_or(lines.len());
            match (start + 1..end).find(|&index| lines[index].split('=').next().is_some_and(|key| key.trim() == "enabled")) {
                Some(index) => lines[index] = format!("enabled = {value}"),
                None => lines.insert(start + 1, format!("enabled = {value}")),
            }
        }
        None => { lines.push(String::new()); lines.push("[routing]".into()); lines.push(format!("enabled = {value}")); }
    }
    Ok((lines.join("\n") + "\n", value))
}

#[derive(Debug, Default, PartialEq)]
pub struct Stats {
    pub calls: u64,
    pub models: BTreeMap<String, u64>,
    /// Suggestions the user answered (OpenCode dialog or a remembered choice).
    pub answered: u64,
    pub accepted: u64,
    /// Advisory suggestions shown without a switch (Claude Code).
    pub advised: u64,
    pub by_host: BTreeMap<String, u64>,
}

/// Reads OpenCode's `usage.jsonl` (actual choices) and the core's
/// `events.jsonl` for other hosts. OpenCode rows in `events.jsonl` are
/// skipped because they are recorded before the user answers.
pub fn read_stats() -> Stats {
    let mut stats = Stats::default();
    let Some(directory) = state_directory() else { return stats; };
    let events = env::var("SYSTEM1ROUTER_TELEMETRY").map(PathBuf::from).unwrap_or_else(|_| directory.join("events.jsonl"));
    for (path, native) in [(directory.join("usage.jsonl"), false), (events, true)] {
        let Ok(text) = fs::read_to_string(path) else { continue; };
        for entry in text.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
            let host = entry.get("host").and_then(Value::as_str).unwrap_or("unknown");
            if native && host == "opencode" { continue; }
            // Held Claude Code prompts are counted when resolved (usage.jsonl).
            if native && entry.get("policyOutcome").and_then(Value::as_str) == Some("consent-requested") { continue; }
            if entry.get("policyOutcome").and_then(Value::as_str) == Some("disabled") { continue; }
            stats.calls += 1;
            *stats.by_host.entry(host.into()).or_default() += 1;
            *stats.models.entry(entry.get("model").and_then(Value::as_str).unwrap_or("unknown").into()).or_default() += 1;
            let suggested = entry.get("suggestedModel").and_then(Value::as_str).is_some();
            match entry.get("accepted").and_then(Value::as_bool) {
                Some(accepted) if suggested => { stats.answered += 1; if accepted { stats.accepted += 1; } }
                _ if suggested => stats.advised += 1,
                _ => {}
            }
        }
    }
    stats
}

pub fn format_stats(stats: &Stats) -> String {
    if stats.calls == 0 { return "No routed prompts recorded yet.".into(); }
    let mut models: Vec<_> = stats.models.iter().collect();
    models.sort_by(|a, b| b.1.cmp(a.1));
    let models = models.iter().map(|(model, count)| format!("{model} {count}")).collect::<Vec<_>>().join(", ");
    let hosts = stats.by_host.iter().map(|(host, count)| format!("{host} {count}")).collect::<Vec<_>>().join(", ");
    let rate = if stats.answered > 0 { format!("{:.0}%", 100.0 * stats.accepted as f64 / stats.answered as f64) } else { "—".into() };
    format!("Prompts: {} ({hosts}). By model: {models}. Answered suggestions: {} (accepted {}, {rate}). Advisory suggestions: {}.", stats.calls, stats.answered, stats.accepted, stats.advised)
}

pub fn status_text(config: &Config, path: Option<&Path>) -> String {
    let path = path.map(Path::to_path_buf).or_else(default_config_path);
    let summary = config.safe_summary();
    let provider_ready = match config.providers.selected.as_str() { "jev" => summary["jevConfigured"] == true, "laya" => summary["layaConfigured"] == true, _ => false };
    format!(
        "Routing {} · provider {}{} · {} rule(s) · config {} · provider log {}",
        if config.routing.enabled { "ON" } else { "OFF" },
        config.providers.selected,
        if provider_ready { "" } else { " (not configured)" },
        config.rules.len(),
        path.map(|path| path.display().to_string()).unwrap_or_else(|| "(none)".into()),
        provider_log_path().map(|path| path.display().to_string()).unwrap_or_else(|| "(none)".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_json_and_preserves_other_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        fs::write(&path, r#"{"providers":{"selected":"jev"},"routing":{"enabled":true,"failOpen":true}}"#).unwrap();
        assert!(!set_routing_enabled(&path, None).unwrap());
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().keys().collect::<Vec<_>>(), vec!["providers", "routing"]);
        assert_eq!(value["routing"], serde_json::json!({ "enabled": false, "failOpen": true }));
        assert!(set_routing_enabled(&path, Some(true)).unwrap());
        assert!(set_routing_enabled(&path, None).is_ok_and(|enabled| !enabled));
    }

    #[test]
    fn toggles_toml_in_place() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "# mine\n[routing]\nenabled = false # note\n\n[providers]\nselected = 'jev'\n").unwrap();
        assert!(set_routing_enabled(&path, None).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "# mine\n[routing]\nenabled = true\n\n[providers]\nselected = 'jev'\n");
        fs::write(&path, "[providers]\nselected = 'jev'\n").unwrap();
        assert!(set_routing_enabled(&path, None).unwrap());
        let parsed: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["routing"]["enabled"].as_bool(), Some(true));
        assert!(set_routing_enabled(&directory.path().join("missing.json"), None).is_err());
    }

    #[test]
    fn stats_combine_opencode_choices_and_advisory_hosts() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("system1router");
        fs::create_dir_all(&state).unwrap();
        fs::write(state.join("usage.jsonl"), [
            r#"{"host":"opencode","model":"p/a","suggestedModel":"p/a","accepted":true}"#,
            r#"{"host":"opencode","model":"p/b","suggestedModel":"p/a","accepted":false}"#,
            r#"{"host":"opencode","model":"p/b","policyOutcome":"disabled"}"#,
        ].join("\n")).unwrap();
        fs::write(state.join("events.jsonl"), [
            r#"{"host":"opencode","model":"p/b","policyOutcome":"consent-requested"}"#,
            r#"{"host":"claude-code","model":"anthropic/claude-opus-5-5","suggestedModel":"anthropic/haiku"}"#,
            "partial",
        ].join("\n")).unwrap();
        let previous = env::var_os("XDG_STATE_HOME");
        env::set_var("XDG_STATE_HOME", directory.path());
        env::remove_var("SYSTEM1ROUTER_TELEMETRY");
        let stats = read_stats();
        match previous { Some(value) => env::set_var("XDG_STATE_HOME", value), None => env::remove_var("XDG_STATE_HOME") }
        assert_eq!((stats.calls, stats.answered, stats.accepted, stats.advised), (3, 2, 1, 1));
        assert_eq!(stats.by_host["claude-code"], 1);
        assert!(format_stats(&stats).contains("accepted 1, 50%"));
    }
}
