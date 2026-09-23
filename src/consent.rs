use crate::protocol::{ConsentOutcome, Recommendation};

pub const USE_SUGGESTED_MODEL: &str = "use suggested model";
pub const KEEP_CURRENT_MODEL: &str = "keep current model";

pub fn choices() -> [(ConsentOutcome, &'static str); 2] {
    [(ConsentOutcome::UseSuggestedModel, USE_SUGGESTED_MODEL), (ConsentOutcome::KeepCurrentModel, KEEP_CURRENT_MODEL)]
}

pub fn prompt(recommendation: &Recommendation, current: Option<&str>) -> String {
    format!("Suggested {}{}; current {}. Choose '{}' or '{}'. Reason: {}", recommendation.model.as_deref().unwrap_or("gateway target"), recommendation.variant.as_ref().map(|value| format!(" ({value})")).unwrap_or_default(), current.unwrap_or("host selection"), USE_SUGGESTED_MODEL, KEEP_CURRENT_MODEL, recommendation.reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ClassificationConfidence, Recommendation, RoutingClass, RoutingMode};
    #[test]
    fn exposes_only_mutually_exclusive_choices() { let choices = choices(); assert_eq!(choices.len(), 2); assert_eq!(choices[0].1, USE_SUGGESTED_MODEL); assert_eq!(choices[1].1, KEEP_CURRENT_MODEL); }
    #[test]
    fn prompt_names_current_and_suggested_targets() { let recommendation = Recommendation { rule_id: "r".into(), mode: RoutingMode::Advisory, provider: Some("p".into()), model: Some("m".into()), variant: None, gateway_target: None, reason: "reason".into(), routing_class: RoutingClass::CodeSimple, task_type: "code-generation".into(), confidence: ClassificationConfidence { routing_class: 0.9, task_type: 0.9, context: 0.9 } }; assert!(prompt(&recommendation, Some("old")).contains("old")); }
}
