use crate::{config::{ClassSelection, Config, LayaConfig}, protocol::{ClassificationConfidence, ProviderMetadata, RoutingClass, RoutingClassification}, redaction::project_event};
pub use crate::redaction::ProviderInput;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Unavailable(String),
    Timeout,
    Cancelled,
    Malformed(String),
    LowConfidence,
    Transport(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Malformed(message) | Self::Transport(message) => f.write_str(message),
            Self::Timeout => f.write_str("provider timed out"),
            Self::Cancelled => f.write_str("provider cancelled"),
            Self::LowConfidence => f.write_str("provider confidence is below the configured threshold"),
        }
    }
}

pub trait DecisionProvider: Send + Sync {
    fn name(&self) -> &str;
    fn classify(&self, input: &ProviderInput, timeout: Duration) -> Result<RoutingClassification, ProviderError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequest {
    pub query: String,
    pub context: crate::protocol::BoundedContext,
    pub correlation_id: String,
}

#[derive(Debug, Clone)]
pub struct UnavailableProvider {
    name: String,
    reason: String,
}

impl UnavailableProvider {
    pub fn new(name: impl Into<String>, reason: impl Into<String>) -> Self { Self { name: name.into(), reason: reason.into() } }
}

impl DecisionProvider for UnavailableProvider {
    fn name(&self) -> &str { &self.name }
    fn classify(&self, _input: &ProviderInput, _timeout: Duration) -> Result<RoutingClassification, ProviderError> { Err(ProviderError::Unavailable(self.reason.clone())) }
}

#[derive(Debug, Clone)]
pub struct StaticProvider {
    pub result: Result<RoutingClassification, ProviderError>,
}

impl DecisionProvider for StaticProvider {
    fn name(&self) -> &str { "static" }
    fn classify(&self, _input: &ProviderInput, _timeout: Duration) -> Result<RoutingClassification, ProviderError> { self.result.clone() }
}

#[derive(Debug, Clone)]
pub struct JevProvider {
    pub endpoint: String,
    pub credential_env: Option<String>,
    pub model: Option<String>,
    pub state_template: Option<String>,
    pub questions: Option<Value>,
}

impl JevProvider {
    pub fn from_config(config: &Config) -> Result<Self, ProviderError> {
        let endpoint = config.providers.jev.endpoint.clone().or(config.providers.jev.base_url.clone()).or_else(|| std::env::var("JEV_BASE_URL").ok()).ok_or_else(|| ProviderError::Unavailable("Jev endpoint/base URL is not configured".into()))?;
        if let Some(key) = &config.providers.jev.credential_env {
            if std::env::var(key).ok().filter(|value| !value.trim().is_empty()).is_none() { return Err(ProviderError::Unavailable("Jev credential is not available".into())); }
        }
        Ok(Self { endpoint, credential_env: config.providers.jev.credential_env.clone(), model: config.providers.jev.model.clone(), state_template: config.providers.jev.state_template.clone(), questions: config.providers.jev.questions.clone() })
    }
}

impl DecisionProvider for JevProvider {
    fn name(&self) -> &str { "jev" }
    fn classify(&self, input: &ProviderInput, timeout: Duration) -> Result<RoutingClassification, ProviderError> {
        let body = typesafe_request(input, self);
        verbose_exchange("request", &body);
        if let Ok(response) = std::env::var("SYSTEM1ROUTER_JEV_RESPONSE") {
            let value: Value = serde_json::from_str(&response).map_err(|e| ProviderError::Malformed(format!("Jev response: {e}")))?;
            verbose_exchange("response", &value);
            let value: RoutingClassification = if value.get("answers").is_some() {
                parse_typesafe_classification(&value, &input.correlation_id)?
            } else { serde_json::from_value(value).map_err(|e| ProviderError::Malformed(format!("Jev response: {e}")))? };
            return validate_classification(value, &input.correlation_id);
        }
        let token = self.credential_env.as_ref().and_then(|name| std::env::var(name).ok());
        let response = post_json(&self.endpoint, &body, token.as_deref(), timeout)?;
        verbose_exchange("response", &response);
        let result = if response.get("answers").is_some() {
            parse_typesafe_classification(&response, &input.correlation_id)?
        } else {
            let response = response.get("classification").cloned().unwrap_or(response);
            serde_json::from_value(response).map_err(|e| ProviderError::Malformed(format!("Jev response: {e}")))?
        };
        validate_classification(result, &input.correlation_id)
    }
}

#[derive(Debug, Clone)]
pub struct LayaProvider { config: LayaConfig }

impl LayaProvider {
    pub fn from_config(config: &Config) -> Result<Self, ProviderError> {
        let mut runtime = config.providers.laya.clone();
        if runtime.command.is_none() && runtime.endpoint.is_none() && runtime.base_url.is_none() { runtime.base_url = std::env::var("LAYA_BASE_URL").ok(); }
        if runtime.command.is_none() && runtime.endpoint.is_none() && runtime.base_url.is_none() { return Err(ProviderError::Unavailable("Laya runtime is not configured".into())); }
        Ok(Self { config: runtime })
    }
}

impl DecisionProvider for LayaProvider {
    fn name(&self) -> &str { "laya" }
    fn classify(&self, input: &ProviderInput, timeout: Duration) -> Result<RoutingClassification, ProviderError> {
        if self.config.command.is_none() {
            let endpoint = self.config.endpoint.as_ref().or(self.config.base_url.as_ref()).ok_or_else(|| ProviderError::Unavailable("Laya local worker endpoint is missing".into()))?;
            let request = serde_json::to_value(DecisionRequest { query: input.query.clone(), context: input.context.clone(), correlation_id: input.correlation_id.clone() }).map_err(|e| ProviderError::Malformed(e.to_string()))?;
            let response = post_json(endpoint, &request, None, timeout)?;
            let response = response.get("classification").cloned().unwrap_or(response);
            let result: RoutingClassification = serde_json::from_value(response).map_err(|e| ProviderError::Malformed(format!("Laya response: {e}")))?;
            return validate_classification(result, &input.correlation_id);
        }
        let command = self.config.command.as_ref().expect("checked above");
        let mut child = Command::new(command).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| ProviderError::Transport(format!("start Laya worker: {e}")))?;
        let request = serde_json::to_vec(&DecisionRequest { query: input.query.clone(), context: input.context.clone(), correlation_id: input.correlation_id.clone() }).map_err(|e| ProviderError::Malformed(e.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() { use std::io::Write; stdin.write_all(&request).map_err(|e| ProviderError::Transport(format!("write Laya request: {e}")))?; }
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait().map_err(|e| ProviderError::Transport(format!("wait for Laya worker: {e}")))? {
                if !status.success() { return Err(ProviderError::Transport(format!("Laya worker exited with {status}"))); }
                let output = child.wait_with_output().map_err(|e| ProviderError::Transport(format!("read Laya response: {e}")))?;
                let result: RoutingClassification = serde_json::from_slice(&output.stdout).map_err(|e| ProviderError::Malformed(format!("Laya response: {e}")))?;
                return validate_classification(result, &input.correlation_id);
            }
            if Instant::now() >= deadline { let _ = child.kill(); return Err(ProviderError::Timeout); }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

pub fn build_provider(config: &Config) -> Box<dyn DecisionProvider> {
    match config.providers.selected.as_str() {
        "jev" => JevProvider::from_config(config).map(|provider| Box::new(provider) as Box<dyn DecisionProvider>).unwrap_or_else(|error| Box::new(UnavailableProvider::new("jev", error.to_string()))),
        "laya" => LayaProvider::from_config(config).map(|provider| Box::new(provider) as Box<dyn DecisionProvider>).unwrap_or_else(|error| Box::new(UnavailableProvider::new("laya", error.to_string()))),
        "none" | "" => Box::new(UnavailableProvider::new("none", "no decision provider selected")),
        name => Box::new(UnavailableProvider::new(name, "unknown decision provider")),
    }
}

pub fn validate_classification(mut result: RoutingClassification, correlation_id: &str) -> Result<RoutingClassification, ProviderError> {
    let values = [result.confidence.routing_class, result.confidence.task_type, result.confidence.context];
    if values.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) { return Err(ProviderError::Malformed("confidence must be between 0 and 1".into())); }
    if result.task_type.trim().is_empty() || result.provider_metadata.provider.trim().is_empty() { return Err(ProviderError::Malformed("taskType and providerMetadata.provider are required".into())); }
    result.correlation_id = correlation_id.to_string();
    Ok(result)
}

fn verbose_exchange(direction: &str, body: &Value) {
    if std::env::var("SYSTEM1ROUTER_VERBOSE").ok().as_deref() != Some("1") { return; }
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_millis()).unwrap_or_default();
    let entry = json!({ "timestampMs": timestamp, "provider": "jev", "direction": direction, "body": body });
    let Ok(line) = serde_json::to_string(&entry) else { return; };
    eprintln!("[system1router] Jev {direction}: {body}");
    let Some(path) = verbose_log_path() else { return; };
    let Some(parent) = path.parent() else { return; };
    if fs::create_dir_all(parent).is_err() { return; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 2 * 1024 * 1024) {
        let rotated = PathBuf::from(format!("{}.1", path.display()));
        let _ = fs::remove_file(&rotated);
        let _ = fs::rename(&path, rotated);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
        }
        let _ = writeln!(file, "{line}");
    }
}

fn verbose_log_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SYSTEM1ROUTER_VERBOSE_LOG") { return Some(PathBuf::from(path)); }
    if let Ok(base) = std::env::var("XDG_STATE_HOME") { return Some(PathBuf::from(base).join("system1router/provider-debug.jsonl")); }
    #[cfg(windows)]
    { std::env::var("APPDATA").ok().map(|base| PathBuf::from(base).join("system1router/provider-debug.jsonl")) }
    #[cfg(not(windows))]
    { std::env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local/state/system1router/provider-debug.jsonl")) }
}

fn redact_endpoint(endpoint: &str) -> String {
    let without_query = endpoint.split(['?', '#']).next().unwrap_or(endpoint);
    let Some((scheme, rest)) = without_query.split_once("://") else { return without_query.to_string(); };
    let (authority, path) = rest.find('/').map(|index| (&rest[..index], &rest[index..])).unwrap_or((rest, ""));
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    format!("{scheme}://{authority}{path}")
}

fn post_json(endpoint: &str, body: &serde_json::Value, bearer: Option<&str>, timeout: Duration) -> Result<serde_json::Value, ProviderError> {
    let agent = ureq::AgentBuilder::new().timeout_connect(timeout).timeout(timeout).build();
    let mut request = agent.post(endpoint).set("Content-Type", "application/json");
    if let Some(value) = bearer {
        request = request.set("Authorization", &format!("Bearer {value}"));
    }
    let response = request.send_json(body).map_err(|error| match error {
        ureq::Error::Status(status, _) => {
            verbose_exchange("error", &json!({ "status": status }));
            ProviderError::Transport(format!("provider returned status {status}"))
        }
        ureq::Error::Transport(error) => {
            verbose_exchange("error", &json!({ "type": "transport" }));
            let message = error.to_string().replace(endpoint, &redact_endpoint(endpoint));
            ProviderError::Transport(format!("provider request failed: {message}"))
        }
    })?;
    response.into_json().map_err(|error| ProviderError::Malformed(format!("provider response JSON: {error}")))
}

/// Renders bounded context as compact labelled text. Empty fields are omitted,
/// which keeps provider prompts short and avoids noise such as `null` fields.
pub fn render_context(context: &crate::protocol::BoundedContext) -> String {
    let mut sections = Vec::new();
    let mut push = |label: &str, value: Option<&str>| {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) { sections.push(format!("{label}: {value}")); }
    };
    push("Project", context.project_summary.as_deref());
    push("Repository", context.repository.as_deref());
    push("Branch", context.branch.as_deref());
    if !context.files.is_empty() { push("Files in play", Some(&context.files.join(", "))); }
    if !context.tools.is_empty() { push("Tools used", Some(&context.tools.join(", "))); }
    if let Some(summary) = context.conversation_summary.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        sections.push(format!("Recent conversation:\n{summary}"));
    }
    sections.join("\n")
}

