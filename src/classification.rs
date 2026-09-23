use crate::{config::{Config, LayaConfig}, protocol::{ClassificationConfidence, ProviderMetadata, RoutingClass, RoutingClassification}, redaction::project_event};
pub use crate::redaction::ProviderInput;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{process::{Command, Stdio}, time::{Duration, Instant}};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderHealth {
    pub available: bool,
    pub provider: String,
    pub message: String,
}

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
    fn health(&self) -> ProviderHealth;
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
    fn health(&self) -> ProviderHealth { ProviderHealth { available: false, provider: self.name.clone(), message: self.reason.clone() } }
    fn classify(&self, _input: &ProviderInput, _timeout: Duration) -> Result<RoutingClassification, ProviderError> { Err(ProviderError::Unavailable(self.reason.clone())) }
}

#[derive(Debug, Clone)]
pub struct StaticProvider {
    pub result: Result<RoutingClassification, ProviderError>,
}

impl DecisionProvider for StaticProvider {
    fn name(&self) -> &str { "static" }
    fn health(&self) -> ProviderHealth { ProviderHealth { available: self.result.is_ok(), provider: "static".into(), message: "test provider".into() } }
    fn classify(&self, _input: &ProviderInput, _timeout: Duration) -> Result<RoutingClassification, ProviderError> { self.result.clone() }
}

#[derive(Debug, Clone)]
pub struct JevProvider {
    pub endpoint: String,
    pub credential_env: Option<String>,
    pub model: Option<String>,
}

impl JevProvider {
    pub fn from_config(config: &Config) -> Result<Self, ProviderError> {
        let endpoint = config.providers.jev.endpoint.clone().or(config.providers.jev.base_url.clone()).or_else(|| std::env::var("JEV_BASE_URL").ok()).ok_or_else(|| ProviderError::Unavailable("Jev endpoint/base URL is not configured".into()))?;
        if let Some(key) = &config.providers.jev.credential_env {
            if std::env::var(key).ok().filter(|value| !value.trim().is_empty()).is_none() { return Err(ProviderError::Unavailable("Jev credential is not available".into())); }
        }
        Ok(Self { endpoint, credential_env: config.providers.jev.credential_env.clone(), model: config.providers.jev.model.clone() })
    }
}

impl DecisionProvider for JevProvider {
    fn name(&self) -> &str { "jev" }
    fn health(&self) -> ProviderHealth { ProviderHealth { available: true, provider: "jev".into(), message: format!("configured endpoint {}", redact_endpoint(&self.endpoint)) } }
    fn classify(&self, input: &ProviderInput, timeout: Duration) -> Result<RoutingClassification, ProviderError> {
        if let Ok(response) = std::env::var("SYSTEM1ROUTER_JEV_RESPONSE") {
            let value: RoutingClassification = serde_json::from_str(&response).map_err(|e| ProviderError::Malformed(format!("Jev response: {e}")))?;
            return validate_classification(value, &input.correlation_id);
        }
        let body = typesafe_request(input, self.model.as_deref());
        let token = self.credential_env.as_ref().and_then(|name| std::env::var(name).ok());
        let response = post_json(&self.endpoint, &body, token.as_deref(), timeout)?;
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
    fn health(&self) -> ProviderHealth { ProviderHealth { available: self.config.command.is_some() || self.config.endpoint.is_some(), provider: "laya".into(), message: if self.config.command.is_some() { "local worker configured".into() } else if self.config.endpoint.is_some() { "configured local endpoint".into() } else { "local worker unavailable".into() } } }
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

fn redact_endpoint(endpoint: &str) -> String {
    endpoint.split('?').next().unwrap_or(endpoint).to_string()
}

fn post_json(endpoint: &str, body: &serde_json::Value, bearer: Option<&str>, timeout: Duration) -> Result<serde_json::Value, ProviderError> {
    let agent = ureq::AgentBuilder::new().timeout_connect(timeout).timeout(timeout).build();
    let mut request = agent.post(endpoint).set("Content-Type", "application/json");
    if let Some(value) = bearer {
        request = request.set("Authorization", &format!("Bearer {value}"));
    }
    let response = request.send_json(body).map_err(|error| match error {
        ureq::Error::Status(status, _) => ProviderError::Transport(format!("provider returned status {status}")),
        ureq::Error::Transport(error) => ProviderError::Transport(format!("provider request failed: {error}")),
    })?;
    response.into_json().map_err(|error| ProviderError::Malformed(format!("provider response JSON: {error}")))
}

fn typesafe_request(input: &ProviderInput, model: Option<&str>) -> Value {
    let context = serde_json::to_string(&input.context).unwrap_or_else(|_| "{}".into());
    json!({
        "state": format!("{}\n\nBounded context: {context}", input.query),
        "model": model.unwrap_or("jev-latest"),
        "questions": {
            "routing_class": {
                "type": "choice",
                "instructions": "Choose exactly one routing class for this request.",
                "criteria": {
                    "question_simple": "A straightforward question that needs a short, direct answer.",
                    "question_moderate": "A question requiring explanation, synthesis, or several reasoning steps.",
                    "question_hard": "A difficult question requiring deep analysis or specialist reasoning.",
                    "code_simple": "A small, localized code change or simple implementation task.",
                    "code_moderate": "A code task spanning several files, components, or reasoning steps.",
                    "code_hard": "A complex architecture, debugging, or high-risk code task."
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
        }
    })
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
    Ok(RoutingClassification {
        routing_class,
        task_type: task_type.into(),
        context_signals: vec![format!("context_score_{context_score}")],
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

pub fn classify_event(provider: &dyn DecisionProvider, event: &crate::protocol::NormalizedEvent, config: &Config) -> Result<RoutingClassification, ProviderError> {
    let input = project_event(event, &config.redaction);
    let result = provider.classify(&input, Duration::from_millis(config.timeouts.provider_ms))?;
    let result = validate_classification(result, &input.correlation_id)?;
    if result.confidence.routing_class < config.routing.min_confidence || result.confidence.task_type < config.routing.min_confidence { return Err(ProviderError::LowConfidence); }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ClassificationConfidence, ProviderMetadata, RoutingClass};
    #[test]
    fn validates_canonical_classes_and_confidence() {
        let result = RoutingClassification { routing_class: RoutingClass::CodeHard, task_type: "code-generation".into(), context_signals: vec![], confidence: ClassificationConfidence { routing_class: 0.9, task_type: 0.8, context: 0.7 }, provider_metadata: ProviderMetadata { provider: "test".into(), version: "1".into(), model: None, latency_ms: None }, correlation_id: "old".into() };
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
    }
}
