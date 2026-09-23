use crate::{config::RedactionConfig, protocol::{BoundedContext, NormalizedEvent, RequestPayload}};
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub struct ProviderInput {
    pub query: String,
    pub context: BoundedContext,
    pub correlation_id: String,
}

pub fn redact_text(input: &str, config: &RedactionConfig) -> String {
    let mut value = input.to_string();
    let mut patterns = vec!["sk-".to_string(), "Bearer ".to_string(), "token=".to_string(), "api_key=".to_string()];
    patterns.extend(config.patterns.iter().cloned());
    for pattern in patterns {
        let mut start = 0;
        while let Some(relative) = value[start..].find(&pattern) {
            let begin = start + relative;
            let end = value[begin..].find(char::is_whitespace).map(|offset| begin + offset).unwrap_or(value.len());
            let replacement = format!("{}[REDACTED]", pattern.trim_end_matches('='));
            value.replace_range(begin..end, &replacement);
            start = begin + replacement.len();
            if start >= value.len() { break; }
        }
    }
    value
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max { return value.to_string(); }
    value.chars().take(max).collect::<String>() + "...[truncated]"
}

fn redact_value(value: &Value, config: &RedactionConfig) -> Value {
    match value {
        Value::Object(object) => {
            let mut output = Map::new();
            for (key, item) in object {
                let sensitive = config.redact_tool_arguments && ["authorization", "token", "secret", "password", "api_key", "tool_arguments"].iter().any(|part| key.to_lowercase().contains(part));
                output.insert(key.clone(), if sensitive { Value::String("[REDACTED]".into()) } else { redact_value(item, config) });
            }
            Value::Object(output)
        }
        Value::Array(items) => Value::Array(items.iter().map(|item| redact_value(item, config)).collect()),
        Value::String(text) => Value::String(redact_text(text, config)),
        _ => value.clone(),
    }
}

pub fn project_event(event: &NormalizedEvent, config: &RedactionConfig) -> ProviderInput {
    let context = event.context.clone();
    let context_json = serde_json::to_value(context).unwrap_or(Value::Null);
    let context_json = redact_value(&context_json, config);
    let context: BoundedContext = serde_json::from_value(context_json).unwrap_or_default();
    ProviderInput {
        query: truncate(&redact_text(&event.request.query, config), config.max_query_chars),
        context,
        correlation_id: crate::protocol::correlation_id(event),
    }
}

pub fn safe_request_payload(request: &RequestPayload, config: &RedactionConfig) -> Value {
    serde_json::json!({
        "query": truncate(&redact_text(&request.query, config), config.max_query_chars),
        "attachments": if config.redact_paths { Value::Array(Vec::new()) } else { redact_value(&Value::Array(request.attachments.clone()), config) },
        "tools": request.tools.iter().map(|tool| redact_value(tool, config)).collect::<Vec<_>>(),
        "toolArguments": if config.redact_tool_arguments { Value::String("[REDACTED]".into()) } else { redact_value(&request.tool_arguments, config) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_common_secrets() {
        let config = RedactionConfig::default();
        let result = redact_text("use sk-abc123 and token=secret", &config);
        assert!(!result.contains("abc123"));
        assert!(!result.contains("secret"));
    }
}
