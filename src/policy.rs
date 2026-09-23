use crate::{config::{Config, PolicyRule}, protocol::{CapabilityLevel, ModelCatalog, NormalizedEvent, Recommendation, RoutingClassification, RoutingMode}};

#[derive(Debug, Clone, PartialEq)]
pub enum PolicyResolution { Recommendation(Recommendation), Bypass(String), Unavailable(String) }

pub fn resolve(classification: &RoutingClassification, event: &NormalizedEvent, config: &Config) -> PolicyResolution {
    let mut indexed: Vec<(usize, &PolicyRule)> = config.rules.iter().enumerate().collect();
    indexed.sort_by_key(|(index, rule)| (rule.priority, *index));
    let mut unavailable = false;
    for (_, rule) in indexed {
        if !matches_rule(rule, classification, event) { continue; }
        if let Some(recommendation) = materialize(rule.id.clone(), rule.mode.clone().unwrap_or(config.routing.default_mode.clone()), &rule.target, rule.reason.clone(), classification, event, &event.catalog, config) {
            return recommendation;
        }
        unavailable = true;
    }
    let default = &config.default_recommendation;
    if let Some(target) = &default.target {
        if let Some(recommendation) = materialize("default".into(), default.mode.clone().unwrap_or(config.routing.default_mode.clone()), target, default.reason.clone(), classification, event, &event.catalog, config) { return recommendation; }
        unavailable = true;
    }
    if unavailable { return PolicyResolution::Unavailable("matching target is absent from the host model catalog or gateway allowlist".into()); }
    PolicyResolution::Bypass("no eligible policy rule matched".into())
}

fn matches_rule(rule: &PolicyRule, classification: &RoutingClassification, event: &NormalizedEvent) -> bool {
    if rule.routing_class.as_ref() != Some(&classification.routing_class) && rule.routing_class.is_some() { return false; }
    if !rule.task_types.is_empty() && !rule.task_types.iter().any(|task| task == &classification.task_type) { return false; }
    if rule.host.as_ref().is_some_and(|host| host != &event.host) { return false; }
    if rule.project_id.as_ref().is_some_and(|project| event.context.project_id.as_ref() != Some(project)) { return false; }
    if rule.session_id.as_ref().is_some_and(|session| session != &event.session_id) { return false; }
    if rule.min_confidence.is_some_and(|minimum| classification.confidence.routing_class < minimum || classification.confidence.task_type < minimum) { return false; }
    if let Some(predicate) = &rule.predicate { if !predicate_matches(predicate, event) { return false; } }
    true
}

fn predicate_matches(predicate: &str, event: &NormalizedEvent) -> bool {
    let Some((key, expected)) = predicate.split_once('=') else { return false; };
    match key {
        "branch" => event.context.branch.as_deref() == Some(expected),
        "repository" => event.context.repository.as_deref() == Some(expected),
        "cwd" => event.cwd.as_deref() == Some(expected),
        _ => false,
    }
}

fn materialize(rule_id: String, mode: RoutingMode, target: &crate::protocol::PolicyTarget, reason: Option<String>, classification: &RoutingClassification, event: &NormalizedEvent, catalog: &ModelCatalog, config: &Config) -> Option<PolicyResolution> {
    if mode == RoutingMode::Gateway {
        let id = target.gateway_target.as_ref()?;
        let gateway = config.gateway.upstream_allowlist.iter().find(|candidate| &candidate.id == id).cloned()?;
        return Some(PolicyResolution::Recommendation(Recommendation { rule_id, mode, provider: None, model: None, variant: gateway.variant.clone(), gateway_target: Some(gateway), reason: reason.unwrap_or_else(|| "gateway policy match".into()), routing_class: classification.routing_class.clone(), task_type: classification.task_type.clone(), confidence: classification.confidence.clone() }));
    }
    let model = target.model.as_ref()?;
    let provider = target.provider.clone().unwrap_or_else(|| "default".into());
    let available = catalog.models.iter().find(|candidate| candidate.provider == provider && candidate.model == *model);
    if available.is_none() { return None; }
    if let Some(variant) = &target.variant { if available.is_some_and(|candidate| !candidate.variants.is_empty() && !candidate.variants.contains(variant)) { return None; } }
    let negotiated = negotiate_mode(mode, &event.capabilities, config);
    let explanation = reason.unwrap_or_else(|| format!("Rule {rule_id} matched {} ({})", classification.routing_class, classification.task_type));
    Some(PolicyResolution::Recommendation(Recommendation { rule_id, mode: negotiated, provider: Some(provider), model: Some(model.clone()), variant: target.variant.clone(), gateway_target: None, reason: explanation, routing_class: classification.routing_class.clone(), task_type: classification.task_type.clone(), confidence: classification.confidence.clone() }))
}

