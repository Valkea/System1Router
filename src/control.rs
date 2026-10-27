//! Router controls shared by every host: toggle routing, status and usage
//! stats. OpenCode has its own TUI versions of these; Claude Code reaches
//! them through `/s1router_*` prompts intercepted by its hook.
use crate::config::{default_config_path, Config};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
/// the new value.
pub fn set_routing_enabled(path: &Path, enabled: Option<bool>) -> Result<bool, String> {
    let current = read_config_value(path, "routing", "enabled")?.and_then(|value| value.as_bool()) == Some(true);
    let value = enabled.unwrap_or(!current);
    set_config(path, "routing", &[("enabled", Value::Bool(value))])?;
    Ok(value)
}

/// How the person is asked: `auto` applies suggestions, `confirm` and `menu`
/// ask in that dialog. `ask_below` (`Some(None)` clears it) applies confident
/// suggestions without asking. Returns the resulting setting as a sentence.
pub fn set_consent(path: &Path, style: Option<&str>, ask_below: Option<Option<f32>>) -> Result<String, String> {
    let mut values = Vec::new();
    match style {
        Some("auto") => values.push(("mode", Value::from("always-accept"))),
        Some(style @ ("confirm" | "menu")) => { values.push(("mode", Value::from("interactive"))); values.push(("style", Value::from(style))); }
        Some(other) => return Err(format!("unknown consent style {other:?}: use auto, confirm or menu")),
        None => {}
    }
    if let Some(threshold) = ask_below {
        if threshold.is_some_and(|value| !(0.0..=1.0).contains(&value)) { return Err("the ask-below confidence must be between 0 and 1".into()); }
        values.push(("askBelow", threshold.map(|value| Value::from((f64::from(value) * 100.0).round() / 100.0)).unwrap_or(Value::Null)));
    }
    if !values.is_empty() { set_config(path, "consent", &values)?; }
    let mode = read_config_value(path, "consent", "mode")?.and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_else(|| "interactive".into());
    let style = read_config_value(path, "consent", "style")?.and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_else(|| "confirm".into());
    let below = read_config_value(path, "consent", "askBelow")?.and_then(|value| value.as_f64());
    Ok(match mode.as_str() {
        "always-accept" => "consent auto: suggestions apply without asking".into(),
        "interactive" => format!("consent {style}: {}{}", if style == "menu" { "pick the effort in a menu" } else { "use or keep each suggestion" }, below.map(|value| format!(", asked only below {:.0}% confidence", value * 100.0)).unwrap_or_default()),
        other => format!("consent {other}"),
    })
}

fn is_toml(path: &Path) -> bool { path.extension().and_then(|extension| extension.to_str()) == Some("toml") }

fn read_config_value(path: &Path, section: &str, key: &str) -> Result<Option<Value>, String> {
    // A first command (`toggle on`, `provider local`) creates a JSON config.
    if !path.exists() && !is_toml(path) { if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; } fs::write(path, "{}\n").map_err(|error| format!("write {}: {error}", path.display()))?; }
    let text = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let root: Value = if is_toml(path) {
        let parsed: toml::Value = toml::from_str(&text).map_err(|error| format!("parse TOML config: {error}"))?;
        serde_json::to_value(parsed).map_err(|error| error.to_string())?
    } else { serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))? };
    Ok(root.get(section).and_then(|section| section.get(key)).cloned())
}

/// Sets `section.key` for each pair (null removes it). JSON keeps its key
/// order; TOML is edited in place so its comments and layout survive. The
/// file is replaced atomically.
pub fn set_config(path: &Path, section: &str, values: &[(&str, Value)]) -> Result<(), String> {
    // A first command (`toggle on`, `provider local`) creates a JSON config.
    if !path.exists() && !is_toml(path) { if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; } fs::write(path, "{}\n").map_err(|error| format!("write {}: {error}", path.display()))?; }
    let text = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let updated = if is_toml(path) {
        toml::from_str::<toml::Value>(&text).map_err(|error| format!("parse TOML config: {error}"))?;
        values.iter().fold(text, |text, (key, value)| set_toml(&text, section, key, value))
    } else {
        let mut root: Value = serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))?;
        let object = root.as_object_mut().ok_or("config root must be a JSON object")?;
        let table = object.entry(section).or_insert_with(|| Value::Object(Default::default())).as_object_mut().ok_or_else(|| format!("`{section}` must be an object"))?;
        for (key, value) in values {
            if value.is_null() { table.remove(*key); } else { table.insert((*key).into(), value.clone()); }
        }
        serde_json::to_string_pretty(&root).map_err(|error| error.to_string())? + "\n"
    };
    let temporary = path.with_extension(format!("s1r-{}.tmp", std::process::id()));
    fs::write(&temporary, updated).map_err(|error| format!("write {}: {error}", temporary.display()))?;
    #[cfg(unix)]
    if let Ok(metadata) = fs::metadata(path) { let _ = fs::set_permissions(&temporary, metadata.permissions()); }
    fs::rename(&temporary, path).map_err(|error| { let _ = fs::remove_file(&temporary); format!("replace {}: {error}", path.display()) })
}

