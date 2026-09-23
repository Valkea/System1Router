use crate::{classification::DecisionProvider, config::Config, protocol::{CapabilityHints, CapabilityLevel, ModelCatalog, NormalizedEvent, RequestPayload}, runtime::process_event};
use serde_json::Value;
use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, sync::{Arc, Mutex}, time::{Duration, Instant}};

#[derive(Debug, Clone)]
pub struct GatewayServer { pub bind: String }

impl GatewayServer {
    pub fn from_config(config: &Config) -> Result<Self, String> { if !config.gateway.enabled { return Err("gateway is disabled; enable it explicitly in configuration".into()); } if config.gateway.upstream_allowlist.is_empty() { return Err("gateway requires at least one allowlisted upstream".into()); } Ok(Self { bind: config.gateway.bind.clone() }) }

    pub fn serve(&self, config: Config, provider: Box<dyn DecisionProvider>) -> Result<(), String> {
        let listener = TcpListener::bind(&self.bind).map_err(|e| format!("bind gateway: {e}"))?;
        let provider: Arc<dyn DecisionProvider> = provider.into();
        let limiter = config.gateway.rate_limit_per_minute.map(|limit| Arc::new(Mutex::new((Instant::now(), 0_u32, limit))));
        eprintln!("system1router gateway listening on {}", self.bind);
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let config = config.clone();
                    let provider = Arc::clone(&provider);
                    let limiter = limiter.clone();
                    std::thread::spawn(move || if let Err(error) = handle_connection(stream, &config, provider.as_ref(), limiter) { eprintln!("gateway request failed: {error}"); });
                }
                Err(error) => eprintln!("gateway accept failed: {error}"),
            }
        }
        Ok(())
    }
}

