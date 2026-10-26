use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const PROTOCOL_VERSION: &str = "1";
pub const EXIT_MALFORMED_EVENT: i32 = 10;
pub const EXIT_UNSUPPORTED_VERSION: i32 = 11;
pub const EXIT_STARTUP_FAILURE: i32 = 12;
pub const EXIT_TIMEOUT: i32 = 13;
pub const EXIT_INTERNAL_ERROR: i32 = 14;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityLevel {
    ObserveOnly,
    ConsentOnly,
    FullOverride,
    Gateway,
}

impl Default for CapabilityLevel {
    fn default() -> Self {
        Self::ObserveOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingMode {
    Advisory,
    ConsentOnly,
    FullOverride,
    Gateway,
}

impl Default for RoutingMode {
    fn default() -> Self {
        Self::Advisory
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingClass {
    QuestionSimple,
    QuestionModerate,
    QuestionHard,
    CodeSimple,
    CodeModerate,
    CodeHard,
}

impl RoutingClass {
    pub const ALL: [Self; 6] = [
        Self::QuestionSimple,
        Self::QuestionModerate,
        Self::QuestionHard,
        Self::CodeSimple,
        Self::CodeModerate,
        Self::CodeHard,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::QuestionSimple => "question_simple",
            Self::QuestionModerate => "question_moderate",
            Self::QuestionHard => "question_hard",
            Self::CodeSimple => "code_simple",
            Self::CodeModerate => "code_moderate",
            Self::CodeHard => "code_hard",
        }
    }

    pub fn is_code(&self) -> bool { matches!(self, Self::CodeSimple | Self::CodeModerate | Self::CodeHard) }

    /// Difficulty tier: 0 simple, 1 moderate, 2 hard.
    pub fn tier(&self) -> usize {
        match self {
            Self::QuestionSimple | Self::CodeSimple => 0,
            Self::QuestionModerate | Self::CodeModerate => 1,
            Self::QuestionHard | Self::CodeHard => 2,
        }
    }

    pub fn from_parts(code: bool, tier: usize) -> Self {
        match (code, tier) {
            (false, 0) => Self::QuestionSimple,
            (false, 1) => Self::QuestionModerate,
            (false, _) => Self::QuestionHard,
            (true, 0) => Self::CodeSimple,
            (true, 1) => Self::CodeModerate,
            (true, _) => Self::CodeHard,
        }
    }

    pub fn parse(value: &str) -> Option<Self> { Self::ALL.into_iter().find(|class| class.as_str() == value) }
}

impl fmt::Display for RoutingClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reasoning efforts from least to most, as hosts name them (Claude Code
/// `/effort`, OpenAI `reasoning_effort`, OpenCode variants).
pub const EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

pub fn effort_rank(effort: &str) -> Option<usize> { EFFORTS.iter().position(|level| *level == effort) }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelState {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub variants: Vec<String>,
    /// Per-million-token prices reported by the host. Zero or absent means unknown
    /// (for example subscription-billed models), which disables cost reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelCost {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayTarget {
    pub id: String,
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub variant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    #[serde(default)]
    pub models: Vec<ModelRef>,
    #[serde(default)]
    pub gateway_targets: Vec<GatewayTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoundedContext {
    #[serde(default)]
    pub conversation_summary: Option<String>,
    #[serde(default)]
    pub project_summary: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    /// Tokens already in the conversation's prompt (including cached tokens),
    /// as last reported by the host. Used to price the cache loss of a switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    /// Milliseconds since the conversation's last response. Past the prompt
    /// cache's lifetime a switch loses nothing cached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_ms: Option<u64>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityHints {
    #[serde(default)]
    pub level: CapabilityLevel,
    #[serde(default)]
    pub model_mutation: bool,
    #[serde(default)]
    pub interactive_consent: bool,
    #[serde(default)]
    pub gateway: bool,
    #[serde(default)]
    pub protocol_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequestPayload {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub attachments: Vec<Value>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub tool_arguments: Value,
    #[serde(default)]
    pub original_payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedEvent {
    pub protocol_version: String,
    pub host: String,
    pub event: String,
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    pub request: RequestPayload,
    #[serde(default)]
    pub current_model: Option<ModelState>,
    #[serde(default)]
    pub context: BoundedContext,
    #[serde(default)]
    pub catalog: ModelCatalog,
    #[serde(default)]
    pub capabilities: CapabilityHints,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationConfidence {
    pub routing_class: f32,
    pub task_type: f32,
    #[serde(default)]
    pub context: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMetadata {
    pub provider: String,
    pub version: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingClassification {
    pub routing_class: RoutingClass,
    pub task_type: String,
    #[serde(default)]
    pub context_signals: Vec<String>,
    /// Optional provider distribution over the six classes. When present, the
    /// router derives the class from it instead of trusting the argmax. The
    /// local model gives its distribution over efforts instead (telemetry only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_probabilities: Option<std::collections::BTreeMap<String, f32>>,
    pub confidence: ClassificationConfidence,
    pub provider_metadata: ProviderMetadata,
    pub correlation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTarget {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub gateway_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub rule_id: String,
    pub mode: RoutingMode,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub gateway_target: Option<GatewayTarget>,
    pub reason: String,
    pub routing_class: RoutingClass,
    pub task_type: String,
    pub confidence: ClassificationConfidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsentOutcome {
    UseSuggestedModel,
    KeepCurrentModel,
    Unavailable,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Allow,
    Block,
    Ask,
    Bypass,
    Route,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub details: Value,
}

impl Diagnostic {
    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), severity: "info".into(), details: Value::Null }
    }

    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), severity: "warning".into(), details: Value::Null }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), severity: "error".into(), details: Value::Null }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedDecision {
    pub protocol_version: String,
    pub correlation_id: String,
    pub outcome: DecisionOutcome,
    #[serde(default)]
    pub recommendation: Option<Recommendation>,
    #[serde(default)]
    pub consent: Option<ConsentOutcome>,
    pub capability: CapabilityLevel,
    pub diagnostic: Diagnostic,
    #[serde(default)]
    pub telemetry: Option<RoutingEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingEvent {
    pub correlation_id: String,
    pub host: String,
    pub routing_class: Option<RoutingClass>,
    #[serde(default)]
    pub task_type: Option<String>,
    pub mode: RoutingMode,
    pub policy_outcome: String,
    pub latency_ms: u64,
    pub error_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
    /// The effort the request runs at, after an answered effort swap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The provider's class distribution and model, to compare class
    /// selections offline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<std::collections::BTreeMap<String, f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_model: Option<String>,
    /// The redacted request and context the provider saw
    /// (`telemetry.capturePrompts`), to label decisions afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<Value>,
    #[serde(default)]
    pub debug_capture: Option<Value>,
}

pub fn correlation_id(event: &NormalizedEvent) -> String {
    if let Some(id) = &event.correlation_id {
        if !id.trim().is_empty() {
            return id.clone();
        }
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    format!("s1r-{millis:x}-{:x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

pub fn supported_protocol(version: &str) -> bool {
    version == PROTOCOL_VERSION
}
