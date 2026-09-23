//! Host-specific installation is separate from request translation. New
//! assistants can add an installer without changing the routing state machine.
use crate::{adapters::Host, config::Config, protocol::PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::{env, fs, path::{Path, PathBuf}};

pub const INSTALL_VERSION: &str = env!("CARGO_PKG_VERSION");
const OWNER_KEY: &str = "_system1router";

pub fn default_config_path(host: Host) -> PathBuf {
    if let Ok(path) = env::var("SYSTEM1ROUTER_HOST_CONFIG") { return PathBuf::from(path); }
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    match host { Host::ClaudeCode => PathBuf::from(home).join(".claude/settings.json"), Host::Codex => PathBuf::from(home).join(".codex/config.json"), Host::CopilotCli => PathBuf::from(home).join(".config/github-copilot/hooks.json"), Host::CopilotVsCode => PathBuf::from(home).join(".config/system1router/copilot-vscode.json"), Host::OpenCode => PathBuf::from(home).join(".config/opencode/opencode.json"), Host::Generic => PathBuf::from(home).join(".config/system1router/generic-hook.json") }
}

pub fn install(host: Host, config_path: Option<&Path>, executable: Option<&str>) -> Result<Value, String> {
    if host == Host::OpenCode { return Ok(json!({ "host": host.as_str(), "status": "use plugin entry", "version": INSTALL_VERSION })); }
    let path = config_path.map(PathBuf::from).unwrap_or_else(|| default_config_path(host));
    let mut root: Value = if path.exists() { serde_json::from_str(&fs::read_to_string(&path).map_err(|e| format!("read host config: {e}"))?).map_err(|e| format!("parse host config: {e}"))? } else { json!({}) };
    let command = executable.unwrap_or("system1router");
    let entry = json!({ "type": "command", "command": format!("{command} hook --host {}", host.as_str()), OWNER_KEY: { "version": INSTALL_VERSION, "protocolVersion": PROTOCOL_VERSION, "hash": installation_hash(host, command) } });
    let array = match host { Host::ClaudeCode | Host::Codex => root.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!({})).as_object_mut().unwrap().entry("UserPromptSubmit").or_insert_with(|| json!([])).as_array_mut().ok_or("host UserPromptSubmit hooks must be an array")?, _ => root.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!([])).as_array_mut().ok_or("host hooks must be an array")? };
    if !array.iter().any(|item| item.get(OWNER_KEY).is_some() || item.get("hooks").and_then(Value::as_array).is_some_and(|items| items.iter().any(|hook| hook.get(OWNER_KEY).is_some()))) { array.push(entry); }
    write_json(&path, &root)?;
    Ok(json!({ "host": host.as_str(), "status": "installed", "path": path, "version": INSTALL_VERSION, "protocolVersion": PROTOCOL_VERSION }))
}

pub fn uninstall(host: Host, config_path: Option<&Path>) -> Result<Value, String> {
    if host == Host::OpenCode { return Ok(json!({ "host": host.as_str(), "status": "remove plugin entry" })); }
    let path = config_path.map(PathBuf::from).unwrap_or_else(|| default_config_path(host));
    if !path.exists() { return Ok(json!({ "host": host.as_str(), "status": "not-installed", "path": path })); }
    let mut root: Value = serde_json::from_str(&fs::read_to_string(&path).map_err(|e| format!("read host config: {e}"))?).map_err(|e| format!("parse host config: {e}"))?;
    if let Some(array) = match host { Host::ClaudeCode | Host::Codex => root.get_mut("hooks").and_then(|hooks| hooks.get_mut("UserPromptSubmit")), _ => root.get_mut("hooks") }.and_then(Value::as_array_mut) { array.retain(|item| item.get(OWNER_KEY).is_none() && !item.get("hooks").and_then(Value::as_array).is_some_and(|items| items.iter().any(|hook| hook.get(OWNER_KEY).is_some()))); }
    write_json(&path, &root)?;
    Ok(json!({ "host": host.as_str(), "status": "uninstalled", "path": path }))
}

pub fn doctor(config: &Config) -> Value {
    let hosts = [Host::OpenCode, Host::ClaudeCode, Host::Codex, Host::CopilotCli, Host::CopilotVsCode, Host::Generic].into_iter().map(|host| json!({ "host": host.as_str(), "apiVersion": host.api_version(), "capability": host.capability(), "configPath": default_config_path(host), "installed": is_owned_installation(&default_config_path(host)) })).collect::<Vec<_>>();
    json!({ "version": INSTALL_VERSION, "protocolVersion": PROTOCOL_VERSION, "hosts": hosts, "config": config.safe_summary() })
}

fn installation_hash(host: Host, command: &str) -> String { let mut hash = 2_166_136_261_u64; for byte in format!("{}:{command}:{INSTALL_VERSION}", host.as_str()).bytes() { hash = (hash ^ u64::from(byte)).wrapping_mul(16_777_619); } format!("{hash:016x}") }
fn is_owned_installation(path: &Path) -> bool { let Ok(contents) = fs::read_to_string(path) else { return false; }; let Ok(value) = serde_json::from_str::<Value>(&contents) else { return false; }; contents.contains(OWNER_KEY) || value.get(OWNER_KEY).is_some() }

fn write_json(path: &Path, value: &Value) -> Result<(), String> { if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| format!("create host config directory: {e}"))?; } let bytes = serde_json::to_vec_pretty(value).map_err(|e| format!("serialize host config: {e}"))?; fs::write(path, bytes).map_err(|e| format!("write host config: {e}")) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn installation_is_idempotent_and_preserves_fields() { let directory = tempfile::tempdir().unwrap(); let path = directory.path().join("settings.json"); fs::write(&path, r#"{"other":true}"#).unwrap(); install(Host::ClaudeCode, Some(&path), Some("router")).unwrap(); install(Host::ClaudeCode, Some(&path), Some("router")).unwrap(); let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap(); assert_eq!(value["other"], true); assert_eq!(value["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1); }
}
