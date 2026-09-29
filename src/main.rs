use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{cmp::Ordering, env, io::{self, IsTerminal, Read, Write}, path::{Path, PathBuf}, process::Command, sync::mpsc, thread, time::Duration};
use system1router_core::{adapters::{self, Host, OpenCodeScope}, classification::build_provider, config::Config, gateway::GatewayServer, protocol::{CapabilityLevel, DecisionOutcome, Diagnostic, NormalizedDecision, PROTOCOL_VERSION, EXIT_INTERNAL_ERROR, EXIT_MALFORMED_EVENT, EXIT_STARTUP_FAILURE, EXIT_TIMEOUT, EXIT_UNSUPPORTED_VERSION}, runtime::{process_event_safe, record_decision}};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--version") { println!("system1router {} protocol {}", env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION); return; }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        match args.first().map(String::as_str) {
            Some("install") => print_install_help(),
            Some("update") => print_update_help(),
            _ => print_help(),
        }
        return;
    }
    match args.first().map(String::as_str) {
        Some("install") => { command_opencode_install(&args); return; }
        Some("update") => { command_update(&args); return; }
        Some("__refresh-opencode-plugins") => {
            match adapters::refresh_opencode_installations() {
                Ok(results) => print_json(json!({ "status": "refreshed", "installs": results })),
                Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); }
            }
            return;
        }
        Some("init") => { command_install(&args, true); return; }
        Some("uninstall") => { command_install(&args, false); return; }
        _ => {}
    }
    let config_path = argument(&args, "--config").map(PathBuf::from);
    let config = match Config::load(config_path.as_deref()) { Ok(config) => config, Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); } };
    match args.first().map(String::as_str) {
        Some("doctor") => print_json(adapters::doctor(&config)),
        Some("gateway") => command_gateway(&args, config),
        Some("benchmark") => command_benchmark(&config),
        Some("hook") => command_hook(&args, config),
        _ => command_event(config),
    }
}

fn print_help() {
    println!("system1router {}", env!("CARGO_PKG_VERSION"));
    println!("\nUsage: system1router <command> [options]\n");
    println!("Commands:");
    println!("  install [--host H|all] [--scope project|global]       Install for detected assistants (OpenCode, Claude Code)");
    println!("  update [--check]                                      Check for and install updates");
    println!("  init --host <host>                                    Install a host hook or OpenCode plugin");
    println!("  uninstall --host <host> [--scope project|global]      Remove an installation");
    println!("  doctor                                                Show host and configuration diagnostics");
    println!("  hook --host <host>                                    Process one host hook event");
    println!("  gateway                                               Start the optional routing gateway");
    println!("  benchmark                                             Run a local routing benchmark");
    println!("  <no command>                                          Read a normalized event from stdin");
    println!("\nOptions: --config PATH, --version, --help");
}

fn print_install_help() {
    println!("Usage: system1router install [--host opencode|claude-code|all] [--scope project|global] [--project-dir PATH] [--executable PATH]");
    println!("\nDetects the installed coding assistants and installs System1Router for them:");
    println!("  OpenCode     server + TUI plugins (per-request model switch with a confirmation dialog)");
    println!("  Claude Code  UserPromptSubmit hook in settings.json (project: .claude/settings.local.json);");
    println!("               advisory: shows the suggested model and the /model command");
    println!("\nWithout --host, a menu lists detected assistants; without --scope, a menu asks for project or global.");
}

fn print_update_help() {
    println!("Usage: system1router update [--check]");
    println!("\nChecks the published crates.io version and installs it when newer. --check only reports availability. In the System1Router source checkout, an unpublished version can be installed from the current checkout.");
}

fn print_result(result: Result<Value, String>) {
    match result {
        Ok(value) => print_json(value),
        Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); }
    }
}

fn parse_opencode_scope(value: &str) -> Option<OpenCodeScope> {
    match value { "project" | "local" => Some(OpenCodeScope::Project), "global" | "user" => Some(OpenCodeScope::Global), _ => None }
}

fn interactive() -> bool { io::stdin().is_terminal() && io::stdout().is_terminal() }

fn prompt_line(prompt: &str) -> Result<String, String> {
    print!("{prompt}");
    io::stdout().flush().map_err(|error| format!("write prompt: {error}"))?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).map_err(|error| format!("read answer: {error}"))?;
    Ok(answer.trim().to_string())
}

