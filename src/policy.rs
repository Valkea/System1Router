use crate::{config::{Config, PolicyRule}, protocol::{CapabilityLevel, ModelCatalog, NormalizedEvent, PolicyTarget, Recommendation, RoutingClassification, RoutingMode}};

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

/// True when some rule applicable to this host/project/session could move the
/// request away from the current model. When false, classifying the request
/// cannot change anything, so the provider call is skipped.
pub fn alternative_possible(event: &NormalizedEvent, config: &Config) -> bool {
    let Some(current) = &event.current_model else { return true; };
    let differs = |target: &PolicyTarget| {
        target.gateway_target.is_some()
            || target.model.as_deref() != Some(current.model.as_str())
            || target.provider.as_deref().is_some_and(|provider| provider != current.provider)
            || target.variant.as_ref().is_some_and(|variant| Some(variant) != current.variant.as_ref())
    };
    config.rules.iter().any(|rule| matches_scope(rule, event) && differs(&rule.target))
        || config.default_recommendation.target.as_ref().is_some_and(differs)
}

/// Estimates whether switching models costs more on the next request than it
/// saves. A different model cannot reuse the conversation's prompt cache, so
/// a "cheaper" model re-reads the whole context at its full input price while
/// the current model would read it at the cached price. Only downgrades are
/// suppressed: upgrades are about quality, and unknown (zero) prices are
/// never guessed.
pub fn switch_discards_cache_savings(recommendation: &Recommendation, event: &NormalizedEvent) -> Option<String> {
    let tokens = event.context.context_tokens.filter(|tokens| *tokens > 0)? as f64;
    let current = event.current_model.as_ref()?;
    let provider = recommendation.provider.as_deref()?;
    let model = recommendation.model.as_deref()?;
    if current.provider == provider && current.model == model { return None; }
    let price = |provider: &str, model: &str| event.catalog.models.iter().find(|item| item.provider == provider && item.model == model).and_then(|item| item.cost.clone()).filter(|cost| cost.input > 0.0);
    let current_cost = price(&current.provider, &current.model)?;
    let suggested_cost = price(provider, model)?;
    if suggested_cost.input >= current_cost.input { return None; }
    let stay = tokens * if current_cost.cache_read > 0.0 { current_cost.cache_read } else { current_cost.input };
    let switch = tokens * suggested_cost.input;
    (switch >= stay).then(|| format!("keeping {} reuses {} cached context tokens; switching would re-read them at full price (${:.4} vs ${:.4})", current.model, tokens as u64, switch / 1e6, stay / 1e6))
}

fn matches_scope(rule: &PolicyRule, event: &NormalizedEvent) -> bool {
    if rule.host.as_ref().is_some_and(|host| host != &event.host) { return false; }
    if rule.project_id.as_ref().is_some_and(|project| event.context.project_id.as_ref() != Some(project)) { return false; }
    if rule.session_id.as_ref().is_some_and(|session| session != &event.session_id) { return false; }
    if let Some(predicate) = &rule.predicate { if !predicate_matches(predicate, event) { return false; } }
    true
}

