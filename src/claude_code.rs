//! Claude Code adapter: a `UserPromptSubmit` command hook.
//!
//! Claude Code hooks cannot change the model of the request being submitted,
//! so this adapter is advisory (consent-only). It classifies the prompt with
//! context read from the session transcript and shows the user a one-line
//! recommendation with the `/model` command to apply it. Nothing is added to
//! the model's context, so the hook costs no tokens in the conversation.
use crate::{
    adapters::Host,
    protocol::{BoundedContext, CapabilityHints, DecisionOutcome, ModelState, NormalizedDecision, NormalizedEvent, RequestPayload, PROTOCOL_VERSION},
};
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub const HOOK_EVENT: &str = "UserPromptSubmit";
const HOOK_TIMEOUT_SECONDS: u64 = 10;
const TRANSCRIPT_TAIL_BYTES: u64 = 1024 * 1024;
const SUMMARY_CHARS: usize = 3_000;

#[derive(Debug, Default, PartialEq)]
pub struct TranscriptContext {
    pub summary: Option<String>,
    pub files: Vec<String>,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub context_tokens: Option<u64>,
}

pub fn normalize(payload: Value, fallback_cwd: Option<String>) -> NormalizedEvent {
    let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
    let session_id = text("session_id").unwrap_or_else(|| "unknown-session".into());
    let cwd = text("cwd").or(fallback_cwd);
    let prompt = text("prompt").unwrap_or_default();
    // Slash commands are handled by Claude Code itself; there is nothing to route.
    let query = if prompt.trim_start().starts_with('/') { String::new() } else { prompt };
    let transcript = text("transcript_path").map(|path| read_transcript(Path::new(&path), cwd.as_deref())).unwrap_or_default();
    let cwd_for_settings = cwd.clone();
    NormalizedEvent {
        protocol_version: PROTOCOL_VERSION.into(),
        host: Host::ClaudeCode.as_str().into(),
        event: text("hook_event_name").unwrap_or_else(|| HOOK_EVENT.into()),
        session_id: session_id.clone(),
        turn_id: None,
        cwd,
        request: RequestPayload { query, ..Default::default() },
        // The mod knows the session's model. A hook does not: the transcript
        // shows the model actually answering, and before the first reply the
        // configured model is the best guess.
        // The mod also knows the effort its last request was sent with.
        current_model: text("model").filter(|model| !model.trim().is_empty()).or(transcript.model.clone()).or_else(|| configured_model(cwd_for_settings.as_deref())).map(|model| ModelState { provider: "anthropic".into(), model, variant: text("effort").filter(|effort| !effort.trim().is_empty()), scope: Some(session_id) }),
        context: BoundedContext {
            conversation_summary: transcript.summary,
            files: transcript.files,
            tools: transcript.tools,
            context_tokens: transcript.context_tokens,
            idle_ms: payload.get("idle_ms").and_then(Value::as_u64),
            ..Default::default()
        },
        catalog: Default::default(),
        // Consent is asked by holding the prompt and resuming with ".".
        capabilities: CapabilityHints { level: Host::ClaudeCode.capability(), interactive_consent: true, protocol_version: Some(PROTOCOL_VERSION.into()), ..Default::default() },
        correlation_id: None,
        // Claude Code sends a changed effort as a per-turn system message too
        // (`per-turn-control` beta): the cache survives it (experiments/RESULTS.md).
        metadata: json!({ "hostApiVersion": Host::ClaudeCode.api_version(), "hostProvider": "anthropic", "effortKeepsCache": true }),
    }
}

