use crate::{config::Config, protocol::{ConsentOutcome, DecisionOutcome, NormalizedDecision, NormalizedEvent}, redaction::safe_request_payload};
use std::{fs::{self, OpenOptions}, io::Write, path::PathBuf};

fn path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SYSTEM1ROUTER_TELEMETRY") { return Some(PathBuf::from(path)); }
    if let Ok(base) = std::env::var("XDG_STATE_HOME") { return Some(PathBuf::from(base).join("system1router/events.jsonl")); }
    #[cfg(windows)]
    { std::env::var("APPDATA").ok().map(|base| PathBuf::from(base).join("system1router/events.jsonl")) }
    #[cfg(not(windows))]
    { std::env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local/state/system1router/events.jsonl")) }
}

pub fn record(decision: &NormalizedDecision, request: &NormalizedEvent, config: &Config) {
    let Some(mut event) = decision.telemetry.clone() else { return; };
    if !config.telemetry.enabled || std::env::var("SYSTEM1ROUTER_TELEMETRY_DISABLED").is_ok() { return; }
    let Some(path) = path() else { return; };
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    let model_name = |provider: &str, model: &str| format!("{provider}/{model}");
    event.model = request.current_model.as_ref().map(|model| model_name(&model.provider, &model.model));
    event.suggested_model = decision.recommendation.as_ref().and_then(|rec| {
        rec.gateway_target.as_ref().map(|target| model_name("gateway", &target.model))
            .or_else(|| Some(model_name(rec.provider.as_deref()?, rec.model.as_deref()?)))
    });
    event.accepted = match decision.consent {
        Some(ConsentOutcome::UseSuggestedModel) => Some(true),
        Some(ConsentOutcome::KeepCurrentModel) => Some(false),
        _ => None,
    };
    // An advisory acceptance does not imply the host changed its model.
    if decision.outcome == DecisionOutcome::Route && event.accepted == Some(true) {
        event.model = event.suggested_model.clone().or(event.model);
    }
    if config.telemetry.debug_capture && std::env::var("SYSTEM1ROUTER_DEBUG_CAPTURE").ok().as_deref() == Some("1") { event.debug_capture = Some(safe_request_payload(&request.request, &config.redaction)); }
    if let Ok(line) = serde_json::to_string(&event) {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) { let _ = writeln!(file, "{line}"); }
    }
}

/// Appends a resolved suggestion to `usage.jsonl`, the file OpenCode also
/// writes, so stats count what the user actually chose. `effort` is the
/// effort the turn ran at, for an effort swap.
pub fn record_choice(config: &Config, host: &str, model: Option<&str>, suggested: &str, accepted: bool, effort: Option<&str>) {
    if !config.telemetry.enabled || std::env::var("SYSTEM1ROUTER_TELEMETRY_DISABLED").is_ok() { return; }
    let Some(directory) = crate::control::state_directory() else { return; };
    let _ = fs::create_dir_all(&directory);
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|value| value.as_millis() as u64).unwrap_or_default();
    let mut line = serde_json::json!({ "host": host, "model": model, "suggestedModel": suggested, "accepted": accepted, "timestampMs": timestamp });
    if let Some(effort) = effort { line["effort"] = effort.into(); }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(directory.join("usage.jsonl")) { let _ = writeln!(file, "{line}"); }
}
