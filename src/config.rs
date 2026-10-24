use crate::protocol::{GatewayTarget, PolicyTarget, RoutingClass, RoutingMode};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::{Path, PathBuf}};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    #[serde(default = "default_provider")]
    pub selected: String,
    #[serde(default)]
    pub jev: JevConfig,
    #[serde(default)]
    pub laya: LayaConfig,
}

fn default_provider() -> String { "none".into() }

impl Default for ProviderConfig {
    fn default() -> Self { Self { selected: default_provider(), jev: JevConfig::default(), laya: LayaConfig::default() } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JevConfig {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_jev_credential_env")]
    pub credential_env: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Text sent as System One's state. {query} and {context} are replaced
    /// after the request has been bounded and redacted.
    #[serde(default)]
    pub state_template: Option<String>,
    /// Optional replacement for the System One routing questions.
    #[serde(default)]
    pub questions: Option<serde_json::Value>,
}

impl Default for JevConfig {
    fn default() -> Self { Self { endpoint: None, base_url: None, credential_env: default_jev_credential_env(), model: None, state_template: None, questions: None } }
}

fn default_jev_credential_env() -> Option<String> { Some("JEV_API_KEY".into()) }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LayaConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub startup_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionConfig {
    #[serde(default = "default_limit")]
    pub max_query_chars: usize,
    #[serde(default = "default_limit")]
    pub max_context_chars: usize,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default = "default_true")]
    pub redact_paths: bool,
    #[serde(default = "default_true")]
    pub redact_tool_arguments: bool,
}

fn default_limit() -> usize { 4_000 }
fn default_true() -> bool { true }