fn matches_rule(rule: &PolicyRule, classification: &RoutingClassification, event: &NormalizedEvent) -> bool {
    if rule.routing_class.as_ref() != Some(&classification.routing_class) && rule.routing_class.is_some() { return false; }
    if !rule.task_types.is_empty() && !rule.task_types.iter().any(|task| task == &classification.task_type) { return false; }
    if !matches_scope(rule, event) { return false; }
    if rule.min_confidence.is_some_and(|minimum| classification.confidence.routing_class < minimum || classification.confidence.task_type < minimum) { return false; }
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
    let negotiated = negotiate_mode(mode, &event.capabilities, config);
    // A model is only ever applied after catalog verification. Advisory hosts
    // that expose no catalog may still show the user's configured target.
    if !catalog.models.is_empty() || negotiated == RoutingMode::FullOverride {
        let available = catalog.models.iter().find(|candidate| candidate.provider == provider && candidate.model == *model)?;
        if let Some(variant) = &target.variant { if !available.variants.is_empty() && !available.variants.contains(variant) { return None; } }
    }
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
        let event = NormalizedEvent { protocol_version: "1".into(), host: "test".into(), event: "user".into(), session_id: "s".into(), turn_id: None, cwd: None, request: RequestPayload::default(), current_model: None, context: BoundedContext::default(), catalog: ModelCatalog { models: vec![ModelRef { provider: "p".into(), model: "m".into(), variants: vec![], cost: None }], gateway_targets: vec![] }, capabilities: CapabilityHints::default(), correlation_id: Some("c".into()), metadata: serde_json::Value::Null };
        let classification = RoutingClassification { routing_class: RoutingClass::CodeHard, task_type: "code-generation".into(), context_signals: vec![], class_probabilities: None, confidence: ClassificationConfidence { routing_class: 0.9, task_type: 0.9, context: 0.9 }, provider_metadata: ProviderMetadata { provider: "x".into(), version: "1".into(), model: None, latency_ms: None }, correlation_id: "c".into() };
        (config, event, classification)
    }
    #[test] fn first_matching_rule_wins() { let (config, event, classification) = fixture(); assert!(matches!(resolve(&classification, &event, &config), PolicyResolution::Recommendation(_))); }
    #[test] fn skips_classification_when_no_rule_can_change_the_model() {
        let (config, mut event, _) = fixture();
        assert!(alternative_possible(&event, &config));
        event.current_model = Some(ModelState { provider: "p".into(), model: "m".into(), variant: Some("any".into()), scope: None });
        assert!(!alternative_possible(&event, &config));
        event.host = "other".into();
        let mut scoped = config.clone();
        scoped.rules[0].host = Some("test".into());
        scoped.rules[0].target.model = Some("elsewhere".into());
        assert!(!alternative_possible(&event, &scoped));
    }
    #[test] fn advisory_hosts_without_a_catalog_still_get_advice_but_overrides_do_not() {
        let (config, mut event, classification) = fixture();
        event.catalog.models.clear();
        event.capabilities.level = CapabilityLevel::ConsentOnly;
        assert!(matches!(resolve(&classification, &event, &config), PolicyResolution::Recommendation(_)));
        let mut override_config = config.clone();
        override_config.rules[0].mode = Some(RoutingMode::FullOverride);
        event.capabilities = CapabilityHints { level: CapabilityLevel::FullOverride, model_mutation: true, ..Default::default() };
        assert!(matches!(resolve(&classification, &event, &override_config), PolicyResolution::Unavailable(_)));
    }
    #[test] fn cache_guard_only_blocks_downgrades_that_cost_more() {
        let (_, mut event, _) = fixture();
        let cost = |input: f64, cache_read: f64| Some(ModelCost { input, output: 0.0, cache_read });
        event.catalog.models = vec![
            ModelRef { provider: "p".into(), model: "big".into(), variants: vec![], cost: cost(5.0, 0.5) },
            ModelRef { provider: "p".into(), model: "small".into(), variants: vec![], cost: cost(1.0, 0.1) },
            ModelRef { provider: "p".into(), model: "free".into(), variants: vec![], cost: cost(0.0, 0.0) },
        ];
        event.current_model = Some(ModelState { provider: "p".into(), model: "big".into(), variant: None, scope: None });
        event.context.context_tokens = Some(100_000);
        let rec = |model: &str| Recommendation { rule_id: "r".into(), mode: RoutingMode::FullOverride, provider: Some("p".into()), model: Some(model.into()), variant: None, gateway_target: None, reason: String::new(), routing_class: RoutingClass::CodeSimple, task_type: "t".into(), confidence: ClassificationConfidence { routing_class: 1.0, task_type: 1.0, context: 1.0 } };
        assert!(switch_discards_cache_savings(&rec("small"), &event).is_some(), "1.0/M uncached costs more than 0.5/M cached");
        assert!(switch_discards_cache_savings(&rec("free"), &event).is_none(), "unknown prices are never guessed");
        event.context.context_tokens = Some(0);
        assert!(switch_discards_cache_savings(&rec("small"), &event).is_none(), "a fresh conversation has no cache to lose");
        event.context.context_tokens = Some(100_000);
        event.current_model.as_mut().unwrap().model = "small".into();
        assert!(switch_discards_cache_savings(&rec("big"), &event).is_none(), "upgrades are never suppressed");
    }
    #[test] fn reports_missing_catalog_target() { let (config, mut event, classification) = fixture(); event.catalog.models[0].model = "other".into(); assert!(matches!(resolve(&classification, &event, &config), PolicyResolution::Unavailable(_))); }
}