/// Hosts to install for: `--host`, else an interactive pick among detected
/// assistants, else every detected assistant.
fn choose_hosts(args: &[String]) -> Result<Vec<Host>, String> {
    let detected = adapters::detect_hosts();
    match argument(args, "--host") {
        Some("all") => {
            if detected.is_empty() { return Err("no supported coding assistant was detected; pass --host opencode or --host claude-code".into()); }
            return Ok(detected.into_iter().map(|(host, _)| host).collect());
        }
        Some(name) => return Host::parse(name).map(|host| vec![host]).ok_or_else(|| format!("unsupported host: {name}")),
        None => {}
    }
    if detected.is_empty() { return Err("no supported coding assistant was detected; pass --host opencode or --host claude-code".into()); }
    if !interactive() || detected.len() == 1 { return Ok(detected.into_iter().map(|(host, _)| host).collect()); }
    println!("Detected coding assistants:");
    for (index, (host, evidence)) in detected.iter().enumerate() { println!("  {}) {:<12} {evidence}", index + 1, host_label(*host)); }
    let answer = prompt_line("Install for which? [Enter = all, e.g. 1,2, q = cancel]: ")?;
    if answer.eq_ignore_ascii_case("q") { return Ok(Vec::new()); }
    if answer.is_empty() || answer.eq_ignore_ascii_case("all") { return Ok(detected.into_iter().map(|(host, _)| host).collect()); }
    answer.split([',', ' ']).filter(|item| !item.is_empty()).map(|item| {
        item.parse::<usize>().ok().and_then(|index| detected.get(index.wrapping_sub(1))).map(|(host, _)| *host).ok_or_else(|| format!("invalid choice: {item}"))
    }).collect()
}

fn host_label(host: Host) -> &'static str {
    match host { Host::OpenCode => "OpenCode", Host::ClaudeCode => "Claude Code", Host::Codex => "Codex", Host::CopilotCli => "Copilot CLI", Host::CopilotVsCode => "Copilot VS Code", Host::Generic => "Generic" }
}

fn choose_opencode_scope() -> Result<Option<OpenCodeScope>, String> {
    if !interactive() {
        return Err("choose an installation scope with --scope project or --scope global".into());
    }
    println!("Install System1Router for:");
    println!("  1) This project only");
    println!("  2) All projects for this user (global)");
    print!("Choose 1 or 2 (q to cancel): ");
    io::stdout().flush().map_err(|error| format!("write install prompt: {error}"))?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).map_err(|error| format!("read install choice: {error}"))?;
    match answer.trim() {
        "1" | "project" | "local" => Ok(Some(OpenCodeScope::Project)),
        "2" | "global" | "user" => Ok(Some(OpenCodeScope::Global)),
        "q" | "Q" | "" => Ok(None),
        _ => Err("invalid install choice; enter 1, 2, or q".into()),
    }
}

fn command_opencode_install(args: &[String]) {
    let hosts = match choose_hosts(args) {
        Ok(hosts) => hosts,
        Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); }
    };
    if hosts.is_empty() { println!("Installation cancelled."); return; }
    if hosts.iter().any(|host| !matches!(host, Host::OpenCode | Host::ClaudeCode)) {
        command_install(args, true);
        return;
    }
    let scope = match argument(args, "--scope") {
        Some(value) => match parse_opencode_scope(value) {
            Some(scope) => Some(scope),
            None => { eprintln!("system1router: --scope must be project or global"); std::process::exit(EXIT_STARTUP_FAILURE); }
        },
        None => match choose_opencode_scope() {
            Ok(scope) => scope,
            Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); }
        },
    };
    let Some(scope) = scope else { println!("Installation cancelled."); return; };
    let directory = argument(args, "--project-dir").map(PathBuf::from);
    let current = env::current_dir().ok();
    let project_dir = directory.as_deref().or(current.as_deref());
    let executable = argument(args, "--executable");
    let mut results = Vec::new();
    let mut failed = false;
    for host in hosts {
        let result = match host {
            Host::ClaudeCode => adapters::install_claude_code(scope, project_dir, executable),
            _ => adapters::install_opencode(scope, project_dir),
        };
        match result {
            Ok(value) => results.push(value),
            Err(error) => { failed = true; results.push(json!({ "host": host.as_str(), "status": "failed", "error": error })); }
        }
    }
    report_installs(&results);
    if failed { std::process::exit(EXIT_STARTUP_FAILURE); }
}

