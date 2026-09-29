use crate::protocol::{CapabilityHints, CapabilityLevel, DecisionOutcome, NormalizedDecision, NormalizedEvent, RequestPayload, BoundedContext, ModelCatalog, ModelRef, ModelState, PROTOCOL_VERSION};
use serde_json::{json, Value};
pub use crate::host_install::{default_config_path, doctor, install, install_opencode, refresh_opencode_installations, uninstall, uninstall_opencode, OpenCodeScope, INSTALL_VERSION};

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