fn typesafe_request(input: &ProviderInput, provider: &JevProvider) -> Value {
    let context = render_context(&input.context);
    let state = render_state(provider.state_template.as_deref().unwrap_or("{context}\n\nCurrent request: {query}"), &input.query, &context);
    let state = state.trim_start().to_string();
    let questions = provider.questions.clone().unwrap_or_else(|| json!({
            "routing_class": {
                "type": "choice",
                "instructions": "Classify the Current request. Use the conversation only to resolve what it refers to (for example 'do it' or 'fix that'). Choose code_* when the assistant is expected to change files or run commands, question_* when it is expected to answer.",
                "criteria": {
                    "question_simple": "A direct question requiring limited reasoning or context.",
                    "question_moderate": "A question requiring multi-step reasoning or synthesis of relevant context.",
                    "question_hard": "A question requiring deep analysis, broad context, or difficult trade-off reasoning.",
                    "code_simple": "A small, localized, low-risk code change or implementation task.",
                    "code_moderate": "A multi-step, multi-file, or non-trivial implementation, debugging, or refactoring task.",
                    "code_hard": "A complex, high-risk, architectural, security, performance, or systemic debugging task."
                }
            },
            "task_type": {
                "type": "choice",
                "instructions": "Choose the primary task type.",
                "criteria": {
                    "code-generation": "Writing or modifying source code.",
                    "debugging": "Investigating or fixing a defect.",
                    "question-answering": "Answering a conceptual or factual question.",
                    "analysis": "Analyzing code, data, or a technical design.",
                    "planning": "Designing or planning an implementation."
                }
            },
            "context": {
                "type": "score",
                "instructions": "How much project context is needed to answer this request correctly?",
                "criteria": [
                    "Little or no project context is needed.",
                    "Some project context is useful.",
                    "Substantial project context is required."
                ]
            }
        }));
    json!({ "state": state, "model": provider.model.as_deref().unwrap_or("jev-latest"), "questions": questions })
}

