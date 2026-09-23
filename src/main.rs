use serde::{Serialize};
use serde_json::{json, Value};
use std::{env, io::{self, Read, Write}, path::PathBuf, sync::mpsc, thread, time::Duration};
use system1router_core::{adapters::{self, Host}, classification::build_provider, config::Config, gateway::GatewayServer, protocol::{CapabilityLevel, DecisionOutcome, Diagnostic, NormalizedDecision, PROTOCOL_VERSION, EXIT_INTERNAL_ERROR, EXIT_MALFORMED_EVENT, EXIT_STARTUP_FAILURE, EXIT_TIMEOUT, EXIT_UNSUPPORTED_VERSION}, runtime::{process_event_safe, record_decision}};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--version") { println!("system1router {} protocol {}", env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION); return; }
    let config_path = argument(&args, "--config").map(PathBuf::from);
    let config = match Config::load(config_path.as_deref()) { Ok(config) => config, Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); } };
    match args.first().map(String::as_str) {
        Some("init") => command_install(&args, true),
        Some("uninstall") => command_install(&args, false),
        Some("doctor") => print_json(adapters::doctor(&config)),
        Some("gateway") => command_gateway(&args, config),
        Some("benchmark") => command_benchmark(&config),
        Some("hook") => command_hook(&args, config),
        _ => command_event(config),
    }
}

fn command_install(args: &[String], install: bool) { let Some(host_name) = argument(args, "--host") else { eprintln!("--host is required"); std::process::exit(EXIT_STARTUP_FAILURE); }; let Some(host) = Host::parse(host_name) else { eprintln!("unsupported host: {host_name}"); std::process::exit(EXIT_STARTUP_FAILURE); }; let path = argument(args, "--host-config").map(PathBuf::from); let result = if install { adapters::install(host, path.as_deref(), argument(args, "--executable")) } else { adapters::uninstall(host, path.as_deref()) }; match result { Ok(value) => print_json(value), Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); } } }

fn command_gateway(_args: &[String], config: Config) { let server = match GatewayServer::from_config(&config) { Ok(server) => server, Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); } }; if let Err(error) = server.serve(config.clone(), build_provider(&config)) { eprintln!("system1router: {error}"); std::process::exit(EXIT_INTERNAL_ERROR); } }

fn command_benchmark(config: &Config) { let provider = build_provider(config); let event = adapters::normalize(Host::Generic, json!({ "prompt": "benchmark", "sessionId": "benchmark", "event": "benchmark" }), None); let start = std::time::Instant::now(); for _ in 0..100 { let _ = process_event_safe(&event, config, provider.as_ref()); } print_json(json!({ "iterations": 100, "elapsedMs": start.elapsed().as_millis(), "averageMs": start.elapsed().as_secs_f64() * 1000.0 / 100.0, "provider": provider.name() })); }

fn command_hook(args: &[String], config: Config) { let host_name = argument(args, "--host").unwrap_or("generic"); let Some(host) = Host::parse(host_name) else { eprintln!("unsupported host: {host_name}"); std::process::exit(EXIT_STARTUP_FAILURE); }; let (payload, timed_out) = read_stdin(config.timeouts.hook_ms); if timed_out { let decision = bypass("timeout", "hook input timed out", "unknown".into()); print_json(adapters::translate_decision(host, &decision)); std::process::exit(EXIT_TIMEOUT); } let payload: Value = match serde_json::from_slice(&payload) { Ok(payload) => payload, Err(error) => { eprintln!("system1router: malformed hook input: {error}"); let decision = bypass("malformed_event", "hook input was not valid JSON", "unknown".into()); print_json(adapters::translate_decision(host, &decision)); std::process::exit(EXIT_MALFORMED_EVENT); } }; let event = adapters::normalize(host, payload, env::current_dir().ok().and_then(|path| path.to_str().map(str::to_owned))); let provider = build_provider(&config); let decision = process_event_safe(&event, &config, provider.as_ref()); record_decision(&decision, &event, &config); let exit = if decision.diagnostic.code == "unsupported_version" { EXIT_UNSUPPORTED_VERSION } else if decision.diagnostic.code == "internal_error" { EXIT_INTERNAL_ERROR } else { 0 }; print_json(adapters::translate_decision(host, &decision)); if exit != 0 { std::process::exit(exit); } }

fn command_event(config: Config) { let (bytes, timed_out) = read_stdin(config.timeouts.hook_ms); if timed_out { print_json(bypass("timeout", "event input timed out", "unknown".into())); std::process::exit(EXIT_TIMEOUT); } let event: system1router_core::protocol::NormalizedEvent = match serde_json::from_slice(&bytes) { Ok(event) => event, Err(error) => { eprintln!("system1router: malformed event: {error}"); print_json(bypass("malformed_event", "event was not valid normalized JSON", "unknown".into())); std::process::exit(EXIT_MALFORMED_EVENT); } }; let provider = build_provider(&config); let decision = process_event_safe(&event, &config, provider.as_ref()); record_decision(&decision, &event, &config); let exit = if decision.diagnostic.code == "unsupported_version" { EXIT_UNSUPPORTED_VERSION } else if decision.diagnostic.code == "internal_error" { EXIT_INTERNAL_ERROR } else { 0 }; print_json(serde_json::to_value(decision).unwrap_or_else(|_| json!({ "outcome": "bypass" }))); if exit != 0 { std::process::exit(exit); } }

fn read_stdin(timeout_ms: u64) -> (Vec<u8>, bool) { let (sender, receiver) = mpsc::channel(); thread::spawn(move || { let mut bytes = Vec::new(); let result = io::stdin().read_to_end(&mut bytes); let _ = sender.send(result.map(|_| bytes)); }); match receiver.recv_timeout(Duration::from_millis(timeout_ms)) { Ok(Ok(bytes)) => (bytes, false), Ok(Err(_)) => (Vec::new(), false), Err(mpsc::RecvTimeoutError::Timeout) => (Vec::new(), true), Err(mpsc::RecvTimeoutError::Disconnected) => (Vec::new(), false) } }

fn argument<'a>(args: &'a [String], name: &str) -> Option<&'a str> { args.windows(2).find(|pair| pair[0] == name).map(|pair| pair[1].as_str()) }
fn bypass(code: &str, message: &str, correlation_id: String) -> NormalizedDecision { NormalizedDecision { protocol_version: PROTOCOL_VERSION.into(), correlation_id, outcome: DecisionOutcome::Bypass, recommendation: None, consent: None, capability: CapabilityLevel::ObserveOnly, diagnostic: Diagnostic::warning(code, message), telemetry: None } }
fn print_json<T: Serialize>(value: T) { let mut stdout = io::stdout().lock(); let _ = serde_json::to_writer(&mut stdout, &value); let _ = stdout.write_all(b"\n"); }