/// Reads the end of a Claude Code JSONL transcript. Only the tail is read so
/// long sessions do not slow the hook down.
pub fn read_transcript(path: &Path, cwd: Option<&str>) -> TranscriptContext {
    let Ok(mut file) = fs::File::open(path) else { return TranscriptContext::default(); };
    let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let start = length.saturating_sub(TRANSCRIPT_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() { return TranscriptContext::default(); }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() { return TranscriptContext::default(); }
    let text = String::from_utf8_lossy(&bytes);
    // A tail read usually starts mid-line; drop the partial first line.
    let body = if start > 0 { text.split_once('\n').map(|(_, rest)| rest).unwrap_or("") } else { &text };
    parse_transcript(body, cwd)
}

pub fn parse_transcript(body: &str, cwd: Option<&str>) -> TranscriptContext {
    let clean = |value: &str| match cwd { Some(root) if !root.is_empty() => value.replace(root, "."), _ => value.to_string() };
    let mut context = TranscriptContext::default();
    let mut blocks: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut tools: Vec<String> = Vec::new();
    for line in body.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue; };
        if entry.get("isSidechain").and_then(Value::as_bool) == Some(true) || entry.get("isMeta").and_then(Value::as_bool) == Some(true) { continue; }
        let kind = entry.get("type").and_then(Value::as_str).unwrap_or_default();
        let Some(message) = entry.get("message") else { continue; };
        match kind {
            "assistant" => {
                if let Some(model) = message.get("model").and_then(Value::as_str).filter(|model| model.starts_with("claude")) { context.model = Some(model.into()); }
                if let Some(usage) = message.get("usage") {
                    let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
                    let total = count("input_tokens") + count("cache_read_input_tokens") + count("cache_creation_input_tokens");
                    if total > 0 { context.context_tokens = Some(total); }
                }
                let mut parts = Vec::new();
                for block in message.get("content").and_then(Value::as_array).into_iter().flatten() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => if let Some(text) = block.get("text").and_then(Value::as_str).filter(|text| !text.trim().is_empty()) { parts.push(clip(&clean(text), 900)); },
                        Some("tool_use") => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                            tools.push(name.into());
                            let input = block.get("input").cloned().unwrap_or(Value::Null);
                            for key in ["file_path", "path", "notebook_path"] {
                                if let Some(file) = input.get(key).and_then(Value::as_str) { files.push(display_path(file, cwd)); }
                            }
                            parts.push(format!("Tool {name}: {}", clip(&clean(&input.to_string()), 180)));
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() { blocks.push(format!("Assistant:\n{}", parts.join("\n"))); }
            }
            "user" => {
                let content = message.get("content");
                if let Some(text) = content.and_then(Value::as_str) {
                    if !text.starts_with('<') && !text.trim().is_empty() { blocks.push(format!("User:\n{}", clip(&clean(text), 900))); }
                    continue;
                }
                let mut parts = Vec::new();
                for block in content.and_then(Value::as_array).into_iter().flatten() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => if let Some(text) = block.get("text").and_then(Value::as_str).filter(|text| !text.starts_with('<') && !text.trim().is_empty()) { parts.push(clip(&clean(text), 900)); },
                        Some("tool_result") => {
                            let output = match block.get("content") {
                                Some(Value::String(text)) => text.clone(),
                                Some(Value::Array(items)) => items.iter().filter_map(|item| item.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
                                _ => String::new(),
                            };
                            if !output.trim().is_empty() { parts.push(format!("Tool result:\n{}", clip(&clean(&output), 500))); }
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() { blocks.push(format!("User:\n{}", parts.join("\n"))); }
            }
            _ => {}
        }
    }
    let summary = keep_recent(&blocks.join("\n\n"), SUMMARY_CHARS);
    if !summary.is_empty() { context.summary = Some(summary); }
    context.files = recent_unique(files, 8);
    context.tools = recent_unique(tools, 10);
    context
}

/// The model Claude Code is configured to use: `ANTHROPIC_MODEL`, then the
/// `model` key of project local, project and user settings (`/model` writes
/// the user settings).
pub fn configured_model(cwd: Option<&str>) -> Option<String> {
    if let Some(model) = env::var("ANTHROPIC_MODEL").ok().filter(|model| !model.trim().is_empty()) { return Some(model); }
    let mut candidates = Vec::new();
    if let Some(cwd) = cwd { let root = Path::new(cwd); candidates.push(root.join(".claude/settings.local.json")); candidates.push(root.join(".claude/settings.json")); }
    candidates.push(settings_path(true, Path::new(".")));
    candidates.into_iter().find_map(|path| read_settings(&path).ok()?.get("model")?.as_str().map(str::to_owned))
}

// ---------------------------------------------------------------------------
// Asking before the request is sent
//
// A hook cannot change the model of the prompt being submitted, so when a
// different model is suggested the prompt is held (blocked, nothing is sent)
// and remembered for the session. The user optionally runs `/model <name>`
// and sends "." (or the same prompt again); the hook then lets it through,
// passing the held prompt as context, without classifying it again.

pub const RESUME: &str = ".";
const PENDING_TTL_MS: u64 = 30 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    pub prompt: String,
    pub suggested: String,
    pub current: Option<String>,
    pub at_ms: u64,
}

fn now_ms() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|value| value.as_millis() as u64).unwrap_or_default() }

fn pending_path() -> Option<PathBuf> { crate::control::state_directory().map(|directory| directory.join("claude-pending.json")) }