fn set_toml(text: &str, section: &str, key: &str, value: &Value) -> String {
    let line = (!value.is_null()).then(|| format!("{key} = {value}"));
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = format!("[{section}]");
    match lines.iter().position(|candidate| candidate.trim() == header) {
        Some(start) => {
            let end = lines.iter().skip(start + 1).position(|candidate| candidate.trim_start().starts_with('[')).map(|offset| start + 1 + offset).unwrap_or(lines.len());
            match ((start + 1..end).find(|&index| lines[index].split('=').next().is_some_and(|name| name.trim() == key)), line) {
                (Some(index), Some(line)) => lines[index] = line,
                (Some(index), None) => { lines.remove(index); }
                (None, Some(line)) => lines.insert(start + 1, line),
                (None, None) => {}
            }
        }
        None => if let Some(line) = line { lines.push(String::new()); lines.push(header); lines.push(line); },
    }
    lines.join("\n") + "\n"
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
    /// Effort swaps by the effort the turn ran at.
    pub efforts: BTreeMap<String, u64>,
    /// Manual effort changes over the router's (`/effort`).
    pub overrides: u64,
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
            if entry.get("kind").and_then(Value::as_str) == Some("override") { stats.overrides += 1; continue; }
            if native && host == "opencode" { continue; }
            // Held Claude Code prompts are counted when resolved (usage.jsonl).
            if native && entry.get("policyOutcome").and_then(Value::as_str) == Some("consent-requested") { continue; }
            if entry.get("policyOutcome").and_then(Value::as_str) == Some("disabled") { continue; }
            stats.calls += 1;
            *stats.by_host.entry(host.into()).or_default() += 1;
            *stats.models.entry(entry.get("model").and_then(Value::as_str).unwrap_or("unknown").into()).or_default() += 1;
            if let Some(effort) = entry.get("effort").and_then(Value::as_str) { *stats.efforts.entry(effort.into()).or_default() += 1; }
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
    let efforts = if stats.efforts.is_empty() { String::new() } else { format!(" By effort: {}.", stats.efforts.iter().map(|(effort, count)| format!("{effort} {count}")).collect::<Vec<_>>().join(", ")) };
    let overrides = if stats.overrides > 0 { format!(" Manual effort overrides: {}.", stats.overrides) } else { String::new() };
    format!("Prompts: {} ({hosts}). By model: {models}.{efforts} Answered suggestions: {} (accepted {}, {rate}). Advisory suggestions: {}.{overrides}", stats.calls, stats.answered, stats.accepted, stats.advised)
}

fn provider_ready(config: &Config) -> bool {
    let summary = config.safe_summary();
    match config.providers.selected.as_str() { "jev" => summary["jevConfigured"] == true, "laya" => summary["layaConfigured"] == true, "local" => summary["localInstalled"] == true, _ => false }
}

/// A classifier model's family, as the UIs name it: `jev-1.14.0` is Jev,
/// `ettin-32m-effort-v1` is Ettin.
pub fn classifier_family(model: &str) -> String {
    let family = model.split(['-', '_', '/']).next().unwrap_or(model);
    let mut letters = family.chars();
    letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
}

/// What classifies prompts: `Jev (jev-1.14.0)`, `Ettin (local model ettin-32m-effort-v1)`.
pub fn classifier_label(config: &Config) -> String {
    let providers = &config.providers;
    match providers.selected.as_str() {
        "jev" => providers.jev.model.as_deref().map_or("Jev".into(), |model| format!("Jev ({model})")),
        "local" => format!("{} (local model {})", classifier_family(&providers.local.model), providers.local.model),
        "laya" => "Laya".into(),
        other => other.into(),
    }
}

pub fn status_text(config: &Config, path: Option<&Path>) -> String {
    let path = path.map(Path::to_path_buf).or_else(default_config_path);
    format!(
        "Routing {} · classifier {}{} · {} rule(s) · config {} · provider log {}",
        if config.routing.enabled { "ON" } else { "OFF" },
        classifier_label(config),
        if provider_ready(config) { "" } else { " (not configured)" },
        config.rules.len(),
        path.map(|path| path.display().to_string()).unwrap_or_else(|| "(none)".into()),
        provider_log_path().map(|path| path.display().to_string()).unwrap_or_else(|| "(none)".into()),
    )
}