fn report_installs(results: &[Value]) {
    if !io::stdout().is_terminal() {
        if let [single] = results { print_json(single); } else { print_json(json!({ "installs": results })); }
        return;
    }
    for result in results {
        let host = result["host"].as_str().unwrap_or("host");
        let status = result["status"].as_str().unwrap_or("done");
        let target = result.get("path").and_then(Value::as_str).unwrap_or_default();
        match result.get("error").and_then(Value::as_str) {
            Some(error) => println!("✗ {host}: {error}"),
            None => println!("✓ {host}: {status} {target}"),
        }
        if let Some(warning) = result.get("warning").and_then(Value::as_str) { println!("  ! {warning}"); }
    }
    println!("\nNext steps:");
    if results.iter().any(|result| result["host"] == "opencode") { println!("  • OpenCode: restart it; use /s1router_config to toggle routing."); }
    if results.iter().any(|result| result["host"] == "claude-code") { println!("  • Claude Code: add rules with \"host\": \"claude-code\" (targets such as opus, sonnet, haiku); the suggestion appears above each reply."); }
    println!("  • Run `system1router doctor` to check the provider and rules.");
}

fn command_install(args: &[String], install: bool) {
    let Some(host_name) = argument(args, "--host") else {
        eprintln!("system1router: --host is required for init/uninstall; use `system1router install` for OpenCode");
        std::process::exit(EXIT_STARTUP_FAILURE);
    };
    let Some(host) = Host::parse(host_name) else { eprintln!("unsupported host: {host_name}"); std::process::exit(EXIT_STARTUP_FAILURE); };
    let path = argument(args, "--host-config").map(PathBuf::from);
    let scope = match argument(args, "--scope") {
        Some(value) => match parse_opencode_scope(value) {
            Some(scope) => Some(scope),
            None => { eprintln!("system1router: --scope must be project or global"); std::process::exit(EXIT_STARTUP_FAILURE); }
        },
        None => None,
    };
    let project_dir = argument(args, "--project-dir").map(PathBuf::from);
    let current = env::current_dir().ok();
    let project_dir = project_dir.as_deref().or(current.as_deref());
    let result = if host == Host::OpenCode {
        let scope = scope.unwrap_or(OpenCodeScope::Global);
        if install { adapters::install_opencode(scope, project_dir) } else { adapters::uninstall_opencode(scope, project_dir) }
    } else if host == Host::ClaudeCode && path.is_none() {
        let scope = scope.unwrap_or(OpenCodeScope::Global);
        if install { adapters::install_claude_code(scope, project_dir, argument(args, "--executable")) } else { adapters::uninstall_claude_code(scope, project_dir) }
    } else {
        if scope.is_some() || argument(args, "--project-dir").is_some() {
            eprintln!("system1router: --scope and --project-dir apply only to OpenCode and Claude Code");
            std::process::exit(EXIT_STARTUP_FAILURE);
        }
        if install { adapters::install(host, path.as_deref(), argument(args, "--executable")) } else { adapters::uninstall(host, path.as_deref()) }
    };
    print_result(result);
}