fn load_pending() -> std::collections::BTreeMap<String, Pending> {
    pending_path().and_then(|path| fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn save_pending(mut pending: std::collections::BTreeMap<String, Pending>) {
    let Some(path) = pending_path() else { return; };
    let now = now_ms();
    pending.retain(|_, item| now.saturating_sub(item.at_ms) < PENDING_TTL_MS);
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    if let Ok(bytes) = serde_json::to_vec(&pending) { let _ = fs::write(path, bytes); }
}

pub fn hold(session: &str, prompt: &str, suggested: &str, current: Option<&str>) {
    let mut pending = load_pending();
    pending.insert(session.into(), Pending { prompt: prompt.into(), suggested: suggested.into(), current: current.map(str::to_owned), at_ms: now_ms() });
    save_pending(pending);
}

/// Removes and returns the session's held prompt, if still fresh.
pub fn take_held(session: &str) -> Option<Pending> {
    let mut pending = load_pending();
    let item = pending.remove(session)?;
    save_pending(pending);
    (now_ms().saturating_sub(item.at_ms) < PENDING_TTL_MS).then_some(item)
}

/// Output for a prompt that resumes a held one, or `None` for a new prompt.
pub fn resume(held: &Pending, prompt: &str) -> Option<Value> {
    let prompt = prompt.trim();
    if prompt == held.prompt.trim() { return Some(json!({})); }
    if prompt != RESUME { return None; }
    Some(json!({ "hookSpecificOutput": { "hookEventName": HOOK_EVENT, "additionalContext": format!(
        "System1Router held the user's previous message while they chose a model. Their \".\" means: continue with that message now. Respond to it exactly as if they had just sent it:\n\n{}", held.prompt) } }))
}

/// Holds the prompt and asks, when the decision suggests another model.
pub fn ask(decision: &NormalizedDecision, event: &NormalizedEvent) -> Option<Value> {
    if decision.outcome == DecisionOutcome::Block { return None; }
    let recommendation = decision.recommendation.as_ref()?;
    let model = recommendation.model.as_deref()?;
    let current = event.current_model.as_ref().map(|state| state.model.as_str());
    if current.is_some_and(|current| same_model(current, model)) { return None; }
    hold(&event.session_id, &event.request.query, model, current);
    let percent = (recommendation.confidence.routing_class * 100.0).round();
    let current_label = current.unwrap_or("the current model");
    Some(json!({ "decision": "block", "reason": format!(
        "S1R · {} ({percent:.0}%) suggests {model} instead of {current_label}. {}\n  → switch: /model {model} then send {RESUME}\n  → keep {current_label}: send {RESUME}\nYour message is held and will be sent as is; nothing was sent yet.",
        recommendation.routing_class, recommendation.reason) }))
}

// ---------------------------------------------------------------------------
// The Claude Code mod
//
// The System1Router mod (function hooks, `claude-code/system1router`) switches
// the model of the turn itself, so it asks in a native dialog and only needs
// the decision. While it runs it sets MOD_ENV to its session id, and the
// command hook of that session leaves routing to it so a prompt is never
// classified twice. Matching the id keeps a nested `claude` without the mod,
// which inherits the variable, routed by its own hook.

pub const MOD_ENV: &str = "SYSTEM1ROUTER_CLAUDE_MOD";

pub fn mod_active(session_id: &str) -> bool { !session_id.is_empty() && env::var(MOD_ENV).is_ok_and(|value| value == session_id) }

/// The session's transcript under `<config dir>/projects/*/`, for a caller
/// that knows only the session id.
pub fn find_transcript(session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id.contains("..") { return None; }
    let base = env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude"));
    let name = format!("{session_id}.jsonl");
    fs::read_dir(base.join("projects")).ok()?.flatten().map(|entry| entry.path().join(&name)).find(|path| path.is_file())
}

/// Claude Code sends a turn's model as given, so a rule's alias (`opus`) must
/// become ids the API knows. Claude Code's own alias settings
/// (`ANTHROPIC_DEFAULT_OPUS_MODEL`, ...) win; otherwise the newest ids of the
/// family among the price table and the current model's generation, newest
/// first: the price table can lag a release, and a generation guess can name
/// a model that does not exist, so the mod tries them in order.
pub fn model_ids(target: &str, current: Option<&str>, table: Option<&crate::prices::PriceTable>) -> Vec<String> {
    if target.starts_with("claude-") || target.is_empty() || !target.chars().all(|c| c.is_ascii_lowercase()) { return vec![target.into()]; }
    if let Some(model) = env::var(format!("ANTHROPIC_DEFAULT_{}_MODEL", target.to_ascii_uppercase())).ok().filter(|model| !model.trim().is_empty()) { return vec![model]; }
    let prefix = format!("claude-{target}-");
    // `5-5` and `5` are versions; a dated id (`4-5-20251001`) or a variant is not.
    let version = |id: &str| id.strip_prefix(&prefix)?.split('-').map(|part| part.parse::<u32>().ok().filter(|value| *value < 1000)).collect::<Option<Vec<_>>>().filter(|parts| parts.len() <= 2);
    let generation = current.and_then(|current| current.split('[').next()?.strip_prefix("claude-")?.split_once('-').map(|(_, version)| format!("{prefix}{version}")));
    let known = table.into_iter().flat_map(|table| table.iter().filter(|(_, entry)| entry.provider == "anthropic").map(|(name, _)| name.clone()));
    let mut ids: Vec<(Vec<u32>, String)> = known.chain(generation).filter_map(|id| Some((version(&id)?, id))).collect();
    ids.sort_by(|a, b| b.cmp(a));
    ids.dedup_by(|a, b| a.1 == b.1);
    let ids: Vec<String> = ids.into_iter().map(|(_, id)| id).take(3).collect();
    if ids.is_empty() { vec![target.into()] } else { ids }
}

/// The decision as the mod acts on it. `action` is `ask` (show the dialog),
/// `switch` or `keep` (decided without asking: a remembered answer or a
/// non-interactive consent mode) or `none` (nothing to choose). `record` says
/// the mod must report the outcome with `system1router choice`, because the
/// core left the answer to the host.
///
/// `kind` is `effort` when the suggestion keeps the session's model and only
/// changes its effort (`routing.swap: "effort"`), `model` otherwise.
pub fn mod_decision(decision: &NormalizedDecision, event: &NormalizedEvent, table: Option<&crate::prices::PriceTable>, remembered: impl Fn(&str) -> Option<bool>) -> Value {
    let current = event.current_model.as_ref().map(|state| state.model.as_str());
    let current_effort = event.current_model.as_ref().and_then(|state| state.variant.as_deref());
    let mut output = json!({ "action": "none", "record": false, "current": current, "currentEffort": current_effort, "code": decision.diagnostic.code, "message": decision.diagnostic.message, "correlationId": decision.correlation_id });
    // A class that changes nothing (same effort, a warm cache) is still shown.
    if let Some(class) = decision.telemetry.as_ref().and_then(|telemetry| telemetry.routing_class.as_ref()) { output["routingClass"] = json!(class); }
    if let Some(effort) = decision.diagnostic.details.get("suggestedEffort") { output["effort"] = effort.clone(); }
    let Some(recommendation) = decision.recommendation.as_ref().filter(|_| decision.outcome != DecisionOutcome::Block) else { return output; };
    let Some(model) = recommendation.model.as_deref() else { return output; };
    output["routingClass"] = json!(recommendation.routing_class);
    output["confidence"] = json!(recommendation.confidence.routing_class);
    output["reason"] = json!(recommendation.reason);
    let variant = recommendation.variant.as_deref();
    let effort = current.is_some_and(|current| same_model(current, model));
    if effort && (variant.is_none() || variant == current_effort) { return output; }
    output["kind"] = json!(if effort { "effort" } else { "model" });
    output["effort"] = json!(variant);
    // The current effort only tells efforts apart; a model's label has none.
    let current_label = crate::control::model_label("anthropic", current.unwrap_or("default"), current_effort.filter(|_| effort));
    // Claude Code only runs Anthropic models; `choice` labels them the same way.
    let suggested_label = crate::control::model_label("anthropic", model, variant);
    let (action, record, remembered) = if decision.outcome == DecisionOutcome::Ask {
        match remembered(&crate::control::preference_key(&current_label, &suggested_label)) {
            Some(true) => ("switch", true, Some(true)),
            Some(false) => ("keep", true, Some(false)),
            None => ("ask", true, None),
        }
    } else if decision.consent == Some(crate::protocol::ConsentOutcome::UseSuggestedModel) {
        ("switch", false, None)
    } else {
        ("keep", false, None)
    };
    output["action"] = json!(action);
    output["record"] = json!(record);
    output["remembered"] = json!(remembered);
    output["model"] = json!(model);
    output["modelIds"] = json!(if effort { vec![current.unwrap_or(model).to_owned()] } else { model_ids(model, current, table) });
    output["variant"] = json!(variant);
    output["currentLabel"] = json!(current_label);
    output["suggestedLabel"] = json!(suggested_label);
    output
}

/// Router controls typed as prompts. The hook answers them itself and blocks
/// the prompt, so they never reach (or cost) a model turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control { Toggle(Option<bool>), Status, Stats }

pub fn control_command(prompt: &str) -> Option<Control> {
    let mut words = prompt.split_whitespace();
    let command = words.next()?;
    let argument = words.next();
    match command {
        "/s1router_toggle" | "/s1router_toogle" => Some(Control::Toggle(match argument { Some("on") => Some(true), Some("off") => Some(false), _ => None })),
        "/s1router_config" | "/s1router_status" => Some(Control::Status),
        "/s1router_stats" => Some(Control::Stats),
        _ => None,
    }
}

/// A blocked prompt shows `reason` to the user and skips the model.
pub fn control_response(message: &str) -> Value { json!({ "decision": "block", "reason": message }) }

const COMMAND_MARKER: &str = "<!-- Installed by system1router; managed file. -->";

/// Slash-command files make the controls discoverable in Claude Code's `/`
/// menu. The hook answers them first and blocks the prompt. The body must not
/// use `!` execution: Claude Code runs that while expanding the command,
/// before the hook, so the action would run twice. It is only an instruction
/// for the model in case the hook is not installed.
fn command_files(executable: &str) -> Vec<(&'static str, String)> {
    let executable = if executable.contains(' ') { format!("\"{executable}\"") } else { executable.to_string() };
    [
        ("s1router_toggle.md", "Toggle System1Router routing on/off (add on or off to force)", "toggle $ARGUMENTS"),
        ("s1router_config.md", "Show System1Router status, config and log paths", "status"),
        ("s1router_stats.md", "Show System1Router calls per model and suggestion acceptance", "stats"),
    ].into_iter().map(|(name, description, arguments)| {
        let command = format!("{executable} {arguments}");
        let allowed = format!("{executable} {}", arguments.split(' ').next().unwrap_or_default());
        (name, format!("---\ndescription: {description}\nallowed-tools: Bash({allowed}:*)\n---\n{COMMAND_MARKER}\nRun `{command}` with the Bash tool and reply with its output only.\n"))
    }).collect()
}

pub fn install_commands(directory: &Path, executable: &str) -> Result<usize, String> {
    let mut written = 0;
    for (name, contents) in command_files(executable) {
        let path = directory.join(name);
        if let Ok(existing) = fs::read_to_string(&path) {
            if !existing.contains(COMMAND_MARKER) { return Err(format!("refusing to overwrite {}, which was not installed by system1router", path.display())); }
            if existing == contents { continue; }
        }
        fs::create_dir_all(directory).map_err(|error| format!("create {}: {error}", directory.display()))?;
        fs::write(&path, contents).map_err(|error| format!("write {}: {error}", path.display()))?;
        written += 1;
    }
    Ok(written)
}

pub fn uninstall_commands(directory: &Path) -> usize {
    command_files("system1router").into_iter().map(|(name, _)| directory.join(name))
        .filter(|path| fs::read_to_string(path).is_ok_and(|text| text.contains(COMMAND_MARKER)) && fs::remove_file(path).is_ok())
        .count()
}

pub fn commands_directory(settings: &Path) -> PathBuf { settings.parent().unwrap_or(Path::new(".")).join("commands") }

/// Claude Code only accepts documented fields. A block is the only decision a
/// prompt hook can make; advice goes to the user as a `systemMessage`.
pub fn translate(decision: &NormalizedDecision, current: Option<&str>) -> Value {
    if decision.outcome == DecisionOutcome::Block {
        return json!({ "decision": "block", "reason": decision.diagnostic.message });
    }
    let Some(recommendation) = &decision.recommendation else { return json!({}); };
    let Some(model) = recommendation.model.as_deref() else { return json!({}); };
    if current.is_some_and(|current| same_model(current, model)) { return json!({}); }
    let variant = recommendation.variant.as_deref().map(|variant| format!(" ({variant})")).unwrap_or_default();
    let percent = (recommendation.confidence.routing_class * 100.0).round();
    let message = format!(
        "S1R · {} ({percent:.0}%) → {model}{variant}{}. Run /model {model} to switch; {}",
        recommendation.routing_class,
        current.map(|current| format!(" instead of {current}")).unwrap_or_default(),
        recommendation.reason,
    );
    json!({ "systemMessage": message })
}

/// Accepts aliases: a rule targeting `opus` matches `claude-opus-5-5`.
pub fn same_model(current: &str, target: &str) -> bool {
    current == target || current.split(['-', '.', '[']).any(|part| part == target)
}

fn clip(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit { return value.to_string(); }
    value.chars().take(limit).collect::<String>() + "…[truncated]"
}

fn keep_recent(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    if count <= limit { return value.to_string(); }
    format!("[Earlier conversation omitted]\n{}", value.chars().skip(count - (limit - 30)).collect::<String>())
}

fn recent_unique(values: Vec<String>, limit: usize) -> Vec<String> {
    let mut output: Vec<String> = Vec::new();
    for value in values.into_iter().rev() {
        if output.len() >= limit { break; }
        if !value.is_empty() && !output.contains(&value) { output.push(value); }
    }
    output.reverse();
    output
}

fn display_path(value: &str, cwd: Option<&str>) -> String {
    let normalized = value.replace('\\', "/");
    if let Some(root) = cwd.map(|root| root.replace('\\', "/")).filter(|root| !root.is_empty()) {
        if let Some(relative) = normalized.strip_prefix(&format!("{}/", root.trim_end_matches('/'))) { return relative.into(); }
    }
    if normalized.starts_with('/') || normalized.chars().nth(1) == Some(':') { return normalized.rsplit('/').next().unwrap_or_default().into(); }
    normalized
}

// ---------------------------------------------------------------------------
// Installation

/// Settings file for a scope. Project installs go to `settings.local.json`,
/// which Claude Code keeps out of version control, because the hook calls a
/// locally installed executable.
pub fn settings_path(global: bool, project_root: &Path) -> PathBuf {
    if global {
        let base = env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude"));
        base.join("settings.json")
    } else {
        project_root.join(".claude").join("settings.local.json")
    }
}

fn home() -> PathBuf { env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| ".".into()) }

pub fn hook_command(executable: &str) -> String {
    let executable = if executable.contains(' ') { format!("\"{executable}\"") } else { executable.to_string() };
    format!("{executable} hook --host claude-code")
}

fn is_owned(hook: &Value) -> bool {
    hook.get("command").and_then(Value::as_str).is_some_and(|command| command.contains("system1router") && command.ends_with("hook --host claude-code"))
}

pub fn install(path: &Path, executable: &str) -> Result<Value, String> {
    install_commands(&commands_directory(path), executable)?;
    let mut root = read_settings(path)?;
    let object = root.as_object_mut().ok_or("Claude Code settings must be a JSON object")?;
    let hooks = object.entry("hooks").or_insert_with(|| json!({})).as_object_mut().ok_or("Claude Code `hooks` must be an object")?;
    let groups = hooks.entry(HOOK_EVENT).or_insert_with(|| json!([])).as_array_mut().ok_or("Claude Code `hooks.UserPromptSubmit` must be an array")?;
    let command = hook_command(executable);
    let mut status = "installed";
    let mut found = false;
    for group in groups.iter_mut() {
        for hook in group.get_mut("hooks").and_then(Value::as_array_mut).into_iter().flatten() {
            if !is_owned(hook) { continue; }
            found = true;
            if hook["command"] != command { hook["command"] = json!(command); status = "updated"; } else { status = "already-installed"; }
        }
    }
    if !found { groups.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECONDS }] })); }
    if status != "already-installed" { write_settings(path, &root)?; }
    Ok(json!({ "host": "claude-code", "status": status, "path": path, "command": command }))
}

