use crate::{config::{Config, PolicyRule}, protocol::{effort_rank, CapabilityLevel, ModelCatalog, ModelCost, ModelState, NormalizedEvent, PolicyTarget, Recommendation, RoutingClass, RoutingClassification, RoutingMode}};

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
    let differs = |target: &PolicyTarget| {
        let Some(current) = &event.current_model else { return true; };
        target.gateway_target.is_some()
            || target.model.as_deref() != Some(current.model.as_str())
            || target.provider.as_deref().is_some_and(|provider| provider != current.provider)
            || target.variant.as_ref().is_some_and(|variant| Some(variant) != current.variant.as_ref())
    };
    let usable = |target: &PolicyTarget| usable_without_catalog(target, event);
    config.rules.iter().any(|rule| matches_scope(rule, event) && usable(&rule.target) && differs(&rule.target))
        || config.default_recommendation.target.as_ref().is_some_and(|target| usable(target) && differs(target))
}

/// Without a catalog the host's providers are unknown; the only provider known
/// to work is the current one, so targets on other providers are ignored
/// (for example OpenAI rules while running Claude Code).
fn usable_without_catalog(target: &PolicyTarget, event: &NormalizedEvent) -> bool {
    if !event.catalog.models.is_empty() || target.gateway_target.is_some() { return true; }
    let host_provider = event.current_model.as_ref().map(|current| current.provider.as_str())
        .or_else(|| event.metadata.get("hostProvider").and_then(serde_json::Value::as_str));
    match (host_provider, &target.provider) {
        (Some(host), Some(provider)) => provider == host,
        _ => true,
    }
}

/// Estimates whether switching models costs more on the next request than it
/// saves. A different model cannot reuse the conversation's prompt cache, so
/// a "cheaper" model re-reads the whole context at its full input price while
/// the current model would read it at the cached price. Only downgrades are
/// suppressed: upgrades are about quality, and unknown (zero) prices are
/// never guessed.
pub fn switch_discards_cache_savings(recommendation: &Recommendation, event: &NormalizedEvent, fallback: &dyn Fn(&str, &str) -> Option<ModelCost>) -> Option<String> {
    let tokens = event.context.context_tokens.filter(|tokens| *tokens > 0)? as f64;
    let current = event.current_model.as_ref()?;
    let provider = recommendation.provider.as_deref()?;
    let model = recommendation.model.as_deref()?;
    if current.provider == provider && current.model == model { return None; }
    let price = |provider: &str, model: &str| event.catalog.models.iter().find(|item| item.provider == provider && item.model == model).and_then(|item| item.cost.clone()).filter(|cost| cost.input > 0.0)
        .or_else(|| fallback(provider, model).filter(|cost| cost.input > 0.0));
    let current_cost = price(&current.provider, &current.model)?;
    let suggested_cost = price(provider, model)?;
    if suggested_cost.input >= current_cost.input { return None; }
    let stay = tokens * if current_cost.cache_read > 0.0 { current_cost.cache_read } else { current_cost.input };
    let switch = tokens * suggested_cost.input;
    (switch >= stay).then(|| format!("keeping {} reuses {} cached context tokens; switching would re-read them at full price (${:.4} vs ${:.4})", current.model, tokens as u64, switch / 1e6, stay / 1e6))
}

/// The effort swap: the current model at the effort `effort.levels` gives the
/// class, or the nearest effort the host lists for that model.
pub fn resolve_effort(classification: &RoutingClassification, event: &NormalizedEvent, config: &Config) -> PolicyResolution {
    let Some(current) = &event.current_model else { return PolicyResolution::Bypass("the current model is unknown".into()); };
    let wanted = config.effort.level(&classification.routing_class);
    let Some(variant) = available_effort(&wanted, current, &event.catalog) else { return PolicyResolution::Unavailable(format!("{} lists no effort near {wanted}", current.model)); };
    let reason = format!("{} work takes {variant} effort", classification.routing_class);
    PolicyResolution::Recommendation(Recommendation { rule_id: format!("effort:{}", classification.routing_class), mode: negotiate_mode(RoutingMode::FullOverride, &event.capabilities, config), provider: Some(current.provider.clone()), model: Some(current.model.clone()), variant: Some(variant), gateway_target: None, reason, routing_class: classification.routing_class.clone(), task_type: classification.task_type.clone(), confidence: classification.confidence.clone() })
}

