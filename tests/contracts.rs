use serde_json::json;
use system1router_core::{adapters::{normalize, translate_claude_model_switch, translate_codex_tool_decision, translate_decision, Host}, classification::{validate_classification, StaticProvider}, config::{Config, PolicyRule}, process_event, protocol::*};

fn classification(class: RoutingClass) -> RoutingClassification {
    RoutingClassification { routing_class: class, task_type: "code-generation".into(), context_signals: vec!["project".into()], confidence: ClassificationConfidence { routing_class: 0.95, task_type: 0.9, context: 0.85 }, provider_metadata: ProviderMetadata { provider: "fixture".into(), version: "1".into(), model: Some("fixture".into()), latency_ms: Some(1) }, correlation_id: "fixture".into() }
}

#[test]
fn every_canonical_class_validates() {
    for class in RoutingClass::ALL { assert!(validate_classification(classification(class), "correlation").is_ok()); }
}

#[test]
fn host_translators_normalize_and_drop_unsupported_switches() {
    for host in [Host::ClaudeCode, Host::Codex, Host::CopilotCli, Host::CopilotVsCode, Host::OpenCode, Host::Generic] {
        let event = normalize(host, json!({ "prompt": "review this", "sessionId": "s", "model": "current", "provider": "p" }), Some("/tmp/project".into()));
        assert_eq!(event.host, host.as_str());
        let decision = NormalizedDecision { protocol_version: "1".into(), correlation_id: "c".into(), outcome: DecisionOutcome::Route, recommendation: Some(Recommendation { rule_id: "r".into(), mode: RoutingMode::FullOverride, provider: Some("p".into()), model: Some("suggested".into()), variant: None, gateway_target: None, reason: "test".into(), routing_class: RoutingClass::CodeSimple, task_type: "code-generation".into(), confidence: classification(RoutingClass::CodeSimple).confidence }), consent: Some(ConsentOutcome::UseSuggestedModel), capability: CapabilityLevel::ConsentOnly, diagnostic: Diagnostic::info("test", "test"), telemetry: None };
        let translated = translate_decision(host, &decision);
        assert!(!translated.to_string().contains("fullOverride"));
    }
}

#[test]
fn policy_and_runtime_preserve_declined_request() {
    let mut config = Config::default();
    config.routing.enabled = true;
    config.consent.mode = system1router_core::config::ConsentMode::AlwaysDecline;
    config.rules.push(PolicyRule { id: "simple".into(), priority: 0, routing_class: Some(RoutingClass::CodeSimple), task_types: vec![], host: None, project_id: None, session_id: None, min_confidence: None, mode: Some(RoutingMode::Advisory), predicate: None, target: PolicyTarget { provider: Some("p".into()), model: Some("m".into()), variant: None, gateway_target: None }, reason: Some("fixture".into()) });
    let mut event = normalize(Host::Generic, json!({ "prompt": "small change", "sessionId": "s", "model": "current", "provider": "p" }), None);
    event.catalog.models.push(ModelRef { provider: "p".into(), model: "m".into(), variants: vec![] });
    event.capabilities = CapabilityHints { level: CapabilityLevel::ConsentOnly, model_mutation: false, interactive_consent: false, gateway: false, protocol_version: Some("1".into()) };
    let provider = StaticProvider { result: Ok(classification(RoutingClass::CodeSimple)) };
    let decision = process_event(&event, &config, &provider);
    assert_eq!(decision.outcome, DecisionOutcome::Allow);
    assert_eq!(decision.consent, Some(ConsentOutcome::KeepCurrentModel));
    assert!(decision.correlation_id.starts_with("s1r-"));
}

#[test]
fn unavailable_provider_fails_open_without_recommendation() {
    let config = Config::default();
    let event = normalize(Host::Generic, json!({ "prompt": "question", "sessionId": "s" }), None);
    let provider = StaticProvider { result: Err(system1router_core::ProviderError::Timeout) };
    let decision = process_event(&event, &config, &provider);
    assert_eq!(decision.outcome, DecisionOutcome::Bypass);
    assert!(decision.recommendation.is_none());
}

#[test]
fn protocol_serialization_is_single_object() {
    let event = normalize(Host::Generic, json!({ "prompt": "hello", "sessionId": "s" }), None);
    let encoded = serde_json::to_string(&event).unwrap();
    assert!(encoded.starts_with('{'));
    assert!(encoded.contains("protocolVersion"));
}

#[test]
fn host_specific_hooks_do_not_claim_model_switching() {
    let decision = NormalizedDecision { protocol_version: "1".into(), correlation_id: "c".into(), outcome: DecisionOutcome::Allow, recommendation: None, consent: None, capability: CapabilityLevel::ConsentOnly, diagnostic: Diagnostic::info("ok", "normal"), telemetry: None };
    let claude = translate_claude_model_switch(&decision);
    assert_eq!(claude["system1router"]["switchInitiated"], false);
    let codex = translate_codex_tool_decision(&decision, Some(json!({ "path": "safe" })));
    assert_eq!(codex["updatedInput"]["path"], "safe");
}

#[test]
fn documented_configuration_example_is_valid() {
    let config: Config = serde_json::from_str(include_str!("../config.example.json")).unwrap();
    config.validate().unwrap();
    assert_eq!(config.providers.jev.base_url.as_deref(), Some("https://api.typesafe.ai/v1/systemone"));
    assert_eq!(config.providers.jev.credential_env.as_deref(), Some("JEV_API_KEY"));
    assert_eq!(config.providers.laya.base_url.as_deref(), Some("http://127.0.0.1:8788/classify"));
}