/// Everything the Claude Code mod's dashboard shows, in one call.
pub fn panel(config: &Config, path: Option<&Path>) -> Value {
    let path = path.map(Path::to_path_buf).or_else(default_config_path);
    let stats = read_stats();
    json!({
        "status": {
            "enabled": config.routing.enabled,
            "provider": config.providers.selected,
            "classifier": if config.providers.selected == "local" { classifier_family(&config.providers.local.model) } else { classifier_family(&config.providers.selected) },
            "classifierLabel": classifier_label(config),
            "providerReady": provider_ready(config),
            "rules": config.rules.len(),
            "claudeCodeRules": config.rules.iter().filter(|rule| rule.host.as_deref() == Some("claude-code")).count(),
            "consentMode": config.consent.mode,
            "consentStyle": config.consent.style,
            "askBelow": config.consent.ask_below,
            "swap": config.routing.swap,
            "efforts": crate::protocol::RoutingClass::ALL.iter().map(|class| (class.as_str(), config.effort.level(class))).collect::<BTreeMap<_, _>>(),
            "configPath": path,
            "providerLog": provider_log_path(),
        },
        "stats": { "calls": stats.calls, "models": stats.models, "answered": stats.answered, "accepted": stats.accepted, "advised": stats.advised, "byHost": stats.by_host, "efforts": stats.efforts, "overrides": stats.overrides },
        "preferences": read_preferences(),
    })
}

// ---------------------------------------------------------------------------
// Remembered consent answers, shared with OpenCode's dialog
// (`.opencode/plugins/system1router/shared.ts` writes the same file).

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preference {
    pub key: String,
    pub accept: bool,
    pub current: String,
    pub suggested: String,
    #[serde(default)]
    pub created_at: u64,
}

/// A model as OpenCode labels it: `provider/model (variant)`.
pub fn model_label(provider: &str, model: &str, variant: Option<&str>) -> String {
    format!("{provider}/{model}{}", variant.map(|variant| format!(" ({variant})")).unwrap_or_default())
}

pub fn preference_key(current: &str, suggested: &str) -> String { format!("{current} → {suggested}") }

fn preferences_path() -> Option<PathBuf> { state_directory().map(|directory| directory.join("consent-preferences.json")) }

pub fn read_preferences() -> Vec<Preference> { preferences_path().map(|path| read_preferences_at(&path)).unwrap_or_default() }

fn read_preferences_at(path: &Path) -> Vec<Preference> {
    let Ok(text) = fs::read_to_string(path) else { return Vec::new(); };
    let Ok(value) = serde_json::from_str::<Value>(&text) else { return Vec::new(); };
    // Skip malformed entries rather than losing every remembered answer.
    value.get("preferences").and_then(Value::as_array).into_iter().flatten().filter_map(|item| serde_json::from_value(item.clone()).ok()).collect()
}

pub fn remembered(key: &str) -> Option<bool> { read_preferences().into_iter().find(|item| item.key == key).map(|item| item.accept) }

pub fn remember(current: &str, suggested: &str, accept: bool) -> Result<(), String> {
    remember_at(&preferences_path().ok_or("no state directory")?, current, suggested, accept)
}

fn remember_at(path: &Path, current: &str, suggested: &str, accept: bool) -> Result<(), String> {
    let key = preference_key(current, suggested);
    let mut preferences = read_preferences_at(path);
    preferences.retain(|item| item.key != key);
    let created_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|value| value.as_millis() as u64).unwrap_or_default();
    preferences.push(Preference { key, accept, current: current.into(), suggested: suggested.into(), created_at });
    write_preferences(path, &preferences)
}

/// Forgets one remembered answer by key, or all of them; returns how many.
pub fn forget(key: Option<&str>) -> Result<usize, String> { forget_at(&preferences_path().ok_or("no state directory")?, key) }

fn forget_at(path: &Path, key: Option<&str>) -> Result<usize, String> {
    let mut preferences = read_preferences_at(path);
    let before = preferences.len();
    match key { Some(key) => preferences.retain(|item| item.key != key), None => preferences.clear() }
    if preferences.len() != before { write_preferences(path, &preferences)?; }
    Ok(before - preferences.len())
}