fn render_state(template: &str, query: &str, context: &str) -> String {
    let mut output = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(next) = rest.strip_prefix("{query}") {
            output.push_str(query);
            rest = next;
        } else if let Some(next) = rest.strip_prefix("{context}") {
            output.push_str(context);
            rest = next;
        } else {
            output.push('{');
            rest = &rest[1..];
        }
    }
    output.push_str(rest);
    output
}

fn parse_typesafe_classification(response: &Value, correlation_id: &str) -> Result<RoutingClassification, ProviderError> {
    let answers = response.get("answers").and_then(Value::as_object).ok_or_else(|| ProviderError::Malformed("Jev response is missing answers".into()))?;
    let routing_answer = answers.get("routing_class").ok_or_else(|| ProviderError::Malformed("Jev response is missing routing_class".into()))?;
    let routing_value = routing_answer.get("choice").and_then(Value::as_str).ok_or_else(|| ProviderError::Malformed("Jev routing_class answer is missing choice".into()))?;
    let routing_class: RoutingClass = serde_json::from_value(Value::String(routing_value.into())).map_err(|_| ProviderError::Malformed(format!("Jev returned unknown routing class: {routing_value}")))?;
    let task_answer = answers.get("task_type").ok_or_else(|| ProviderError::Malformed("Jev response is missing task_type".into()))?;
    let task_type = task_answer.get("choice").and_then(Value::as_str).ok_or_else(|| ProviderError::Malformed("Jev task_type answer is missing choice".into()))?;
    let context_answer = answers.get("context").ok_or_else(|| ProviderError::Malformed("Jev response is missing context".into()))?;
    let context_score = context_answer.get("score").and_then(Value::as_f64).unwrap_or_default();
    let model = response.get("model").and_then(Value::as_str).map(str::to_owned);
    let class_probabilities = routing_answer.get("probabilities").and_then(Value::as_object).map(|items| {
        items.iter().filter_map(|(name, value)| Some((name.clone(), value.as_f64()? as f32))).collect()
    });
    Ok(RoutingClassification {
        routing_class,
        task_type: task_type.into(),
        context_signals: vec![format!("context_score_{context_score}")],
        class_probabilities,
        confidence: ClassificationConfidence {
            routing_class: answer_confidence(routing_answer),
            task_type: answer_confidence(task_answer),
            context: answer_confidence(context_answer),
        },
        provider_metadata: ProviderMetadata { provider: "jev".into(), version: "typesafe-systemone".into(), model, latency_ms: None },
        correlation_id: correlation_id.into(),
    })
}