pub fn uninstall(path: &Path) -> Result<Value, String> {
    let commands = uninstall_commands(&commands_directory(path));
    if !path.exists() && commands == 0 { return Ok(json!({ "host": "claude-code", "status": "not-installed", "path": path })); }
    let mut root = read_settings(path)?;
    let mut removed = false;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        if let Some(groups) = hooks.get_mut(HOOK_EVENT).and_then(Value::as_array_mut) {
            for group in groups.iter_mut() {
                if let Some(items) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                    let before = items.len();
                    items.retain(|hook| !is_owned(hook));
                    removed |= items.len() != before;
                }
            }
            groups.retain(|group| group.get("hooks").and_then(Value::as_array).is_none_or(|items| !items.is_empty()));
            if groups.is_empty() { hooks.remove(HOOK_EVENT); }
        }
        if hooks.is_empty() { root.as_object_mut().map(|object| object.remove("hooks")); }
    }
    if !removed { return Ok(json!({ "host": "claude-code", "status": if commands > 0 { "uninstalled" } else { "not-installed" }, "path": path })); }
    write_settings(path, &root)?;
    Ok(json!({ "host": "claude-code", "status": "uninstalled", "path": path }))
}

pub fn is_installed(path: &Path) -> bool {
    read_settings(path).ok().and_then(|root| root.get("hooks")?.get(HOOK_EVENT)?.as_array().cloned()).is_some_and(|groups| {
        groups.iter().any(|group| group.get("hooks").and_then(Value::as_array).is_some_and(|items| items.iter().any(is_owned)))
    })
}