fn write_preferences(path: &Path, preferences: &[Preference]) -> Result<(), String> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; }
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(&json!({ "preferences": preferences })).map_err(|error| error.to_string())?;
    fs::write(&temporary, text).map_err(|error| format!("write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| { let _ = fs::remove_file(&temporary); format!("replace {}: {error}", path.display()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_classifier_is_named_for_the_uis() {
        assert_eq!((classifier_family("jev-1.14.0"), classifier_family("ettin-32m-effort-v1"), classifier_family("")), ("Jev".into(), "Ettin".into(), String::new()));
        let mut config = Config::default();
        config.providers.selected = "jev".into();
        assert_eq!(classifier_label(&config), "Jev");
        config.providers.jev.model = Some("jev-1.14.0".into());
        assert_eq!(classifier_label(&config), "Jev (jev-1.14.0)");
        config.providers.selected = "local".into();
        assert_eq!(classifier_label(&config), "Ettin (local model ettin-32m-effort-v1)");
        assert!(status_text(&config, None).contains("classifier Ettin (local model ettin-32m-effort-v1)"));
        assert_eq!(panel(&config, None)["status"]["classifier"], "Ettin");
    }

    #[test]
    fn consent_styles_edit_json_and_toml() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        fs::write(&path, r#"{"routing":{"enabled":true},"consent":{"mode":"always-accept","declineOnTimeout":true}}"#).unwrap();
        assert_eq!(set_consent(&path, Some("menu"), Some(Some(0.8))).unwrap(), "consent menu: pick the effort in a menu, asked only below 80% confidence");
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["consent"], serde_json::json!({ "mode": "interactive", "declineOnTimeout": true, "style": "menu", "askBelow": 0.8 }));
        assert_eq!(set_consent(&path, None, Some(None)).unwrap(), "consent menu: pick the effort in a menu");
        assert_eq!(set_consent(&path, Some("auto"), None).unwrap(), "consent auto: suggestions apply without asking");
        assert!(set_consent(&path, Some("loud"), None).is_err() && set_consent(&path, None, Some(Some(1.5))).is_err());
        serde_json::from_str::<crate::config::Config>(&fs::read_to_string(&path).unwrap()).unwrap().validate().unwrap();
        let toml_path = directory.path().join("config.toml");
        fs::write(&toml_path, "# mine\n[routing]\nenabled = true\n").unwrap();
        set_consent(&toml_path, Some("confirm"), Some(Some(0.7))).unwrap();
        let text = fs::read_to_string(&toml_path).unwrap();
        assert!(text.starts_with("# mine\n[routing]\nenabled = true\n") && text.contains("[consent]") && text.contains("style = \"confirm\"") && text.contains("askBelow = 0.7"), "{text}");
        assert_eq!(toml::from_str::<crate::config::Config>(&text).unwrap().consent.ask_below, Some(0.7));
    }

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
        let missing = directory.path().join("new/config.json");
        assert!(set_routing_enabled(&missing, Some(true)).unwrap());
        assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&missing).unwrap()).unwrap(), serde_json::json!({ "routing": { "enabled": true } }));
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
            r#"{"host":"claude-code","model":"anthropic/claude-opus-5-5","suggestedModel":"anthropic/claude-opus-5-5","accepted":true,"effort":"low"}"#,
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
        assert_eq!((stats.calls, stats.answered, stats.accepted, stats.advised), (4, 3, 2, 1));
        assert_eq!(stats.by_host["claude-code"], 2);
        assert_eq!(stats.efforts["low"], 1);
        assert!(format_stats(&stats).contains("By effort: low 1. Answered suggestions: 3 (accepted 2, 67%)"));
    }

    #[test]
    fn preferences_round_trip_in_the_opencode_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("consent-preferences.json");
        // As OpenCode's shared.ts writes it, plus one malformed entry.
        fs::write(&path, r#"{"preferences":[{"key":"openai/gpt-5 → openai/gpt-5 (high)","accept":true,"current":"openai/gpt-5","suggested":"openai/gpt-5 (high)","createdAt":1},{"key":7}]}"#).unwrap();
        assert_eq!(read_preferences_at(&path).len(), 1);
        let current = model_label("anthropic", "claude-sonnet-5-5", None);
        let suggested = model_label("anthropic", "opus", Some("high"));
        assert_eq!(suggested, "anthropic/opus (high)");
        remember_at(&path, &current, &suggested, false).unwrap();
        remember_at(&path, &current, &suggested, true).unwrap();
        let preferences = read_preferences_at(&path);
        assert_eq!(preferences.len(), 2, "answering again replaces the pair");
        assert_eq!((preferences[1].key.as_str(), preferences[1].accept), ("anthropic/claude-sonnet-5-5 → anthropic/opus (high)", true));
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(written["preferences"][1]["createdAt"].as_u64().unwrap() > 0, "camelCase keys, as OpenCode reads them");
        assert_eq!(forget_at(&path, Some("missing")).unwrap(), 0);
        assert_eq!(forget_at(&path, Some(&preference_key(&current, &suggested))).unwrap(), 1);
        assert_eq!(forget_at(&path, None).unwrap(), 1);
        assert!(read_preferences_at(&path).is_empty());
    }
}