fn handle_connection(mut stream: TcpStream, config: &Config, provider: &dyn DecisionProvider, limiter: Option<Arc<Mutex<(Instant, u32, u32)>>>) -> Result<(), String> {
    stream.set_read_timeout(Some(Duration::from_millis(config.timeouts.hook_ms))).ok();
    let request = read_http_request(&mut stream)?;
    if request.method != "POST" || !request.path.starts_with("/v1/") { return write_response(&mut stream, 404, "application/json", br#"{"error":"not found"}"#); }
    if !authorized(&request.headers, config) { return write_response(&mut stream, 401, "application/json", br#"{"error":"unauthorized"}"#); }
    if !rate_allowed(limiter) { return write_response(&mut stream, 429, "application/json", br#"{"error":"rate limit exceeded"}"#); }
    let mut body: Value = serde_json::from_slice(&request.body).map_err(|_| "gateway request body must be JSON".to_string())?;
    let query = extract_query(&body);
    let event = NormalizedEvent { protocol_version: crate::protocol::PROTOCOL_VERSION.into(), host: "gateway".into(), event: "gateway.request".into(), session_id: request.headers.get("x-session-id").cloned().unwrap_or_else(|| "gateway-session".into()), turn_id: None, cwd: None, request: RequestPayload { query, attachments: Vec::new(), tools: body.get("tools").and_then(Value::as_array).cloned().unwrap_or_default(), tool_arguments: Value::Null, original_payload: body.clone() }, current_model: body.get("model").and_then(Value::as_str).map(|model| crate::protocol::ModelState { provider: "gateway".into(), model: model.into(), variant: None, scope: None }), context: Default::default(), catalog: ModelCatalog::default(), capabilities: CapabilityHints { level: CapabilityLevel::Gateway, model_mutation: false, interactive_consent: false, gateway: true, protocol_version: Some(crate::protocol::PROTOCOL_VERSION.into()) }, correlation_id: request.headers.get("x-correlation-id").cloned(), metadata: Value::Null };
    let decision = process_event(&event, config, provider);
    let upstream = decision.recommendation.as_ref().and_then(|recommendation| recommendation.gateway_target.clone()).or_else(|| config.gateway.default_upstream.as_ref().and_then(|id| config.gateway.upstream_allowlist.iter().find(|target| &target.id == id).cloned()));
    let Some(upstream) = upstream else { return write_response(&mut stream, 503, "application/json", br#"{"error":"no eligible gateway upstream"}"#); };
    if let Some(object) = body.as_object_mut() { object.insert("model".into(), Value::String(upstream.model.clone())); }
    let payload = serde_json::to_vec(&body).map_err(|e| format!("serialize gateway request: {e}"))?;
    forward(&upstream.endpoint, &request.path, &payload, &mut stream)
}

fn rate_allowed(limiter: Option<Arc<Mutex<(Instant, u32, u32)>>>) -> bool {
    let Some(limiter) = limiter else { return true; };
    let Ok(mut window) = limiter.lock() else { return false; };
    if window.0.elapsed() >= Duration::from_secs(60) { window.0 = Instant::now(); window.1 = 0; }
    if window.1 >= window.2 { return false; }
    window.1 += 1;
    true
}

fn authorized(headers: &std::collections::HashMap<String, String>, config: &Config) -> bool {
    let Some(env_name) = &config.gateway.auth_token_env else { return false; };
    let Some(expected) = std::env::var(env_name).ok() else { return false; };
    headers.get("authorization").is_some_and(|value| value == &format!("Bearer {expected}"))
}

struct HttpRequest { method: String, path: String, headers: std::collections::HashMap<String, String>, body: Vec<u8> }

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut bytes = Vec::new(); let mut buffer = [0_u8; 4096]; let mut header_end = None;
    for _ in 0..128 { let count = stream.read(&mut buffer).map_err(|e| format!("read gateway request: {e}"))?; if count == 0 { break; } bytes.extend_from_slice(&buffer[..count]); if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") { header_end = Some(index + 4); break; } }
    let header_end = header_end.ok_or_else(|| "gateway request headers are incomplete".to_string())?;
    let header = String::from_utf8(bytes[..header_end].to_vec()).map_err(|_| "gateway headers are not UTF-8".to_string())?;
    let mut lines = header.split("\r\n"); let first = lines.next().ok_or_else(|| "gateway request line is missing".to_string())?; let mut parts = first.split_whitespace(); let method = parts.next().unwrap_or_default().into(); let path = parts.next().unwrap_or_default().into();
    let mut headers: std::collections::HashMap<String, String> = std::collections::HashMap::new(); for line in lines { if let Some((name, value)) = line.split_once(':') { headers.insert(name.trim().to_lowercase(), value.trim().into()); } }
    let length = headers.get("content-length").and_then(|value| value.parse::<usize>().ok()).unwrap_or(0); let mut body = bytes[header_end..].to_vec(); while body.len() < length { let count = stream.read(&mut buffer).map_err(|e| format!("read gateway body: {e}"))?; if count == 0 { break; } body.extend_from_slice(&buffer[..count]); } body.truncate(length);
    Ok(HttpRequest { method, path, headers, body })
}

fn extract_query(body: &Value) -> String { body.get("messages").and_then(Value::as_array).map(|messages| messages.iter().filter_map(|message| message.get("content").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")).or_else(|| body.get("prompt").and_then(Value::as_str).map(str::to_owned)).unwrap_or_default() }

fn forward(endpoint: &str, path: &str, body: &[u8], client: &mut TcpStream) -> Result<(), String> {
    let endpoint = endpoint.strip_prefix("http://").ok_or_else(|| "only explicitly configured http upstreams are supported by the stdlib gateway; use a local TLS proxy for https".to_string())?;
    let (host_port, base_path) = endpoint.split_once('/').map(|(host, path)| (host, format!("/{path}"))).unwrap_or((endpoint, "/".into()));
    let mut stream = TcpStream::connect(host_port).map_err(|e| format!("connect upstream: {e}"))?;
    let request_path = if path.is_empty() { base_path } else { path.to_string() };
    let request = format!("POST {request_path} HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(request.as_bytes()).map_err(|e| format!("write upstream request: {e}"))?; stream.write_all(body).map_err(|e| format!("write upstream body: {e}"))?;
    let mut buffer = [0_u8; 8192];
    loop { let count = stream.read(&mut buffer).map_err(|e| format!("read upstream response: {e}"))?; if count == 0 { break; } client.write_all(&buffer[..count]).map_err(|e| format!("write gateway response: {e}"))?; }
    Ok(())
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> Result<(), String> { let reason = match status { 200 => "OK", 401 => "Unauthorized", 404 => "Not Found", 429 => "Too Many Requests", 503 => "Service Unavailable", _ => "Error" }; let header = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()); stream.write_all(header.as_bytes()).and_then(|_| stream.write_all(body)).map_err(|e| format!("write gateway response: {e}")) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    #[test]
    fn gateway_requires_explicit_enablement_and_allowlist() {
        let config = Config::default();
        assert!(GatewayServer::from_config(&config).is_err());
        let mut config = config;
        config.gateway.enabled = true;
        assert!(GatewayServer::from_config(&config).is_err());
    }
    #[test]
    fn extracts_openai_message_text_without_logging_payload() {
        let value = serde_json::json!({ "messages": [{ "role": "user", "content": "secret prompt" }] });
        assert_eq!(extract_query(&value), "secret prompt");
    }
    #[test]
    fn auth_is_denied_without_configured_secret() {
        let config = Config::default();
        assert!(!authorized(&std::collections::HashMap::new(), &config));
    }
}