/// A host without a catalog takes the effort as named; one with a catalog
/// only the model's own variants, the nearest one (ties go to more effort).
fn available_effort(wanted: &str, current: &ModelState, catalog: &ModelCatalog) -> Option<String> {
    if catalog.models.is_empty() { return Some(wanted.into()); }
    let variants = &catalog.models.iter().find(|item| item.provider == current.provider && item.model == current.model)?.variants;
    if variants.iter().any(|variant| variant == wanted) { return Some(wanted.into()); }
    let rank = effort_rank(wanted)?;
    variants.iter().filter_map(|variant| Some((effort_rank(variant)?, variant))).min_by_key(|(other, _)| (other.abs_diff(rank), std::cmp::Reverse(*other))).map(|(_, variant)| variant.clone())
}

/// True when some class would set an effort other than the current one.
pub fn effort_possible(event: &NormalizedEvent, config: &Config) -> bool {
    let Some(current) = &event.current_model else { return false; };
    current.variant.as_ref().is_none_or(|variant| RoutingClass::ALL.iter().any(|class| config.effort.level(class) != *variant))
}

/// A different effort is rendered ahead of the conversation, so the API
/// re-writes the cached conversation (Anthropic and OpenAI both document
/// this). A lower effort only saves thinking, so it waits while the context
/// is large and the cache still warm; a higher effort is about quality.
pub fn effort_switch_discards_cache(recommendation: &Recommendation, event: &NormalizedEvent, config: &Config) -> Option<String> {
    let guard = config.effort.cache_guard_tokens;
    let tokens = event.context.context_tokens.filter(|tokens| guard > 0 && *tokens >= guard)?;
    if event.context.idle_ms.is_some_and(|idle| idle >= config.effort.cache_ttl_ms) { return None; }
    let current = event.current_model.as_ref()?.variant.as_deref()?;
    let suggested = recommendation.variant.as_deref()?;
    (effort_rank(suggested)? < effort_rank(current)?).then(|| format!("keeping {current} effort reuses {tokens} cached context tokens; {suggested} would re-write them"))
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
    if !usable_without_catalog(target, event) { return None; }
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
    #[test] fn catalogless_hosts_ignore_other_providers() {
        let (config, mut event, classification) = fixture();
        event.catalog.models.clear();
        event.capabilities.level = CapabilityLevel::ConsentOnly;
        event.current_model = Some(ModelState { provider: "anthropic".into(), model: "claude-opus-5-5".into(), variant: None, scope: None });
        assert!(!alternative_possible(&event, &config), "rule targets provider p, which this host cannot use");
        assert!(!matches!(resolve(&classification, &event, &config), PolicyResolution::Recommendation(_)));
        event.current_model.as_mut().unwrap().provider = "p".into();
        assert!(alternative_possible(&event, &config));
        event.current_model = None;
        event.metadata = serde_json::json!({ "hostProvider": "anthropic" });
        assert!(!alternative_possible(&event, &config), "the host provider applies before the first reply too");
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
        assert!(switch_discards_cache_savings(&rec("small"), &event, &|_, _| None).is_some(), "1.0/M uncached costs more than 0.5/M cached");
        assert!(switch_discards_cache_savings(&rec("free"), &event, &|_, _| None).is_none(), "unknown prices are never guessed");
        assert!(switch_discards_cache_savings(&rec("free"), &event, &|_, model| (model == "free").then(|| ModelCost { input: 2.0, output: 0.0, cache_read: 0.2 })).is_some(), "fallback prices fill host zeros");
        event.context.context_tokens = Some(0);
        assert!(switch_discards_cache_savings(&rec("small"), &event, &|_, _| None).is_none(), "a fresh conversation has no cache to lose");
        event.context.context_tokens = Some(100_000);
        event.current_model.as_mut().unwrap().model = "small".into();
        assert!(switch_discards_cache_savings(&rec("big"), &event, &|_, _| None).is_none(), "upgrades are never suppressed");
    }
    fn effort_event(variant: Option<&str>, variants: &[&str]) -> NormalizedEvent {
        let (_, mut event, _) = fixture();
        event.current_model = Some(ModelState { provider: "p".into(), model: "m".into(), variant: variant.map(str::to_owned), scope: None });
        // No variants stands for a host without a catalog (Claude Code).
        event.catalog.models[0].variants = variants.iter().map(|variant| variant.to_string()).collect();
        if variants.is_empty() { event.catalog.models.clear(); }
        event
    }
    #[test] fn effort_swap_keeps_the_model_and_takes_the_nearest_listed_effort() {
        let (config, _, mut classification) = fixture();
        let effort = |event: &NormalizedEvent, classification: &RoutingClassification| match resolve_effort(classification, event, &config) { PolicyResolution::Recommendation(rec) => (rec.model, rec.variant), other => panic!("{other:?}") };
        assert_eq!(effort(&effort_event(None, &[]), &classification), (Some("m".into()), Some("xhigh".into())));
        assert_eq!(effort(&effort_event(None, &["low", "high", "max"]), &classification).1.as_deref(), Some("max"), "xhigh sits between high and max: ties go to more effort");
        classification.routing_class = RoutingClass::QuestionModerate;
        assert_eq!(effort(&effort_event(None, &["low", "high"]), &classification).1.as_deref(), Some("high"), "medium is as near to low as to high");
        assert!(matches!(resolve_effort(&classification, &effort_event(None, &["fast"]), &config), PolicyResolution::Unavailable(_)));
        let mut listed = effort_event(None, &["low"]);
        listed.catalog.models[0].variants.clear();
        assert!(matches!(resolve_effort(&classification, &listed, &config), PolicyResolution::Unavailable(_)), "a listed model without variants takes no effort");
        let mut unknown = effort_event(None, &[]);
        unknown.current_model = None;
        assert!(matches!(resolve_effort(&classification, &unknown, &config), PolicyResolution::Bypass(_)));
    }
    #[test] fn effort_swap_is_skipped_when_every_class_keeps_the_current_effort() {
        let (mut config, _, _) = fixture();
        assert!(effort_possible(&effort_event(None, &[]), &config));
        for class in RoutingClass::ALL { config.effort.levels.insert(class.as_str().into(), "high".into()); }
        assert!(!effort_possible(&effort_event(Some("high"), &[]), &config));
        assert!(effort_possible(&effort_event(Some("low"), &[]), &config));
    }
    #[test] fn effort_cache_guard_holds_lower_efforts_while_the_cache_is_warm() {
        let (config, _, _) = fixture();
        let mut event = effort_event(Some("high"), &[]);
        let rec = |variant: &str| Recommendation { rule_id: "effort".into(), mode: RoutingMode::ConsentOnly, provider: Some("p".into()), model: Some("m".into()), variant: Some(variant.into()), gateway_target: None, reason: String::new(), routing_class: RoutingClass::CodeSimple, task_type: "t".into(), confidence: ClassificationConfidence { routing_class: 1.0, task_type: 1.0, context: 1.0 } };
        event.context.context_tokens = Some(80_000);
        assert!(effort_switch_discards_cache(&rec("low"), &event, &config).is_some());
        assert!(effort_switch_discards_cache(&rec("max"), &event, &config).is_none(), "more effort always applies");
        event.context.idle_ms = Some(config.effort.cache_ttl_ms);
        assert!(effort_switch_discards_cache(&rec("low"), &event, &config).is_none(), "an expired cache has nothing to lose");
        event.context.idle_ms = None;
        event.context.context_tokens = Some(10_000);
        assert!(effort_switch_discards_cache(&rec("low"), &event, &config).is_none(), "a short conversation is cheap to re-write");
    }
    #[test] fn reports_missing_catalog_target() { let (config, mut event, classification) = fixture(); event.catalog.models[0].model = "other".into(); assert!(matches!(resolve(&classification, &event, &config), PolicyResolution::Unavailable(_))); }
}
