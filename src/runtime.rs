use crate::{classification::{classify_event, DecisionProvider, ProviderError}, config::{Config, ConsentMode}, policy::{alternative_possible, resolve, switch_discards_cache_savings, target_available, PolicyResolution}, protocol::{correlation_id, supported_protocol, ConsentOutcome, DecisionOutcome, Diagnostic, NormalizedDecision, NormalizedEvent, RoutingEvent, RoutingMode}, telemetry};
use std::time::Instant;

pub fn process_event(event: &NormalizedEvent, config: &Config, provider: &dyn DecisionProvider) -> NormalizedDecision {
    let started = Instant::now();
    let id = correlation_id(event);
    let capability = event.capabilities.level.clone();
    let base = |outcome: DecisionOutcome, diagnostic: Diagnostic, classification: Option<&crate::protocol::RoutingClassification>, mode: RoutingMode, policy_outcome: &str| NormalizedDecision {
        protocol_version: crate::protocol::PROTOCOL_VERSION.into(), correlation_id: id.clone(), outcome, recommendation: None, consent: None, capability: capability.clone(), diagnostic, telemetry: Some(RoutingEvent { correlation_id: id.clone(), host: event.host.clone(), routing_class: classification.map(|value| value.routing_class.clone()), task_type: classification.map(|value| value.task_type.clone()), mode, policy_outcome: policy_outcome.into(), latency_ms: started.elapsed().as_millis() as u64, error_category: None, model: None, suggested_model: None, accepted: None, debug_capture: None }),
    };

    if !supported_protocol(&event.protocol_version) {
        return base(DecisionOutcome::Bypass, Diagnostic::warning("unsupported_version", "unsupported normalized hook protocol"), None, RoutingMode::Advisory, "unsupported-version");
    }
    if recursion_guarded(event) {
        return base(DecisionOutcome::Bypass, Diagnostic::info("recursion_guard", "routing was bypassed for a router-generated event"), None, RoutingMode::Advisory, "recursion-guard");
    }
    if !config.routing.enabled {
        return base(DecisionOutcome::Bypass, Diagnostic::info("disabled", "routing is disabled"), None, RoutingMode::Advisory, "disabled");
    }
    // Avoid paying provider latency (and sending text) when the answer cannot matter.
    if event.request.query.trim().is_empty() {
        return base(DecisionOutcome::Bypass, Diagnostic::info("empty_query", "the request has no text to classify"), None, RoutingMode::Advisory, "empty-query");
    }
    if !alternative_possible(event, config) {
        return base(DecisionOutcome::Bypass, Diagnostic::info("no_alternative", "no applicable rule targets a model other than the current one"), None, RoutingMode::Advisory, "no-alternative");
    }
    let mut classified_event = event.clone();
    classified_event.correlation_id = Some(id.clone());
    let classification = match classify_event(provider, &classified_event, config) {
        Ok(value) => value,
        Err(error) => return classification_failure(base, error, config.routing.fail_open),
    };
    if !config.routing.allowed_task_types.is_empty() && !config.routing.allowed_task_types.iter().any(|task| task == &classification.task_type) {
        return base(DecisionOutcome::Bypass, Diagnostic::info("task_not_eligible", "task type is not enabled for routing"), Some(&classification), RoutingMode::Advisory, "task-ineligible");
    }
    let resolution = resolve(&classification, event, config);
    let recommendation = match resolution {
        PolicyResolution::Recommendation(value) => value,
        PolicyResolution::Bypass(reason) => return base(DecisionOutcome::Bypass, Diagnostic::info("policy_bypass", reason), Some(&classification), RoutingMode::Advisory, "bypass"),
        PolicyResolution::Unavailable(reason) => return base(DecisionOutcome::Bypass, Diagnostic::warning("policy_unavailable", reason), Some(&classification), RoutingMode::Advisory, "unavailable"),
    };
    if !target_available(&recommendation, event, config) {
        return base(DecisionOutcome::Bypass, Diagnostic::warning("target_unavailable", "the selected target is not available in the current catalog or gateway allowlist"), Some(&classification), recommendation.mode.clone(), "target-unavailable");
    }
    if config.routing.cache_aware {
        if let Some(reason) = switch_discards_cache_savings(&recommendation, event) {
            return base(DecisionOutcome::Allow, Diagnostic::info("cache_sticky", reason), Some(&classification), recommendation.mode.clone(), "cache-sticky");
        }
    }
    // Interactive hosts ask the user themselves; recording a guessed answer
    // here would make every such request look declined in telemetry.
    if config.consent.mode == ConsentMode::Interactive && event.capabilities.interactive_consent && std::env::var("SYSTEM1ROUTER_CONSENT").is_err() {
        let mut decision = base(DecisionOutcome::Ask, Diagnostic::info("consent_requested", "the host will ask the user to choose"), Some(&classification), recommendation.mode.clone(), "consent-requested");
        decision.recommendation = Some(recommendation);
        return decision;
    }
    let consent = choose_consent(event, config);
    let accepted = matches!(consent, ConsentOutcome::UseSuggestedModel);
    let mut decision = base(if accepted && matches!(recommendation.mode, RoutingMode::FullOverride | RoutingMode::Gateway) { DecisionOutcome::Route } else if matches!(consent, ConsentOutcome::Unavailable | ConsentOutcome::TimedOut) && config.consent.mode == ConsentMode::Interactive { DecisionOutcome::Ask } else { DecisionOutcome::Allow }, if accepted { Diagnostic::info("recommendation_accepted", "recommendation accepted for this request") } else { Diagnostic::info("recommendation_declined", "request keeps the captured current model") }, Some(&classification), recommendation.mode.clone(), if accepted { "accepted" } else { "declined" });
    decision.recommendation = Some(recommendation);
    decision.consent = Some(consent);
    decision
}

