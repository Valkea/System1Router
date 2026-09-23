use crate::protocol::{CapabilityHints, CapabilityLevel, DecisionOutcome, NormalizedDecision, NormalizedEvent, RequestPayload, BoundedContext, ModelCatalog, ModelRef, ModelState, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::{env, fs, path::{Path, PathBuf}};

pub const INSTALL_VERSION: &str = env!("CARGO_PKG_VERSION");
const OWNER_KEY: &str = "_system1router";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host { OpenCode, ClaudeCode, Codex, CopilotCli, CopilotVsCode, Generic }

impl Host {
    pub fn parse(value: &str) -> Option<Self> { match value { "opencode" => Some(Self::OpenCode), "claude" | "claude-code" => Some(Self::ClaudeCode), "codex" => Some(Self::Codex), "copilot" | "copilot-cli" => Some(Self::CopilotCli), "vscode" | "copilot-vscode" => Some(Self::CopilotVsCode), "generic" => Some(Self::Generic), _ => None } }
    pub fn as_str(self) -> &'static str { match self { Self::OpenCode => "opencode", Self::ClaudeCode => "claude-code", Self::Codex => "codex", Self::CopilotCli => "copilot-cli", Self::CopilotVsCode => "copilot-vscode", Self::Generic => "generic" } }
    pub fn capability(self) -> CapabilityLevel { match self { Self::OpenCode => CapabilityLevel::ConsentOnly, Self::ClaudeCode | Self::Codex => CapabilityLevel::ConsentOnly, Self::CopilotCli | Self::CopilotVsCode => CapabilityLevel::ObserveOnly, Self::Generic => CapabilityLevel::ObserveOnly } }
    pub fn api_version(self) -> &'static str { match self { Self::OpenCode => "@opencode-ai/plugin 1.4.3", Self::ClaudeCode => "UserPromptSubmit/PreModelSwitch documented hook contract", Self::Codex => "UserPromptSubmit/tool hook contract", Self::CopilotCli => "userPromptSubmitted/userPromptTransformed", Self::CopilotVsCode => "documented prompt/tool hook contract", Self::Generic => "system1router protocol v1" } }
}

pub fn normalize(host: Host, payload: Value, cwd: Option<String>) -> NormalizedEvent {
    let session_id = string_field(&payload, &["sessionId", "session_id", "conversationId", "conversation_id"]).unwrap_or_else(|| "unknown-session".into());
    let query = string_field(&payload, &["prompt", "userPrompt", "user_prompt", "input", "query", "text"]).unwrap_or_else(|| extract_text(&payload));
    let provider = string_field(&payload, &["provider", "providerId", "provider_id"]);
    let model = string_field(&payload, &["model", "modelId", "model_id"]);
    let variant = string_field(&payload, &["variant"]);
    let current_model = model.map(|model| ModelState { provider: provider.unwrap_or_else(|| "default".into()), model, variant, scope: Some(session_id.clone()) });
    let catalog = payload.get("models").and_then(Value::as_array).map(|models| ModelCatalog { models: models.iter().filter_map(|model| { Some(ModelRef { provider: string_field(model, &["provider", "providerId"]).unwrap_or_else(|| "default".into()), model: string_field(model, &["model", "modelId"])?, variants: model.get("variants").and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default() }) }).collect(), gateway_targets: Vec::new() }).unwrap_or_default();
    NormalizedEvent { protocol_version: string_field(&payload, &["protocolVersion", "protocol_version"]).unwrap_or_else(|| PROTOCOL_VERSION.into()), host: host.as_str().into(), event: string_field(&payload, &["event", "hookEventName", "type"]).unwrap_or_else(|| "user_prompt".into()), session_id, turn_id: string_field(&payload, &["turnId", "turn_id", "messageId", "message_id"]), cwd, request: RequestPayload { query, attachments: payload.get("attachments").and_then(Value::as_array).cloned().unwrap_or_default(), tools: payload.get("tools").and_then(Value::as_array).cloned().unwrap_or_default(), tool_arguments: payload.get("toolArguments").cloned().unwrap_or(Value::Null), original_payload: payload.clone() }, current_model, context: BoundedContext { conversation_summary: string_field(&payload, &["conversationSummary", "conversation_summary"]), project_summary: string_field(&payload, &["projectSummary", "project_summary"]), project_id: string_field(&payload, &["projectId", "project_id"]), repository: string_field(&payload, &["repository", "repo"]), branch: string_field(&payload, &["branch"]), files: payload.get("files").and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default(), tools: Vec::new(), metadata: Value::Null }, catalog, capabilities: CapabilityHints { level: host.capability(), model_mutation: false, interactive_consent: false, gateway: false, protocol_version: Some(PROTOCOL_VERSION.into()) }, correlation_id: string_field(&payload, &["correlationId", "correlation_id"]), metadata: json!({ "hostApiVersion": host.api_version() }) }
}

pub fn translate_decision(host: Host, decision: &NormalizedDecision) -> Value {
    let reason = decision.diagnostic.message.clone();
    let recommendation = decision.recommendation.as_ref().map(|item| format_recommendation(item));
    match host {
        Host::ClaudeCode => {
            let mut response = json!({ "decision": match decision.outcome { DecisionOutcome::Block => "block", DecisionOutcome::Ask => "ask", _ => "allow" }, "reason": reason });
            if let Some(text) = recommendation { response["hookSpecificOutput"] = json!({ "hookEventName": "UserPromptSubmit", "additionalContext": format!("System1Router recommendation (advisory): {text}") }); }
            response
        }
        Host::Codex => json!({ "decision": if decision.outcome == DecisionOutcome::Block { "block" } else { "allow" }, "reason": reason, "system1router": { "outcome": decision.outcome, "correlationId": decision.correlation_id } }),
        Host::CopilotCli => json!({ "userPromptTransformed": Value::Null, "decision": "allow", "reason": reason, "system1router": { "outcome": decision.outcome, "recommendation": recommendation } }),
        Host::CopilotVsCode => json!({ "decision": "allow", "reason": reason, "context": recommendation.map(|text| format!("System1Router recommendation (advisory): {text}")) }),
        Host::OpenCode | Host::Generic => json!({ "outcome": decision.outcome, "correlationId": decision.correlation_id, "diagnostic": reason, "recommendation": recommendation }),
    }
}

/// Claude Code's model-switch hook can allow or deny a host-initiated switch,
/// but it cannot initiate a switch from a user-prompt hook.
pub fn translate_claude_model_switch(decision: &NormalizedDecision) -> Value {
    json!({ "decision": if decision.outcome == DecisionOutcome::Block { "deny" } else { "allow" }, "reason": decision.diagnostic.message, "system1router": { "capability": "consent-only", "switchInitiated": false } })
}

/// Codex tool hooks may return documented tool decisions and updated input;
/// they are deliberately separate from model-request interception.
pub fn translate_codex_tool_decision(decision: &NormalizedDecision, updated_input: Option<Value>) -> Value {
    let mut result = json!({ "decision": if decision.outcome == DecisionOutcome::Block { "reject" } else { "allow" }, "reason": decision.diagnostic.message });
    if let Some(input) = updated_input { result["updatedInput"] = input; }
    result
}

fn format_recommendation(recommendation: &crate::protocol::Recommendation) -> String {
    if let Some(target) = &recommendation.gateway_target { return format!("use gateway target {} ({})", target.id, target.model); }
    format!("use {}{}", recommendation.model.as_deref().unwrap_or("configured model"), recommendation.variant.as_ref().map(|variant| format!(" variant {variant}")).unwrap_or_default())
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> { keys.iter().find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_owned)) }
fn extract_text(value: &Value) -> String { match value { Value::String(text) => text.clone(), Value::Array(items) => items.iter().map(extract_text).filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n"), Value::Object(object) => ["content", "text", "message"].iter().find_map(|key| object.get(*key).map(extract_text)).unwrap_or_default(), _ => String::new() } }

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

pub fn doctor(config: &crate::config::Config) -> Value {
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