fn read_settings(path: &Path) -> Result<Value, String> {
    if !path.exists() { return Ok(json!({})); }
    let text = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if text.trim().is_empty() { return Ok(json!({})); }
    serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}; fix the file or add the hook manually", path.display()))
}

fn write_settings(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; }
    let temporary = path.with_extension(format!("json.s1r-{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())? + "\n";
    fs::write(&temporary, text).map_err(|error| format!("write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| { let _ = fs::remove_file(&temporary); format!("replace {}: {error}", path.display()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::*;

    const TRANSCRIPT: &str = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>ignored</local-command-caveat>"}}
{"type":"user","message":{"role":"user","content":"Why does /work/app/src/parser.rs fail?"}}
{"type":"assistant","message":{"model":"claude-sonnet-5-5","role":"assistant","content":[{"type":"text","text":"Let me look."},{"type":"tool_use","name":"Read","input":{"file_path":"/work/app/src/parser.rs"}}],"usage":{"input_tokens":3,"cache_read_input_tokens":20000,"cache_creation_input_tokens":1000}}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"fn parse() {}"}]}}
{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-4-5","role":"assistant","content":[{"type":"text","text":"subagent"}]}}
not json
"#;

    #[test]
    fn transcript_gives_context_model_and_tokens() {
        let context = parse_transcript(TRANSCRIPT, Some("/work/app"));
        assert_eq!(context.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(context.context_tokens, Some(21_003));
        assert_eq!(context.files, vec!["src/parser.rs"]);
        assert_eq!(context.tools, vec!["Read"]);
        let summary = context.summary.unwrap();
        assert!(summary.contains("Why does ./src/parser.rs fail?"));
        assert!(summary.contains("Tool result:\nfn parse() {}"));
        assert!(!summary.contains("subagent") && !summary.contains("local-command"));
    }

    #[test]
    fn tail_read_skips_the_partial_first_line() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("t.jsonl");
        let filler = format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{}\"}}}}\n", "x".repeat(TRANSCRIPT_TAIL_BYTES as usize));
        fs::write(&path, filler + TRANSCRIPT).unwrap();
        assert_eq!(read_transcript(&path, None).model.as_deref(), Some("claude-sonnet-5-5"));
    }

    #[test]
    fn slash_commands_are_not_classified() {
        let event = normalize(json!({ "session_id": "s", "prompt": "/model opus", "hook_event_name": "UserPromptSubmit" }), None);
        assert!(event.request.query.is_empty());
        assert_eq!(event.capabilities.level, CapabilityLevel::ConsentOnly);
    }

    fn decision(outcome: DecisionOutcome, model: Option<&str>) -> NormalizedDecision {
        NormalizedDecision {
            protocol_version: "1".into(), correlation_id: "c".into(), outcome, consent: None, capability: CapabilityLevel::ConsentOnly,
            diagnostic: Diagnostic::info("x", "reason text"), telemetry: None,
            recommendation: model.map(|model| Recommendation { rule_id: "r".into(), mode: RoutingMode::Advisory, provider: Some("anthropic".into()), model: Some(model.into()), variant: None, gateway_target: None, reason: "hard work".into(), routing_class: RoutingClass::CodeHard, task_type: "debugging".into(), confidence: ClassificationConfidence { routing_class: 0.83, task_type: 1.0, context: 1.0 } }),
        }
    }

    #[test]
    fn output_uses_only_documented_fields() {
        let current = Some("claude-sonnet-5-5");
        let advice = translate(&decision(DecisionOutcome::Ask, Some("opus")), current);
        assert_eq!(advice.as_object().unwrap().keys().collect::<Vec<_>>(), vec!["systemMessage"]);
        let message = advice["systemMessage"].as_str().unwrap();
        assert!(message.contains("code_hard (83%) → opus instead of claude-sonnet-5-5") && message.contains("/model opus"), "{message}");
        assert_eq!(translate(&decision(DecisionOutcome::Ask, Some("sonnet")), current), json!({}));
        assert_eq!(translate(&decision(DecisionOutcome::Bypass, None), current), json!({}));
        assert_eq!(translate(&decision(DecisionOutcome::Block, None), current)["decision"], "block");
    }

    #[test]
    fn install_uses_the_documented_schema_and_preserves_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, r#"{"model":"opus","hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"other"}]}],"Stop":[]},"zeta":1}"#).unwrap();
        assert_eq!(install(&path, "/opt/bin/system1router").unwrap()["status"], "installed");
        assert_eq!(install(&path, "/opt/bin/system1router").unwrap()["status"], "already-installed");
        assert_eq!(install(&path, "/usr/bin/system1router").unwrap()["status"], "updated");
        let text = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value.as_object().unwrap().keys().collect::<Vec<_>>(), vec!["model", "hooks", "zeta"], "key order is preserved");
        let groups = value["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[1], json!({ "hooks": [{ "type": "command", "command": "/usr/bin/system1router hook --host claude-code", "timeout": 10 }] }));
        assert!(is_installed(&path));
        assert_eq!(uninstall(&path).unwrap()["status"], "uninstalled");
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["hooks"], json!({ "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "other" }] }], "Stop": [] }));
        assert!(!is_installed(&path));
        assert_eq!(uninstall(&path).unwrap()["status"], "not-installed");
    }

    #[test]
    fn uninstall_removes_empty_containers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.local.json");
        install(&path, "system1router").unwrap();
        uninstall(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().trim(), "{}");
    }

    #[test]
    fn control_prompts_are_recognised() {
        assert_eq!(control_command("/s1router_toggle"), Some(Control::Toggle(None)));
        assert_eq!(control_command("/s1router_toogle off"), Some(Control::Toggle(Some(false))));
        assert_eq!(control_command("  /s1router_stats "), Some(Control::Stats));
        assert_eq!(control_command("/s1router_config"), Some(Control::Status));
        assert_eq!(control_command("explain /s1router_toggle"), None);
        assert_eq!(control_command("/s1router_togglex"), None);
    }

    #[test]
    fn commands_are_installed_with_the_hook_and_removed_with_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let commands = directory.path().join("commands");
        fs::create_dir_all(&commands).unwrap();
        fs::write(commands.join("mine.md"), "user command").unwrap();
        install(&path, "/opt/bin/system1router").unwrap();
        let toggle = fs::read_to_string(commands.join("s1router_toggle.md")).unwrap();
        assert!(toggle.starts_with("---\ndescription: Toggle System1Router"));
        assert!(toggle.contains("allowed-tools: Bash(/opt/bin/system1router toggle:*)") && toggle.contains("Run `/opt/bin/system1router toggle $ARGUMENTS`") && !toggle.contains("!`"));
        uninstall(&path).unwrap();
        assert!(!commands.join("s1router_toggle.md").exists() && !commands.join("s1router_stats.md").exists());
        assert!(commands.join("mine.md").exists());
        fs::write(commands.join("s1router_stats.md"), "user-owned").unwrap();
        assert!(install(&path, "system1router").is_err(), "never overwrite a user's file");
    }

    #[test]
    fn mod_decision_says_what_to_do() {
        let mut event = normalize(json!({ "session_id": "s", "prompt": "refactor the parser", "model": "claude-sonnet-5-5" }), None);
        assert_eq!(event.current_model.as_ref().unwrap().model, "claude-sonnet-5-5", "the mod's model wins over the transcript and settings");
        let none = |_: &str| None;
        let asked = mod_decision(&decision(DecisionOutcome::Ask, Some("opus")), &event, None, none);
        assert_eq!((asked["action"].as_str(), asked["record"].as_bool(), asked["model"].as_str()), (Some("ask"), Some(true), Some("opus")));
        assert_eq!((asked["routingClass"].as_str(), asked["current"].as_str()), (Some("code_hard"), Some("claude-sonnet-5-5")));
        let key = asked["currentLabel"].as_str().unwrap().to_owned() + " → " + asked["suggestedLabel"].as_str().unwrap();
        assert_eq!(key, "anthropic/claude-sonnet-5-5 → anthropic/opus");
        let always = mod_decision(&decision(DecisionOutcome::Ask, Some("opus")), &event, None, |asked: &str| (asked == key).then_some(true));
        assert_eq!((always["action"].as_str(), always["remembered"].as_bool()), (Some("switch"), Some(true)));
        let never = mod_decision(&decision(DecisionOutcome::Ask, Some("opus")), &event, None, |_: &str| Some(false));
        assert_eq!(never["action"], "keep");
        let mut auto = decision(DecisionOutcome::Allow, Some("opus"));
        auto.consent = Some(ConsentOutcome::UseSuggestedModel);
        let switched = mod_decision(&auto, &event, None, none);
        assert_eq!((switched["action"].as_str(), switched["record"].as_bool()), (Some("switch"), Some(false)), "the core already recorded it");
        assert_eq!(mod_decision(&decision(DecisionOutcome::Bypass, None), &event, None, none)["action"], "none");
        assert_eq!(mod_decision(&decision(DecisionOutcome::Block, Some("opus")), &event, None, none)["action"], "none");
        event.current_model.as_mut().unwrap().model = "claude-opus-5-5".into();
        let same = mod_decision(&decision(DecisionOutcome::Ask, Some("opus")), &event, None, none);
        assert_eq!((same["action"].as_str(), same["routingClass"].as_str()), (Some("none"), Some("code_hard")), "already on it, but the class is still shown");
    }

    #[test]
    fn mod_decision_tells_an_effort_swap_from_a_model_swap() {
        let event = normalize(json!({ "session_id": "s", "prompt": "fix the typo", "model": "claude-opus-5-5", "effort": "high", "idle_ms": 1200 }), None);
        assert_eq!((event.current_model.as_ref().unwrap().variant.as_deref(), event.context.idle_ms), (Some("high"), Some(1200)));
        assert_eq!(event.metadata["effortKeepsCache"], true);
        let none = |_: &str| None;
        let mut lower = decision(DecisionOutcome::Ask, Some("claude-opus-5-5"));
        lower.recommendation.as_mut().unwrap().variant = Some("low".into());
        let asked = mod_decision(&lower, &event, None, none);
        assert_eq!((asked["action"].as_str(), asked["kind"].as_str(), asked["effort"].as_str(), asked["currentEffort"].as_str()), (Some("ask"), Some("effort"), Some("low"), Some("high")));
        assert_eq!(asked["modelIds"], json!(["claude-opus-5-5"]), "an effort swap never resolves another model");
        assert_eq!(asked["currentLabel"].as_str().unwrap().to_owned() + " → " + asked["suggestedLabel"].as_str().unwrap(), "anthropic/claude-opus-5-5 (high) → anthropic/claude-opus-5-5 (low)");
        lower.recommendation.as_mut().unwrap().variant = Some("high".into());
        assert_eq!(mod_decision(&lower, &event, None, none)["action"], "none", "already at that effort");
        let model = mod_decision(&decision(DecisionOutcome::Ask, Some("sonnet")), &event, None, none);
        assert_eq!((model["kind"].as_str(), model["currentLabel"].as_str()), (Some("model"), Some("anthropic/claude-opus-5-5")));
        let mut sticky = decision(DecisionOutcome::Allow, None);
        sticky.diagnostic = Diagnostic::info("cache_sticky", "keeping high effort reuses 80000 cached context tokens");
        sticky.diagnostic.details = json!({ "effort": "high", "suggestedEffort": "low" });
        sticky.telemetry = Some(crate::protocol::RoutingEvent { correlation_id: "c".into(), host: "claude-code".into(), routing_class: Some(RoutingClass::CodeSimple), task_type: None, mode: RoutingMode::Advisory, policy_outcome: "cache-sticky".into(), latency_ms: 0, error_category: None, model: None, suggested_model: None, accepted: None, effort: None, probabilities: None, provider_model: None, capture: None, debug_capture: None });
        let held = mod_decision(&sticky, &event, None, none);
        assert_eq!((held["action"].as_str(), held["code"].as_str(), held["routingClass"].as_str(), held["effort"].as_str()), (Some("none"), Some("cache_sticky"), Some("code_simple"), Some("low")));
    }

    #[test]
    fn aliases_become_model_ids_newest_first() {
        let table = crate::prices::trim(&json!({
            "claude-haiku-4-5": { "input_cost_per_token": 1e-6, "litellm_provider": "anthropic" },
            "claude-haiku-4-5-20251001": { "input_cost_per_token": 1e-6, "litellm_provider": "anthropic" },
            "claude-opus-5": { "input_cost_per_token": 5e-6, "litellm_provider": "anthropic" },
            "claude-opus-5-5": { "input_cost_per_token": 5e-6, "litellm_provider": "anthropic" },
            "bedrock/claude-opus-6-0": { "input_cost_per_token": 5e-6, "litellm_provider": "bedrock" }
        }));
        let previous = env::var_os("ANTHROPIC_DEFAULT_HAIKU_MODEL");
        env::remove_var("ANTHROPIC_DEFAULT_HAIKU_MODEL");
        // The table lags: the current generation's sibling comes first, the table's id is the fallback.
        let haiku = model_ids("haiku", Some("claude-opus-5-5[1m]"), Some(&table));
        let opus = model_ids("opus", Some("claude-fable-5-1"), Some(&table));
        let unknown = model_ids("fable", None, None);
        env::set_var("ANTHROPIC_DEFAULT_HAIKU_MODEL", "my-haiku");
        let overridden = model_ids("haiku", Some("claude-opus-5-5"), Some(&table));
        match previous { Some(value) => env::set_var("ANTHROPIC_DEFAULT_HAIKU_MODEL", value), None => env::remove_var("ANTHROPIC_DEFAULT_HAIKU_MODEL") }
        assert_eq!(haiku, vec!["claude-haiku-5-5", "claude-haiku-4-5"]);
        assert_eq!(opus, vec!["claude-opus-5-5", "claude-opus-5-1", "claude-opus-5"]);
        assert_eq!(unknown, vec!["fable"], "nothing better: the mod falls back to the session's model");
        assert_eq!(overridden, vec!["my-haiku"]);
        assert_eq!(model_ids("claude-sonnet-5-5", None, Some(&table)), vec!["claude-sonnet-5-5"]);
    }

    #[test]
    fn the_mod_routes_only_its_own_session() {
        let previous = env::var_os(MOD_ENV);
        env::set_var(MOD_ENV, "session-a");
        let (own, nested, empty) = (mod_active("session-a"), mod_active("session-b"), mod_active(""));
        match previous { Some(value) => env::set_var(MOD_ENV, value), None => env::remove_var(MOD_ENV) }
        assert!(own && !nested && !empty);
    }

    #[test]
    fn transcripts_are_found_by_session_id() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("projects/-work-app");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("abc-123.jsonl"), TRANSCRIPT).unwrap();
        let previous = env::var_os("CLAUDE_CONFIG_DIR");
        env::set_var("CLAUDE_CONFIG_DIR", directory.path());
        let found = find_transcript("abc-123");
        let escaped = find_transcript("../projects/-work-app/abc-123");
        let missing = find_transcript("other");
        match previous { Some(value) => env::set_var("CLAUDE_CONFIG_DIR", value), None => env::remove_var("CLAUDE_CONFIG_DIR") }
        assert_eq!(found, Some(project.join("abc-123.jsonl")));
        assert_eq!((escaped, missing), (None, None));
    }

    #[test]
    fn aliases_match_full_model_ids() {
        assert!(same_model("claude-opus-5-5", "opus"));
        assert!(same_model("claude-opus-5-5", "claude-opus-5-5"));
        assert!(!same_model("claude-sonnet-5-5", "opus"));
    }
}