pub fn process_event_safe(event: &NormalizedEvent, config: &Config, provider: &dyn DecisionProvider) -> NormalizedDecision {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process_event(event, config, provider))).unwrap_or_else(|_| NormalizedDecision { protocol_version: crate::protocol::PROTOCOL_VERSION.into(), correlation_id: correlation_id(event), outcome: if config.routing.fail_open { DecisionOutcome::Bypass } else { DecisionOutcome::Block }, recommendation: None, consent: None, capability: event.capabilities.level.clone(), diagnostic: Diagnostic::error("internal_error", "routing failed internally; no model recommendation was applied"), telemetry: None })
}

fn classification_failure<F>(mut base: F, error: ProviderError, fail_open: bool) -> NormalizedDecision where F: FnMut(DecisionOutcome, Diagnostic, Option<&crate::protocol::RoutingClassification>, RoutingMode, &str) -> NormalizedDecision {
    let category = match error { ProviderError::Timeout => "provider-timeout", ProviderError::Malformed(_) => "malformed-response", ProviderError::LowConfidence => "low-confidence", ProviderError::Unavailable(_) => "provider-unavailable", ProviderError::Cancelled => "cancelled", ProviderError::Transport(_) => "provider-error" };
    let provider_error = error.to_string();
    let mut result = base(if fail_open { DecisionOutcome::Bypass } else { DecisionOutcome::Block }, Diagnostic::warning(category, "classification is unavailable; no model recommendation was applied"), None, RoutingMode::Advisory, "classification-unavailable");
    result.diagnostic.details = serde_json::json!({ "providerError": provider_error });
    if let Some(telemetry) = &mut result.telemetry { telemetry.error_category = Some(category.into()); }
    result
}

fn choose_consent(event: &NormalizedEvent, config: &Config) -> ConsentOutcome {
    match config.consent.mode {
        ConsentMode::AlwaysAccept => ConsentOutcome::UseSuggestedModel,
        ConsentMode::AlwaysDecline | ConsentMode::Disabled => ConsentOutcome::KeepCurrentModel,
        ConsentMode::Interactive => {
            if !event.capabilities.interactive_consent { return if config.consent.decline_on_timeout { ConsentOutcome::Unavailable } else { ConsentOutcome::UseSuggestedModel }; }
            match std::env::var("SYSTEM1ROUTER_CONSENT").ok().as_deref() {
                Some("accept") | Some("use suggested model") => ConsentOutcome::UseSuggestedModel,
                Some("decline") | Some("keep current model") => ConsentOutcome::KeepCurrentModel,
                _ => ConsentOutcome::TimedOut,
            }
        }
    }
}

fn recursion_guarded(event: &NormalizedEvent) -> bool {
    event.event.starts_with("system1router.") || event.metadata.get("system1routerDepth").and_then(|value| value.as_u64()).is_some_and(|depth| depth > 0)
}

pub fn record_decision(decision: &NormalizedDecision, event: &NormalizedEvent, config: &Config) {
    telemetry::record(decision, event, config);
}