fn answer_confidence(answer: &Value) -> f32 {
    answer.get("confidence").and_then(Value::as_f64).unwrap_or_default() as f32
}

/// Derives the class from a provider distribution. The six classes are two
/// ordinal scales (question/code × simple/moderate/hard), so the kind is
/// chosen by total probability mass and the tier by the median of that
/// kind's distribution. Unlike argmax this does not pick "simple" when most
/// of the mass says "moderate or harder", which is the costly error. `Upper`
/// takes the hardest tier still holding a fifth of the kind's mass: Jev
/// errs one tier low, and this corrects most of it (experiments/RESULTS.md).
const UPPER_SHARE: f32 = 0.2;

pub fn select_class(mut result: RoutingClassification, selection: ClassSelection) -> RoutingClassification {
    if selection == ClassSelection::Argmax { return result; }
    let Some(probabilities) = &result.class_probabilities else { return result; };
    let mut mass = [[0.0_f32; 3]; 2];
    for (name, probability) in probabilities {
        let Some(class) = RoutingClass::parse(name) else { continue; };
        if probability.is_finite() && *probability > 0.0 { mass[usize::from(class.is_code())][class.tier()] += probability; }
    }
    let totals = [mass[0].iter().sum::<f32>(), mass[1].iter().sum::<f32>()];
    let total = totals[0] + totals[1];
    if total <= f32::EPSILON { return result; }
    let code = totals[1] > totals[0] || (totals[1] == totals[0] && result.routing_class.is_code());
    let kind = &mass[usize::from(code)];
    let kind_total = totals[usize::from(code)];
    let mut cumulative = 0.0;
    let mut tier = 2;
    for (index, value) in kind.iter().enumerate() {
        cumulative += value;
        if cumulative >= kind_total / 2.0 { tier = index; break; }
    }
    if selection == ClassSelection::Upper { tier = (0..3).rev().find(|&index| kind[index..].iter().sum::<f32>() >= kind_total * UPPER_SHARE).unwrap_or(0); }
    let selected = RoutingClass::from_parts(code, tier);
    if selected != result.routing_class {
        result.context_signals.push(format!("class_adjusted_from_{}", result.routing_class));
        // Confidence of the adjusted class is its own probability share.
        result.confidence.routing_class = (kind[tier] / total).clamp(0.0, 1.0);
        result.routing_class = selected;
    }
    result
}

