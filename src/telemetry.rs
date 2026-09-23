use crate::{config::Config, protocol::{NormalizedEvent, RoutingEvent}, redaction::safe_request_payload};
use std::{fs::{self, OpenOptions}, io::Write, path::PathBuf};

fn path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SYSTEM1ROUTER_TELEMETRY") { return Some(PathBuf::from(path)); }
    std::env::var("XDG_STATE_HOME").ok().map(|base| PathBuf::from(base).join("system1router/events.jsonl")).or_else(|| std::env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local/state/system1router/events.jsonl")))
}

pub fn record(event: Option<&RoutingEvent>, request: &NormalizedEvent, config: &Config) {
    if event.is_none() || !config.telemetry.enabled || std::env::var("SYSTEM1ROUTER_TELEMETRY_DISABLED").is_ok() { return; }
    let Some(path) = path() else { return; };
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    let mut event = event.unwrap().clone();
    if config.telemetry.debug_capture && std::env::var("SYSTEM1ROUTER_DEBUG_CAPTURE").ok().as_deref() == Some("1") { event.debug_capture = Some(safe_request_payload(&request.request, &config.redaction)); }
    if let Ok(line) = serde_json::to_string(&event) {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) { let _ = writeln!(file, "{line}"); }
    }
    let _ = config;
}