#[derive(Deserialize)]
struct CratesIoResponse { #[serde(rename = "crate")] crate_info: CratesIoCrate }

#[derive(Deserialize)]
struct CratesIoCrate { max_version: String }

fn latest_crates_io_version() -> Result<Option<String>, String> {
    let endpoint = env::var("SYSTEM1ROUTER_CRATE_API").unwrap_or_else(|_| "https://crates.io/api/v1/crates/system1router".into());
    match ureq::get(&endpoint).set("User-Agent", "system1router-updater").call() {
        Ok(response) => response.into_json::<CratesIoResponse>().map(|value| Some(value.crate_info.max_version)).map_err(|error| format!("parse crates.io version response: {error}")),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(error) => Err(format!("check crates.io for updates: {error}")),
    }
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    fn parts(version: &str) -> Option<(Vec<u64>, bool)> {
        let core = version.split(['-', '+']).next()?;
        let parts = core.split('.').map(str::parse::<u64>).collect::<Result<Vec<_>, _>>().ok()?;
        Some((parts, version.contains('-')))
    }
    let Some((candidate_parts, candidate_pre)) = parts(candidate) else { return false; };
    let Some((current_parts, current_pre)) = parts(current) else { return false; };
    for index in 0..candidate_parts.len().max(current_parts.len()) {
        match candidate_parts.get(index).copied().unwrap_or(0).cmp(&current_parts.get(index).copied().unwrap_or(0)) {
            Ordering::Greater => return true,
            Ordering::Less => return false,
            Ordering::Equal => {}
        }
    }
    current_pre && !candidate_pre
}

fn cargo_install_root() -> PathBuf {
    env::var_os("CARGO_INSTALL_ROOT").or_else(|| env::var_os("CARGO_HOME")).map(PathBuf::from)
        .or_else(|| env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(|home| PathBuf::from(home).join(".cargo")))
        .unwrap_or_else(|| PathBuf::from(".cargo"))
}

fn run_cargo_install(spec: &[&str]) -> Result<(), String> {
    let root = cargo_install_root();
    let status = Command::new("cargo").args(spec).arg("--force").arg("--locked").arg("--root").arg(&root)
        .status().map_err(|error| format!("start cargo install: {error}"))?;
    if !status.success() { return Err("cargo install failed".into()); }
    let executable = root.join("bin").join(if cfg!(windows) { "system1router.exe" } else { "system1router" });
    if executable.exists() {
        let status = Command::new(&executable).arg("__refresh-opencode-plugins").status()
            .map_err(|error| format!("refresh installed OpenCode plugins: {error}"))?;
        if !status.success() { return Err("updated executable could not refresh installed OpenCode plugins".into()); }
    }
    Ok(())
}

fn current_checkout() -> bool {
    let Ok(text) = std::fs::read_to_string("Cargo.toml") else { return false; };
    let Ok(manifest) = text.parse::<toml::Value>() else { return false; };
    manifest.get("package").and_then(|package| package.get("name")).and_then(toml::Value::as_str) == Some("system1router")
        && Path::new("src/main.rs").is_file()
}

fn command_update(args: &[String]) {
    let check_only = args.iter().any(|arg| arg == "--check");
    let current = env!("CARGO_PKG_VERSION");
    println!("Current System1Router version: {current}");
    match latest_crates_io_version() {
        Ok(Some(latest)) if version_is_newer(&latest, current) => {
            if check_only { println!("Update available: {latest}"); return; }
            println!("Installing System1Router {latest}…");
            if let Err(error) = run_cargo_install(&["install", "system1router", "--version", &latest]) {
                eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE);
            }
            println!("Updated to {latest}.");
        }
        Ok(Some(latest)) => {
            println!("Already up to date ({latest}).");
            if !check_only { if let Err(error) = adapters::refresh_opencode_installations() { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); } }
        }
        Ok(None) if !check_only && current_checkout() => {
            println!("No crates.io release is published; updating from the current source checkout.");
            if let Err(error) = run_cargo_install(&["install", "--path", "."]) { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); }
            println!("Updated from the current checkout.");
        }
        Ok(None) => {
            println!("No crates.io release is published for System1Router yet.");
            if !check_only { if let Err(error) = adapters::refresh_opencode_installations() { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); } }
        }
        Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); }
    }
}

fn command_gateway(_args: &[String], config: Config) { let server = match GatewayServer::from_config(&config) { Ok(server) => server, Err(error) => { eprintln!("system1router: {error}"); std::process::exit(EXIT_STARTUP_FAILURE); } }; if let Err(error) = server.serve(config.clone(), build_provider(&config)) { eprintln!("system1router: {error}"); std::process::exit(EXIT_INTERNAL_ERROR); } }

fn command_benchmark(config: &Config) { let provider = build_provider(config); let event = adapters::normalize(Host::Generic, json!({ "prompt": "benchmark", "sessionId": "benchmark", "event": "benchmark" }), None); let start = std::time::Instant::now(); for _ in 0..100 { let _ = process_event_safe(&event, config, provider.as_ref()); } print_json(json!({ "iterations": 100, "elapsedMs": start.elapsed().as_millis(), "averageMs": start.elapsed().as_secs_f64() * 1000.0 / 100.0, "provider": provider.name() })); }