pub fn classify_event(provider: &dyn DecisionProvider, event: &crate::protocol::NormalizedEvent, config: &Config) -> Result<RoutingClassification, ProviderError> {
    let input = project_event(event, &config.redaction);
    let result = provider.classify(&input, Duration::from_millis(config.timeouts.provider_ms))?;
    let result = select_class(validate_classification(result, &input.correlation_id)?, config.routing.class_selection);
    if result.confidence.routing_class < config.routing.min_confidence || result.confidence.task_type < config.routing.min_confidence { return Err(ProviderError::LowConfidence); }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ClassificationConfidence, ProviderMetadata, RoutingClass};
    #[test]
    fn validates_canonical_classes_and_confidence() {
        let result = RoutingClassification { routing_class: RoutingClass::CodeHard, task_type: "code-generation".into(), context_signals: vec![], class_probabilities: None, confidence: ClassificationConfidence { routing_class: 0.9, task_type: 0.8, context: 0.7 }, provider_metadata: ProviderMetadata { provider: "test".into(), version: "1".into(), model: None, latency_ms: None }, correlation_id: "old".into() };
        assert_eq!(validate_classification(result, "new").unwrap().correlation_id, "new");
    }

    #[test]
    fn parses_typesafe_system_one_response() {
        let response = json!({
            "model": "jev-1.13.0",
            "answers": {
                "routing_class": { "type": "choice", "choice": "code_hard", "confidence": 0.9 },
                "task_type": { "type": "choice", "choice": "debugging", "confidence": 0.8 },
                "context": { "type": "score", "score": 2.0, "confidence": 0.7 }
            }
        });
        let result = parse_typesafe_classification(&response, "correlation").unwrap();
        assert_eq!(result.routing_class, RoutingClass::CodeHard);
        assert_eq!(result.task_type, "debugging");
        assert_eq!(result.provider_metadata.model.as_deref(), Some("jev-1.13.0"));
        assert!(result.class_probabilities.is_none());
        let with_probabilities = json!({ "answers": {
            "routing_class": { "choice": "code_simple", "confidence": 0.45, "probabilities": { "code_simple": 0.45, "code_moderate": 0.4, "code_hard": 0.15 } },
            "task_type": { "choice": "debugging", "confidence": 0.8 },
            "context": { "score": 1.0, "confidence": 0.7 } } });
        let parsed = parse_typesafe_classification(&with_probabilities, "c").unwrap();
        assert_eq!(parsed.class_probabilities.unwrap().get("code_moderate"), Some(&0.4));
    }

    #[test]
    fn custom_state_and_questions_use_redacted_input() {
        let provider = JevProvider {
            endpoint: "http://localhost".into(), credential_env: None, model: None,
            state_template: Some("Decide: {query}; context: {context}".into()),
            questions: Some(json!({ "routing_class": { "type": "choice" }, "task_type": { "type": "choice" }, "context": { "type": "score" } })),
        };
        let input = ProviderInput { query: "[REDACTED]".into(), context: Default::default(), correlation_id: "c".into() };
        let body = typesafe_request(&input, &provider);
        assert!(body["state"].as_str().unwrap().starts_with("Decide: [REDACTED]; context:"));
        assert_eq!(body["questions"], provider.questions.unwrap());
        assert_eq!(render_state("{query} {context}", "literal {context}", "literal {query}"), "literal {context} literal {query}");
    }

    #[test]
    fn default_state_places_context_before_the_current_request() {
        let provider = JevProvider {
            endpoint: "http://localhost".into(), credential_env: None, model: None,
            state_template: None, questions: None,
        };
        let input = ProviderInput {
            query: "current request".into(),
            context: crate::protocol::BoundedContext {
                conversation_summary: Some("earlier conversation".into()),
                ..Default::default()
            },
            correlation_id: "c".into(),
        };
        let state = typesafe_request(&input, &provider)["state"].as_str().unwrap().to_owned();
        assert!(state.find("earlier conversation").unwrap() < state.find("Current request: current request").unwrap());
        assert!(!state.contains("null"), "empty context fields must not be sent: {state}");
    }

    #[test]
    fn empty_context_sends_only_the_request() {
        let provider = JevProvider { endpoint: "http://localhost".into(), credential_env: None, model: None, state_template: None, questions: None };
        let input = ProviderInput { query: "commit".into(), context: Default::default(), correlation_id: "c".into() };
        assert_eq!(typesafe_request(&input, &provider)["state"], "Current request: commit");
    }

    fn distribution(pairs: &[(&str, f32)], top: RoutingClass) -> RoutingClassification {
        RoutingClassification {
            routing_class: top, task_type: "analysis".into(), context_signals: vec![],
            class_probabilities: Some(pairs.iter().map(|(name, value)| (name.to_string(), *value)).collect()),
            confidence: ClassificationConfidence { routing_class: 0.45, task_type: 0.9, context: 0.9 },
            provider_metadata: ProviderMetadata { provider: "jev".into(), version: "1".into(), model: None, latency_ms: None },
            correlation_id: "c".into(),
        }
    }

    #[test]
    fn median_tier_corrects_an_underpowered_argmax() {
        let result = select_class(distribution(&[("code_simple", 0.45), ("code_moderate", 0.35), ("code_hard", 0.20)], RoutingClass::CodeSimple), ClassSelection::Median);
        assert_eq!(result.routing_class, RoutingClass::CodeModerate);
        assert!((result.confidence.routing_class - 0.35).abs() < 1e-6);
        assert!(result.context_signals.contains(&"class_adjusted_from_code_simple".to_string()));
    }

    #[test]
    fn upper_tier_takes_the_hardest_tier_holding_a_fifth_of_the_mass() {
        let result = select_class(distribution(&[("code_simple", 0.45), ("code_moderate", 0.35), ("code_hard", 0.20)], RoutingClass::CodeSimple), ClassSelection::Upper);
        assert_eq!(result.routing_class, RoutingClass::CodeHard);
        assert!(result.context_signals.contains(&"class_adjusted_from_code_simple".to_string()));
        let moderate = select_class(distribution(&[("question_simple", 0.6), ("question_moderate", 0.3), ("question_hard", 0.1)], RoutingClass::QuestionSimple), ClassSelection::Upper);
        assert_eq!(moderate.routing_class, RoutingClass::QuestionModerate, "0.1 is below a fifth, 0.4 is above");
        let sure = select_class(distribution(&[("question_simple", 0.95), ("question_moderate", 0.05)], RoutingClass::QuestionSimple), ClassSelection::Upper);
        assert_eq!(sure.routing_class, RoutingClass::QuestionSimple);
    }

    #[test]
    fn kind_uses_total_mass() {
        let result = select_class(distribution(&[("question_simple", 0.40), ("code_simple", 0.35), ("code_moderate", 0.25)], RoutingClass::QuestionSimple), ClassSelection::Median);
        assert_eq!(result.routing_class, RoutingClass::CodeSimple);
    }

    #[test]
    fn confident_argmax_and_argmax_mode_are_unchanged() {
        let confident = select_class(distribution(&[("question_simple", 0.99), ("code_simple", 0.01)], RoutingClass::QuestionSimple), ClassSelection::Median);
        assert_eq!(confident.routing_class, RoutingClass::QuestionSimple);
        assert!((confident.confidence.routing_class - 0.45).abs() < 1e-6);
        let argmax = select_class(distribution(&[("code_simple", 0.45), ("code_moderate", 0.35), ("code_hard", 0.20)], RoutingClass::CodeSimple), ClassSelection::Argmax);
        assert_eq!(argmax.routing_class, RoutingClass::CodeSimple);
    }
}