impl Default for RedactionConfig {
    fn default() -> Self { Self { max_query_chars: default_limit(), max_context_chars: default_limit(), patterns: Vec::new(), redact_paths: true, redact_tool_arguments: true } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeoutConfig {
    #[serde(default = "default_provider_timeout")]
    pub provider_ms: u64,
    #[serde(default = "default_hook_timeout")]
    pub hook_ms: u64,
    #[serde(default = "default_consent_timeout")]
    pub consent_ms: u64,
}

fn default_provider_timeout() -> u64 { 3_000 }
fn default_hook_timeout() -> u64 { 6_000 }
fn default_consent_timeout() -> u64 { 15_000 }

impl Default for TimeoutConfig {
    fn default() -> Self { Self { provider_ms: default_provider_timeout(), hook_ms: default_hook_timeout(), consent_ms: default_consent_timeout() } }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConsentMode { Interactive, AlwaysAccept, AlwaysDecline, Disabled }

impl Default for ConsentMode { fn default() -> Self { Self::Interactive } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentConfig {
    #[serde(default)]
    pub mode: ConsentMode,
    #[serde(default = "default_true")]
    pub decline_on_timeout: bool,
}

impl Default for ConsentConfig { fn default() -> Self { Self { mode: ConsentMode::default(), decline_on_timeout: true } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub default_mode: RoutingMode,
    #[serde(default = "default_true")]
    pub fail_open: bool,
    #[serde(default)]
    pub min_confidence: f32,
    #[serde(default)]
    pub allowed_task_types: Vec<String>,
    /// How the class is derived when the provider returns a class distribution.
    #[serde(default)]
    pub class_selection: ClassSelection,
    /// Skip a downgrade that would cost more than it saves because switching
    /// models discards the conversation's prompt cache. Needs host prices.
    #[serde(default = "default_true")]
    pub cache_aware: bool,
    /// What a decision changes: the model, or only the current model's effort.
    #[serde(default)]
    pub swap: Swap,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Swap {
    /// Switch to the model a matching rule names.
    #[default]
    Model,
    /// Keep the model and set the effort `effort.levels` gives the class
    /// (Claude Code `/effort`, an OpenCode variant). Rules are not used.
    Effort,
}

/// The effort swap (`routing.swap: "effort"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffortConfig {
    /// Effort per routing class (`code_hard: "xhigh"`); a class left out keeps
    /// its default.
    #[serde(default)]
    pub levels: std::collections::BTreeMap<String, String>,
    /// From this many context tokens a lower effort waits: changing effort
    /// re-writes the cached conversation, which costs more than the thinking
    /// it saves. A higher effort always applies. 0 never waits.
    #[serde(default = "default_effort_cache_guard")]
    pub cache_guard_tokens: u64,
    /// How long the host's prompt cache outlives the last response; after that
    /// much idle time nothing cached is lost and any effort applies.
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_ms: u64,
}

fn default_effort_cache_guard() -> u64 { 50_000 }
fn default_cache_ttl() -> u64 { 300_000 }

impl Default for EffortConfig { fn default() -> Self { Self { levels: Default::default(), cache_guard_tokens: default_effort_cache_guard(), cache_ttl_ms: default_cache_ttl() } } }

impl EffortConfig {
    pub fn level(&self, class: &RoutingClass) -> String {
        self.levels.get(class.as_str()).cloned().unwrap_or_else(|| match class {
            RoutingClass::QuestionSimple => "low",
            RoutingClass::QuestionModerate | RoutingClass::CodeSimple => "medium",
            RoutingClass::QuestionHard | RoutingClass::CodeModerate => "high",
            RoutingClass::CodeHard => "xhigh",
        }.into())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ClassSelection {
    /// Pick code vs question by probability mass, then the median difficulty
    /// tier. Minimises expected tier error when the provider is uncertain.
    #[default]
    Median,
    /// Use the provider's top choice unchanged.
    Argmax,
    /// Kind by mass, then the hardest tier holding a fifth of that kind's
    /// mass. Against a provider that errs one tier low (Jev), the fewest
    /// too-low efforts.
    Upper,
}

impl Default for RoutingConfig { fn default() -> Self { Self { enabled: false, default_mode: RoutingMode::Advisory, fail_open: true, min_confidence: 0.0, allowed_task_types: Vec::new(), class_selection: ClassSelection::default(), cache_aware: true, swap: Swap::default() } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default)]
    pub auth_token_env: Option<String>,
    #[serde(default)]
    pub upstream_allowlist: Vec<GatewayTarget>,
    #[serde(default)]
    pub default_upstream: Option<String>,
    #[serde(default)]
    pub rate_limit_per_minute: Option<u32>,
}

fn default_bind() -> String { "127.0.0.1:8765".into() }

impl Default for GatewayConfig { fn default() -> Self { Self { enabled: false, bind: default_bind(), auth_token_env: None, upstream_allowlist: Vec::new(), default_upstream: None, rate_limit_per_minute: None } } }

/// Fallback model prices for hosts that report none.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricesConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_prices_url")]
    pub url: String,
}

fn default_prices_url() -> String { crate::prices::DEFAULT_URL.into() }

impl Default for PricesConfig { fn default() -> Self { Self { enabled: true, url: default_prices_url() } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub debug_capture: bool,
    /// Also record the redacted request and context the provider saw, so a
    /// trial's decisions can be labelled afterwards. Local only; off by default.
    #[serde(default)]
    pub capture_prompts: bool,
}

impl Default for TelemetryConfig { fn default() -> Self { Self { enabled: true, debug_capture: false, capture_prompts: false } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRule {
    pub id: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub routing_class: Option<RoutingClass>,
    #[serde(default)]
    pub task_types: Vec<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub min_confidence: Option<f32>,
    #[serde(default)]
    pub mode: Option<RoutingMode>,
    #[serde(default)]
    pub predicate: Option<String>,
    pub target: PolicyTarget,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DefaultRecommendation {
    #[serde(default)]
    pub mode: Option<RoutingMode>,
    #[serde(default)]
    pub target: Option<PolicyTarget>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub providers: ProviderConfig,
    #[serde(default)]
    pub redaction: RedactionConfig,
    #[serde(default)]
    pub timeouts: TimeoutConfig,
    #[serde(default)]
    pub consent: ConsentConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    #[serde(default)]
    pub prices: PricesConfig,
    #[serde(default)]
    pub effort: EffortConfig,
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
    #[serde(default)]
    pub default_recommendation: DefaultRecommendation,
}

impl Default for Config {
    fn default() -> Self { Self { providers: ProviderConfig::default(), redaction: RedactionConfig::default(), timeouts: TimeoutConfig::default(), consent: ConsentConfig::default(), routing: RoutingConfig::default(), gateway: GatewayConfig::default(), telemetry: TelemetryConfig::default(), prices: PricesConfig::default(), effort: EffortConfig::default(), rules: Vec::new(), default_recommendation: DefaultRecommendation::default() } }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self, String> {
        load_dotenv()?;
        let path = path.map(PathBuf::from).or_else(default_config_path);
        let Some(path) = path else { return Ok(Self::default()); };
        if !path.exists() { return Ok(Self::default()); }
        let contents = fs::read_to_string(&path).map_err(|e| format!("read config: {e}"))?;
        let config: Self = if path.extension().and_then(|x| x.to_str()) == Some("toml") {
            toml::from_str(&contents).map_err(|e| format!("parse TOML config: {e}"))?
        } else {
            serde_json::from_str(&contents).map_err(|e| format!("parse JSON config: {e}"))?
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.timeouts.provider_ms == 0 || self.timeouts.hook_ms == 0 { return Err("timeouts must be greater than zero".into()); }
        if !(1..=86_400_000).contains(&self.timeouts.consent_ms) { return Err("timeouts.consentMs must be between 1 and 86400000 milliseconds".into()); }
        if self.redaction.max_query_chars == 0 || self.redaction.max_context_chars == 0 { return Err("redaction limits must be greater than zero".into()); }
        if !(0.0..=1.0).contains(&self.routing.min_confidence) { return Err("routing.minConfidence must be between 0 and 1".into()); }
        if let Some(template) = &self.providers.jev.state_template {
            if !template.contains("{query}") || template.len() > 8_000 { return Err("providers.jev.stateTemplate must contain {query} and be at most 8000 bytes".into()); }
        }
        if let Some(questions) = &self.providers.jev.questions {
            let routing = &questions["routing_class"];
            let task = &questions["task_type"];
            let context = &questions["context"];
            if routing["type"] != "choice" || task["type"] != "choice" || context["type"] != "score" ||
                RoutingClass::ALL.iter().any(|class| !routing["criteria"][class.as_str()].is_string()) ||
                !task["criteria"].is_object() || !context["criteria"].is_array() {
                return Err("providers.jev.questions must keep the six routing classes, choice task_type and score context".into());
            }
        }
        for (class, level) in &self.effort.levels {
            if RoutingClass::parse(class).is_none() || level.trim().is_empty() { return Err(format!("effort.levels.{class} must name a routing class and an effort")); }
        }
        let mut ids = std::collections::HashSet::new();
        for rule in &self.rules {
            if rule.id.trim().is_empty() || !ids.insert(&rule.id) { return Err(format!("rule IDs must be unique: {}", rule.id)); }
            if let Some(confidence) = rule.min_confidence { if !(0.0..=1.0).contains(&confidence) { return Err(format!("rule {} has invalid confidence", rule.id)); } }
            if rule.target.model.is_none() && rule.target.gateway_target.is_none() { return Err(format!("rule {} has no target", rule.id)); }
        }
        let mut upstreams = std::collections::HashSet::new();
        for target in &self.gateway.upstream_allowlist { if !upstreams.insert(&target.id) { return Err(format!("gateway target IDs must be unique: {}", target.id)); } }
        Ok(())
    }

    pub fn safe_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "routingEnabled": self.routing.enabled,
            "provider": self.providers.selected,
            "jevConfigured": (self.providers.jev.endpoint.is_some() || self.providers.jev.base_url.is_some() || env::var("JEV_BASE_URL").is_ok()) && self.providers.jev.credential_env.as_ref().and_then(|k| env::var(k).ok()).is_some_and(|value| !value.trim().is_empty()),
            "layaConfigured": self.providers.laya.command.is_some() || self.providers.laya.endpoint.is_some() || self.providers.laya.base_url.is_some() || env::var("LAYA_BASE_URL").is_ok(),
            "swap": self.routing.swap,
            "rules": self.rules.len(),
            "gatewayEnabled": self.gateway.enabled,
            "gatewayAllowlistedTargets": self.gateway.upstream_allowlist.len(),
            "telemetryEnabled": self.telemetry.enabled,
            "debugCaptureOptIn": self.telemetry.debug_capture,
            "capturePrompts": self.telemetry.capture_prompts,
            "consentMode": format!("{:?}", self.consent.mode).to_lowercase()
        })
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("SYSTEM1ROUTER_CONFIG") { return Some(PathBuf::from(path)); }
    #[cfg(windows)]
    { env::var("APPDATA").ok().map(|p| PathBuf::from(p).join("system1router").join("config.json")) }
    #[cfg(not(windows))]
    { env::var("XDG_CONFIG_HOME").ok().map(|p| PathBuf::from(p).join("system1router").join("config.json")).or_else(|| env::var("HOME").ok().map(|p| PathBuf::from(p).join(".config/system1router/config.json"))) }
}

fn default_dotenv_path() -> Option<PathBuf> {
    #[cfg(windows)]
    { env::var("APPDATA").ok().map(|p| PathBuf::from(p).join("system1router").join(".env")) }
    #[cfg(not(windows))]
    { env::var("XDG_CONFIG_HOME").ok().map(|p| PathBuf::from(p).join("system1router/.env")).or_else(|| env::var("HOME").ok().map(|p| PathBuf::from(p).join(".config/system1router/.env"))) }
}

fn load_dotenv() -> Result<(), String> {
    let Some(path) = default_dotenv_path() else { return Ok(()); };
    if !path.exists() { return Ok(()); }
    load_dotenv_file(&path)
}

fn load_dotenv_file(path: &Path) -> Result<(), String> {
    let contents = fs::read_to_string(&path).map_err(|e| format!("read dotenv: {e}"))?;
    for (key, value) in parse_dotenv(&contents)? {
        if env::var_os(&key).is_none() {
            env::set_var(key, value);
        }
    }
    Ok(())
}

fn parse_dotenv(contents: &str) -> Result<Vec<(String, String)>, String> {
    contents.lines().enumerate().filter_map(|(index, line)| {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { return None; }
        Some(parse_dotenv_line(line, line_number))
    }).collect()
}

fn parse_dotenv_line(line: &str, line_number: usize) -> Result<(String, String), String> {
    let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
    let Some((raw_key, raw_value)) = line.split_once('=') else { return Err(format!("dotenv line {line_number} must contain '='")); };
    let key = raw_key.trim();
    let mut characters = key.chars();
    let valid_key = characters.next().is_some_and(|character| character.is_ascii_alphabetic() || character == '_') && characters.all(|character| character.is_ascii_alphanumeric() || character == '_');
    if !valid_key { return Err(format!("dotenv line {line_number} has an invalid variable name")); }
    Ok((key.into(), parse_dotenv_value(raw_value.trim(), line_number)?))
}

fn parse_dotenv_value(raw: &str, line_number: usize) -> Result<String, String> {
    if let Some(rest) = raw.strip_prefix('"') {
        let mut value = String::new();
        let mut escaped = false;
        let mut characters = rest.chars();
        let mut closed = false;
        while let Some(character) = characters.next() {
            if escaped {
                value.push(match character { 'n' => '\n', 'r' => '\r', 't' => '\t', '\\' => '\\', '"' => '"', other => other });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                closed = true;
                break;
            } else {
                value.push(character);
            }
        }
        let trailing = characters.collect::<String>();
        if !closed || escaped || (!trailing.trim().is_empty() && !trailing.trim_start().starts_with('#')) { return Err(format!("dotenv line {line_number} has an unterminated or malformed quoted value")); }
        return Ok(value);
    }
    if let Some(rest) = raw.strip_prefix('\'') {
        let Some(end) = rest.find('\'') else { return Err(format!("dotenv line {line_number} has an unterminated quoted value")); };
        let trailing = rest[end + 1..].trim();
        if !trailing.is_empty() && !trailing.starts_with('#') { return Err(format!("dotenv line {line_number} has trailing content after a quoted value")); }
        return Ok(rest[..end].to_string());
    }
    let mut value = raw.to_string();
    if let Some(index) = value.find('#').filter(|index| *index > 0 && value[..*index].chars().last().is_some_and(char::is_whitespace)) { value.truncate(index); }
    Ok(value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_dotenv_syntax() {
        let entries = parse_dotenv("# comment\nJEV_API_KEY='secret'\nexport JEV_BASE_URL=\"http://127.0.0.1:8787/decision\"\nTOKEN=abc#def\nMESSAGE=value # comment\n").unwrap();
        assert_eq!(entries, vec![
            ("JEV_API_KEY".into(), "secret".into()),
            ("JEV_BASE_URL".into(), "http://127.0.0.1:8787/decision".into()),
            ("TOKEN".into(), "abc#def".into()),
            ("MESSAGE".into(), "value".into()),
        ]);
    }

    #[test]
    fn rejects_malformed_dotenv_lines() {
        assert!(parse_dotenv("JEV_API_KEY\n").is_err());
        assert!(parse_dotenv("JEV-API-KEY=value\n").is_err());
        assert!(parse_dotenv("JEV_API_KEY=\"unterminated\n").is_err());
    }

    #[test]
    fn dotenv_fills_missing_values_without_overriding_environment() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(".env");
        let fallback_key = "SYSTEM1ROUTER_TEST_DOTENV_FALLBACK";
        let existing_key = "SYSTEM1ROUTER_TEST_DOTENV_EXISTING";
        env::remove_var(fallback_key);
        env::set_var(existing_key, "process-value");
        fs::write(&path, format!("{fallback_key}=dotenv-value\n{existing_key}=dotenv-value\n")).unwrap();

        load_dotenv_file(&path).unwrap();

        assert_eq!(env::var(fallback_key).unwrap(), "dotenv-value");
        assert_eq!(env::var(existing_key).unwrap(), "process-value");
        env::remove_var(fallback_key);
        env::remove_var(existing_key);
    }

    #[test]
    fn rejects_unusable_jev_customization() {
        let mut config = Config::default();
        config.providers.jev.state_template = Some("no user prompt".into());
        assert!(config.validate().is_err());
        config.providers.jev.state_template = Some("{query}".into());
        config.providers.jev.questions = Some(serde_json::json!({ "routing_class": {} }));
        assert!(config.validate().is_err());
    }

    #[test]
    fn effort_levels_override_defaults_per_class() {
        let config: Config = serde_json::from_str(r#"{"routing":{"swap":"effort"},"effort":{"levels":{"code_hard":"max"}}}"#).unwrap();
        assert_eq!(config.routing.swap, Swap::Effort);
        assert_eq!(config.effort.level(&RoutingClass::CodeHard), "max");
        assert_eq!(config.effort.level(&RoutingClass::QuestionSimple), "low");
        assert!(config.validate().is_ok());
        let unknown: Config = serde_json::from_str(r#"{"effort":{"levels":{"code_huge":"max"}}}"#).unwrap();
        assert!(unknown.validate().is_err());
        assert_eq!(Config::default().routing.swap, Swap::Model);
    }

    #[test]
    fn rejects_consent_timeouts_outside_the_ui_range() {
        let mut config = Config::default();
        config.timeouts.consent_ms = 0;
        assert!(config.validate().is_err());
        config.timeouts.consent_ms = 86_400_001;
        assert!(config.validate().is_err());
        config.timeouts.consent_ms = 120_000;
        assert!(config.validate().is_ok());
    }
}