fn command_hook(args: &[String], config: Config) {
    let host_name = argument(args, "--host").unwrap_or("generic");
    let Some(host) = Host::parse(host_name) else { eprintln!("unsupported host: {host_name}"); std::process::exit(EXIT_STARTUP_FAILURE); };
    // Claude Code reports any non-zero exit as a hook error; routing failures are fail-open, so exit cleanly.
    let exit_with = |code: i32| if host == Host::ClaudeCode { 0 } else { code };
    let (payload, timed_out) = read_stdin(config.timeouts.hook_ms);
    if timed_out { let decision = bypass("timeout", "hook input timed out", "unknown".into()); print_json(adapters::translate_decision(host, &decision)); std::process::exit(exit_with(EXIT_TIMEOUT)); }
    let payload: Value = match serde_json::from_slice(&payload) {
        Ok(payload) => payload,
        Err(error) => { eprintln!("system1router: malformed hook input: {error}"); let decision = bypass("malformed_event", "hook input was not valid JSON", "unknown".into()); print_json(adapters::translate_decision(host, &decision)); std::process::exit(exit_with(EXIT_MALFORMED_EVENT)); }
    };
    let event = adapters::normalize(host, payload, env::current_dir().ok().and_then(|path| path.to_str().map(str::to_owned)));
    let provider = build_provider(&config);
    let decision = process_event_safe(&event, &config, provider.as_ref());
    record_decision(&decision, &event, &config);
    let exit = if decision.diagnostic.code == "unsupported_version" { EXIT_UNSUPPORTED_VERSION } else if decision.diagnostic.code == "internal_error" { EXIT_INTERNAL_ERROR } else { 0 };
    print_json(adapters::translate_hook_decision(host, &decision, &event));
    if exit_with(exit) != 0 { std::process::exit(exit); }
}

fn command_event(config: Config) { let (bytes, timed_out) = read_stdin(config.timeouts.hook_ms); if timed_out { print_json(bypass("timeout", "event input timed out", "unknown".into())); std::process::exit(EXIT_TIMEOUT); } let event: system1router_core::protocol::NormalizedEvent = match serde_json::from_slice(&bytes) { Ok(event) => event, Err(error) => { eprintln!("system1router: malformed event: {error}"); print_json(bypass("malformed_event", "event was not valid normalized JSON", "unknown".into())); std::process::exit(EXIT_MALFORMED_EVENT); } }; let provider = build_provider(&config); let decision = process_event_safe(&event, &config, provider.as_ref()); record_decision(&decision, &event, &config); let exit = if decision.diagnostic.code == "unsupported_version" { EXIT_UNSUPPORTED_VERSION } else if decision.diagnostic.code == "internal_error" { EXIT_INTERNAL_ERROR } else { 0 }; print_json(serde_json::to_value(decision).unwrap_or_else(|_| json!({ "outcome": "bypass" }))); if exit != 0 { std::process::exit(exit); } }

fn read_stdin(timeout_ms: u64) -> (Vec<u8>, bool) { let (sender, receiver) = mpsc::channel(); thread::spawn(move || { let mut bytes = Vec::new(); let result = io::stdin().read_to_end(&mut bytes); let _ = sender.send(result.map(|_| bytes)); }); match receiver.recv_timeout(Duration::from_millis(timeout_ms)) { Ok(Ok(bytes)) => (bytes, false), Ok(Err(_)) => (Vec::new(), false), Err(mpsc::RecvTimeoutError::Timeout) => (Vec::new(), true), Err(mpsc::RecvTimeoutError::Disconnected) => (Vec::new(), false) } }

fn argument<'a>(args: &'a [String], name: &str) -> Option<&'a str> { args.windows(2).find(|pair| pair[0] == name).map(|pair| pair[1].as_str()) }
fn bypass(code: &str, message: &str, correlation_id: String) -> NormalizedDecision { NormalizedDecision { protocol_version: PROTOCOL_VERSION.into(), correlation_id, outcome: DecisionOutcome::Bypass, recommendation: None, consent: None, capability: CapabilityLevel::ObserveOnly, diagnostic: Diagnostic::warning(code, message), telemetry: None } }
fn print_json<T: Serialize>(value: T) { let mut stdout = io::stdout().lock(); let _ = serde_json::to_writer(&mut stdout, &value); let _ = stdout.write_all(b"\n"); }


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_scope_accepts_project_and_global_aliases() {
        assert_eq!(parse_opencode_scope("project"), Some(OpenCodeScope::Project));
        assert_eq!(parse_opencode_scope("local"), Some(OpenCodeScope::Project));
        assert_eq!(parse_opencode_scope("global"), Some(OpenCodeScope::Global));
        assert_eq!(parse_opencode_scope("user"), Some(OpenCodeScope::Global));
        assert_eq!(parse_opencode_scope("unknown"), None);
    }

    #[test]
    fn updater_compares_release_versions() {
        assert!(version_is_newer("0.2.0", "0.1.9"));
        assert!(!version_is_newer("0.1.0", "0.1.0"));
        assert!(version_is_newer("0.1.0", "0.1.0-alpha.1"));
        assert!(!version_is_newer("not-a-version", "0.1.0"));
    }
}
