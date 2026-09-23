use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const PROTOCOL_VERSION: &str = "1";
pub const EXIT_OK: i32 = 0;
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
}

impl fmt::Display for RoutingClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

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