#[cfg(test)]
mod ask_tests {
    use super::*;
    use crate::protocol::*;

    #[test]
    fn held_prompts_resume_with_a_dot_or_the_same_text() {
        let directory = tempfile::tempdir().unwrap();
        let previous = env::var_os("XDG_STATE_HOME");
        env::set_var("XDG_STATE_HOME", directory.path());
        let mut event = normalize(json!({ "session_id": "ask-s", "prompt": "refactor the parser", "hook_event_name": "UserPromptSubmit" }), None);
        event.current_model = Some(ModelState { provider: "anthropic".into(), model: "claude-sonnet-5-5".into(), variant: None, scope: None });
        let decision = NormalizedDecision {
            protocol_version: "1".into(), correlation_id: "c".into(), outcome: DecisionOutcome::Ask, consent: None, capability: CapabilityLevel::ConsentOnly,
            diagnostic: Diagnostic::info("consent_requested", "ask"), telemetry: None,
            recommendation: Some(Recommendation { rule_id: "r".into(), mode: RoutingMode::Advisory, provider: Some("anthropic".into()), model: Some("opus".into()), variant: None, gateway_target: None, reason: "Hard work.".into(), routing_class: RoutingClass::CodeHard, task_type: "t".into(), confidence: ClassificationConfidence { routing_class: 0.8, task_type: 1.0, context: 1.0 } }),
        };
        let asked = ask(&decision, &event).unwrap();
        assert_eq!(asked["decision"], "block");
        assert!(asked["reason"].as_str().unwrap().contains("/model opus then send ."));
        let held = take_held("ask-s").unwrap();
        assert_eq!((held.prompt.as_str(), held.suggested.as_str()), ("refactor the parser", "opus"));
        assert!(take_held("ask-s").is_none(), "a held prompt resumes once");
        let resumed = resume(&held, " . ").unwrap();
        assert!(resumed["hookSpecificOutput"]["additionalContext"].as_str().unwrap().ends_with("refactor the parser"));
        assert_eq!(resume(&held, "refactor the parser"), Some(json!({})));
        assert_eq!(resume(&held, "something else"), None);
        event.current_model.as_mut().unwrap().model = "claude-opus-5-5".into();
        assert!(ask(&decision, &event).is_none(), "no question when already on the suggested model");
        match previous { Some(value) => env::set_var("XDG_STATE_HOME", value), None => env::remove_var("XDG_STATE_HOME") }
    }

    #[test]
    fn first_prompt_uses_the_configured_model() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join(".claude")).unwrap();
        fs::write(directory.path().join(".claude/settings.local.json"), r#"{"model":"haiku"}"#).unwrap();
        let previous = env::var_os("ANTHROPIC_MODEL");
        env::remove_var("ANTHROPIC_MODEL");
        assert_eq!(configured_model(directory.path().to_str()).as_deref(), Some("haiku"));
        if let Some(value) = previous { env::set_var("ANTHROPIC_MODEL", value); }
        let event = normalize(json!({ "session_id": "s", "prompt": "hi", "hook_event_name": "UserPromptSubmit", "cwd": directory.path() }), None);
        assert_eq!(event.metadata["hostProvider"], "anthropic");
    }
}