fn negotiate_mode(mode: RoutingMode, capabilities: &crate::protocol::CapabilityHints, config: &Config) -> RoutingMode {
    match mode {
        RoutingMode::FullOverride if capabilities.level != CapabilityLevel::FullOverride && !capabilities.model_mutation => if config.gateway.enabled && capabilities.gateway { RoutingMode::Gateway } else { RoutingMode::ConsentOnly },
        RoutingMode::Gateway if !config.gateway.enabled || !capabilities.gateway => RoutingMode::ConsentOnly,
        other => other,
    }
}

pub fn target_available(recommendation: &Recommendation, event: &NormalizedEvent, config: &Config) -> bool {
    if recommendation.mode == RoutingMode::Gateway { return recommendation.gateway_target.as_ref().is_some_and(|target| config.gateway.upstream_allowlist.iter().any(|allowed| allowed.id == target.id)); }
    recommendation.model.as_ref().is_some_and(|model| event.catalog.models.is_empty() || event.catalog.models.iter().any(|item| item.model == *model && recommendation.provider.as_ref().is_none_or(|provider| item.provider == *provider)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::*;
    fn fixture() -> (Config, NormalizedEvent, RoutingClassification) {
        let mut config = Config::default(); config.routing.enabled = true; config.rules.push(PolicyRule { id: "hard".into(), priority: 0, routing_class: Some(RoutingClass::CodeHard), task_types: vec!["code-generation".into()], host: None, project_id: None, session_id: None, min_confidence: Some(0.8), mode: Some(RoutingMode::Advisory), predicate: None, target: PolicyTarget { provider: Some("p".into()), model: Some("m".into()), variant: None, gateway_target: None }, reason: Some("hard work".into()) });
        let event = NormalizedEvent { protocol_version: "1".into(), host: "test".into(), event: "user".into(), session_id: "s".into(), turn_id: None, cwd: None, request: RequestPayload::default(), current_model: None, context: BoundedContext::default(), catalog: ModelCatalog { models: vec![ModelRef { provider: "p".into(), model: "m".into(), variants: vec![] }], gateway_targets: vec![] }, capabilities: CapabilityHints::default(), correlation_id: Some("c".into()), metadata: serde_json::Value::Null };
        let classification = RoutingClassification { routing_class: RoutingClass::CodeHard, task_type: "code-generation".into(), context_signals: vec![], confidence: ClassificationConfidence { routing_class: 0.9, task_type: 0.9, context: 0.9 }, provider_metadata: ProviderMetadata { provider: "x".into(), version: "1".into(), model: None, latency_ms: None }, correlation_id: "c".into() };
        (config, event, classification)
    }
    #[test] fn first_matching_rule_wins() { let (config, event, classification) = fixture(); assert!(matches!(resolve(&classification, &event, &config), PolicyResolution::Recommendation(_))); }
    #[test] fn reports_missing_catalog_target() { let (config, mut event, classification) = fixture(); event.catalog.models.clear(); assert!(matches!(resolve(&classification, &event, &config), PolicyResolution::Unavailable(_))); }
}
