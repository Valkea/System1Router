//! Host-specific installation is separate from request translation. New
//! assistants can add an installer without changing the routing state machine.
use crate::{adapters::Host, config::Config, protocol::PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::{env, fs, path::{Path, PathBuf}};

pub const INSTALL_VERSION: &str = env!("CARGO_PKG_VERSION");
const OWNER_KEY: &str = "_system1router";
const OPENCODE_MANIFEST: &str = ".system1router-install.json";
const OPENCODE_MARKER: &str = "// Installed by system1router CLI; managed file.\n";
const OPENCODE_SERVER_FILE: &str = "plugins/system1router-server.ts";
const OPENCODE_SHARED_FILE: &str = "system1router-shared.ts";
const OPENCODE_TUI_FILE: &str = "system1router-tui.ts";
const OPENCODE_TUI_SPEC: &str = "./system1router-tui.ts";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenCodeScope { Project, Global }

impl OpenCodeScope {
    pub fn as_str(self) -> &'static str { match self { Self::Project => "project", Self::Global => "global" } }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OpenCodeInstallManifest {
    scope: String,
    version: String,
    files: Vec<String>,
    tui_config: String,
    added_tui_entry: bool,
}

fn opencode_config_dir() -> PathBuf {
    if let Some(path) = env::var_os("OPENCODE_CONFIG_DIR") { return PathBuf::from(path); }
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") { return PathBuf::from(path).join("opencode"); }
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".config/opencode")
}

fn opencode_root(scope: OpenCodeScope, project_dir: Option<&Path>) -> Result<PathBuf, String> {
    match scope {
        OpenCodeScope::Global => Ok(opencode_config_dir()),
        OpenCodeScope::Project => {
            let start = match project_dir {
                Some(path) => path.to_path_buf(),
                None => env::current_dir().map_err(|error| format!("read current directory: {error}"))?,
            };
            Ok(project_root(&start).join(".opencode"))
        }
    }
}

fn project_root(start: &Path) -> PathBuf {
    for directory in start.ancestors() {
        if directory.join(".opencode").is_dir()
            || directory.join(".git").exists()
            || directory.join("opencode.json").is_file()
            || directory.join("opencode.jsonc").is_file()
        {
            return directory.to_path_buf();
        }
    }
    start.to_path_buf()
}

pub fn default_config_path(host: Host) -> PathBuf {
    if let Ok(path) = env::var("SYSTEM1ROUTER_HOST_CONFIG") { return PathBuf::from(path); }
    if host == Host::OpenCode {
        let root = opencode_config_dir();
        let json = root.join("opencode.json");
        return if !json.exists() && root.join("opencode.jsonc").exists() { root.join("opencode.jsonc") } else { json };
    }
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    match host { Host::ClaudeCode => crate::claude_code::settings_path(true, Path::new(".")), Host::Codex => PathBuf::from(home).join(".codex/config.json"), Host::CopilotCli => PathBuf::from(home).join(".config/github-copilot/hooks.json"), Host::CopilotVsCode => PathBuf::from(home).join(".config/system1router/copilot-vscode.json"), Host::OpenCode => unreachable!(), Host::Generic => PathBuf::from(home).join(".config/system1router/generic-hook.json") }
}

/// Absolute path of the running executable, so hooks keep working when the
/// host is started without Cargo's bin directory on PATH.
pub fn default_executable() -> String {
    env::current_exe().ok().and_then(|path| path.canonicalize().ok()).and_then(|path| path.to_str().map(str::to_owned)).unwrap_or_else(|| "system1router".into())
}

fn on_path(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) { vec![format!("{name}.exe"), format!("{name}.cmd"), name.into()] } else { vec![name.into()] };
    env::split_paths(&env::var_os("PATH")?).flat_map(|directory| names.iter().map(move |name| directory.join(name))).find(|path| path.is_file())
}

/// Coding assistants present on this machine, with the evidence found.
pub fn detect_hosts() -> Vec<(Host, String)> {
    let mut hosts = Vec::new();
    if let Some(path) = on_path("opencode") { hosts.push((Host::OpenCode, path.display().to_string())); }
    else if opencode_config_dir().is_dir() { hosts.push((Host::OpenCode, opencode_config_dir().display().to_string())); }
    let claude_dir = crate::claude_code::settings_path(true, Path::new(".")).parent().map(Path::to_path_buf).unwrap_or_default();
    if let Some(path) = on_path("claude") { hosts.push((Host::ClaudeCode, path.display().to_string())); }
    else if claude_dir.is_dir() { hosts.push((Host::ClaudeCode, claude_dir.display().to_string())); }
    hosts
}

fn project_directory(project_dir: Option<&Path>) -> Result<PathBuf, String> {
    let start = match project_dir { Some(path) => path.to_path_buf(), None => env::current_dir().map_err(|error| format!("read current directory: {error}"))? };
    Ok(project_root(&start))
}

pub fn install_claude_code(scope: OpenCodeScope, project_dir: Option<&Path>, executable: Option<&str>) -> Result<Value, String> {
    let path = crate::claude_code::settings_path(scope == OpenCodeScope::Global, &project_directory(project_dir)?);
    let mut result = crate::claude_code::install(&path, &executable.map(str::to_owned).unwrap_or_else(default_executable))?;
    result["scope"] = json!(scope.as_str());
    Ok(result)
}

pub fn uninstall_claude_code(scope: OpenCodeScope, project_dir: Option<&Path>) -> Result<Value, String> {
    let path = crate::claude_code::settings_path(scope == OpenCodeScope::Global, &project_directory(project_dir)?);
    let mut result = crate::claude_code::uninstall(&path)?;
    result["scope"] = json!(scope.as_str());
    Ok(result)
}

pub fn install(host: Host, config_path: Option<&Path>, executable: Option<&str>) -> Result<Value, String> {
    if host == Host::OpenCode { return install_opencode(OpenCodeScope::Global, None); }
    if host == Host::ClaudeCode {
        let path = config_path.map(PathBuf::from).unwrap_or_else(|| default_config_path(host));
        return crate::claude_code::install(&path, &executable.map(str::to_owned).unwrap_or_else(default_executable));
    }
    let path = config_path.map(PathBuf::from).unwrap_or_else(|| default_config_path(host));
    let mut root: Value = if path.exists() { serde_json::from_str(&fs::read_to_string(&path).map_err(|e| format!("read host config: {e}"))?).map_err(|e| format!("parse host config: {e}"))? } else { json!({}) };
    let command = executable.unwrap_or("system1router");
    let entry = json!({ "type": "command", "command": format!("{command} hook --host {}", host.as_str()), OWNER_KEY: { "version": INSTALL_VERSION, "protocolVersion": PROTOCOL_VERSION, "hash": installation_hash(host, command) } });
    let array = match host { Host::ClaudeCode | Host::Codex => root.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!({})).as_object_mut().unwrap().entry("UserPromptSubmit").or_insert_with(|| json!([])).as_array_mut().ok_or("host UserPromptSubmit hooks must be an array")?, _ => root.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!([])).as_array_mut().ok_or("host hooks must be an array")? };
    if !array.iter().any(|item| item.get(OWNER_KEY).is_some() || item.get("hooks").and_then(Value::as_array).is_some_and(|items| items.iter().any(|hook| hook.get(OWNER_KEY).is_some()))) { array.push(entry); }
    write_json(&path, &root)?;
    Ok(json!({ "host": host.as_str(), "status": "installed", "path": path, "version": INSTALL_VERSION, "protocolVersion": PROTOCOL_VERSION }))
}

pub fn install_opencode(scope: OpenCodeScope, project_dir: Option<&Path>) -> Result<Value, String> {
    let root = opencode_root(scope, project_dir)?;
    let project = project_dir.map(project_root);
    install_opencode_at(&root, scope, project.as_deref())
}

pub fn uninstall(host: Host, config_path: Option<&Path>) -> Result<Value, String> {
    if host == Host::OpenCode { return uninstall_opencode(OpenCodeScope::Global, None); }
    if host == Host::ClaudeCode { return crate::claude_code::uninstall(&config_path.map(PathBuf::from).unwrap_or_else(|| default_config_path(host))); }
    let path = config_path.map(PathBuf::from).unwrap_or_else(|| default_config_path(host));
    if !path.exists() { return Ok(json!({ "host": host.as_str(), "status": "not-installed", "path": path })); }
    let mut root: Value = serde_json::from_str(&fs::read_to_string(&path).map_err(|e| format!("read host config: {e}"))?).map_err(|e| format!("parse host config: {e}"))?;
    if let Some(array) = match host { Host::ClaudeCode | Host::Codex => root.get_mut("hooks").and_then(|hooks| hooks.get_mut("UserPromptSubmit")), _ => root.get_mut("hooks") }.and_then(Value::as_array_mut) { array.retain(|item| item.get(OWNER_KEY).is_none() && !item.get("hooks").and_then(Value::as_array).is_some_and(|items| items.iter().any(|hook| hook.get(OWNER_KEY).is_some()))); }
    write_json(&path, &root)?;
    Ok(json!({ "host": host.as_str(), "status": "uninstalled", "path": path }))
}

pub fn uninstall_opencode(scope: OpenCodeScope, project_dir: Option<&Path>) -> Result<Value, String> {
    let root = opencode_root(scope, project_dir)?;
    uninstall_opencode_at(&root, scope)
}

pub fn refresh_opencode_installations() -> Result<Vec<Value>, String> {
    let mut results = Vec::new();
    let global = opencode_config_dir();
    if global.join(OPENCODE_MANIFEST).exists() {
        results.push(install_opencode_at(&global, OpenCodeScope::Global, None)?);
    }
    let cwd = env::current_dir().map_err(|error| format!("read current directory: {error}"))?;
    let project = project_root(&cwd).join(".opencode");
    if project.join(OPENCODE_MANIFEST).exists() {
        results.push(install_opencode_at(&project, OpenCodeScope::Project, None)?);
    }
    Ok(results)
}

fn plugin_assets() -> Vec<(&'static str, String)> {
    let server = include_str!("../.opencode/plugins/system1router/server.ts")
        .replace("from \"./shared\"", "from \"../system1router-shared\"");
    let shared = include_str!("../.opencode/plugins/system1router/shared.ts").to_string();
    let tui = include_str!("../.opencode/plugins/system1router/tui.ts")
        .replace("from \"./shared\"", "from \"./system1router-shared\"");
    [
        (OPENCODE_SERVER_FILE, server),
        (OPENCODE_SHARED_FILE, shared),
        (OPENCODE_TUI_FILE, tui),
    ]
    .into_iter()
    .map(|(name, contents)| (name, format!("{OPENCODE_MARKER}{contents}")))
    .collect()
}

fn tui_config_text() -> Result<String, String> {
    let value = json!({
        "$schema": "https://opencode.ai/tui.json",
        "plugin": [OPENCODE_TUI_SPEC],
    });
    Ok(format!("{OPENCODE_MARKER}{}\n", serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?))
}

fn tui_plugin_is_registered(root: &Path, spec: &str) -> Option<PathBuf> {
    ["tui.json", "tui.jsonc"].into_iter().map(|name| root.join(name)).find(|path| {
        fs::read_to_string(path).ok().is_some_and(|text| text.contains(spec))
    })
}

fn choose_tui_config(root: &Path) -> Result<(PathBuf, bool), String> {
    let json = root.join("tui.json");
    let jsonc = root.join("tui.jsonc");
    match (json.exists(), jsonc.exists()) {
        (false, false) => Ok((json, true)),
        (true, false) => Ok((jsonc, true)),
        (false, true) => Ok((json, true)),
        (true, true) => {
            for path in [&json, &jsonc] {
                let text = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
                if text.contains(OPENCODE_TUI_SPEC) { return Ok((path.to_path_buf(), false)); }
            }
            Err(format!("both {} and {} already exist; add {OPENCODE_TUI_SPEC} to one of them, then rerun install", json.display(), jsonc.display()))
        }
    }
}

fn write_managed_plugin(path: &Path, contents: &str) -> Result<(), String> {
    if path.exists() {
        let existing = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
        if !existing.starts_with(OPENCODE_MARKER) {
            return Err(format!("refusing to overwrite unmanaged plugin file {}; move it aside and retry", path.display()));
        }
    }
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; }
    fs::write(path, contents).map_err(|error| format!("write {}: {error}", path.display()))
}

fn write_tui_registration(path: &Path, added: bool) -> Result<(), String> {
    if added {
        if path.exists() {
            let text = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
            if text.starts_with(OPENCODE_MARKER) && text.contains(OPENCODE_TUI_SPEC) { return Ok(()); }
            return Err(format!("refusing to overwrite unmanaged TUI config {}", path.display()));
        }
        if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; }
        return fs::write(path, tui_config_text()?).map_err(|error| format!("write {}: {error}", path.display()));
    }

    let text = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if text.contains(OPENCODE_TUI_SPEC) { return Ok(()); }
    Err(format!("TUI config {} changed since installation; add {OPENCODE_TUI_SPEC} manually", path.display()))
}

fn has_legacy_project_install(root: &Path) -> bool {
    root.join("plugins/system1router/server.ts").is_file()
        && root.join("plugins/system1router/tui.ts").is_file()
        && root.join("opencode.json").is_file()
        && root.join("tui.json").is_file()
}

fn install_opencode_at(root: &Path, scope: OpenCodeScope, project_dir: Option<&Path>) -> Result<Value, String> {
    let manifest_path = root.join(OPENCODE_MANIFEST);
    let existing_manifest = if manifest_path.exists() {
        Some(serde_json::from_str::<OpenCodeInstallManifest>(
            &fs::read_to_string(&manifest_path).map_err(|error| format!("read {}: {error}", manifest_path.display()))?,
        ).map_err(|error| format!("parse {}: {error}", manifest_path.display()))?)
    } else { None };

    if scope == OpenCodeScope::Project && existing_manifest.is_none() && has_legacy_project_install(root) {
        return Ok(json!({ "host": "opencode", "scope": scope.as_str(), "status": "already-installed", "path": root }));
    }
    if existing_manifest.as_ref().is_some_and(|manifest| manifest.scope != scope.as_str()) {
        return Err(format!("{} is already registered as a {}-scope install", root.display(), existing_manifest.unwrap().scope));
    }

    let assets = plugin_assets();
    let plugin_root = root.join("plugins");
    if existing_manifest.is_none() && tui_plugin_is_registered(root, OPENCODE_TUI_SPEC).is_some() {
        let already_managed = assets.iter().all(|(name, expected)| {
            fs::read_to_string(root.join(name)).ok().is_some_and(|contents| contents == *expected)
        });
        if already_managed {
            return Ok(json!({ "host": "opencode", "scope": scope.as_str(), "status": "already-installed", "path": root }));
        }
        return Err(format!("{OPENCODE_TUI_SPEC} is already registered but is not managed by this installer; remove that entry or use the existing plugin installation"));
    }
    for (name, contents) in &assets {
        let path = root.join(name);
        if path.exists() {
            let existing = fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
            if !existing.starts_with(OPENCODE_MARKER) {
                return Err(format!("refusing to overwrite unmanaged plugin file {}; move it aside and retry", path.display()));
            }
        }
        if contents.is_empty() { return Err(format!("embedded OpenCode plugin asset {name} is empty")); }
    }

    let (tui_path, added_tui_entry) = match &existing_manifest {
        Some(manifest) => (root.join(&manifest.tui_config), manifest.added_tui_entry),
        None => choose_tui_config(root)?,
    };
    if existing_manifest.is_some() && !tui_path.exists() {
        return Err(format!("managed TUI config {} is missing; run install again after restoring it", tui_path.display()));
    }
    if let Some(parent) = plugin_root.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; }
    fs::create_dir_all(&plugin_root).map_err(|error| format!("create {}: {error}", plugin_root.display()))?;
    for (name, contents) in assets {
        write_managed_plugin(&root.join(name), &contents)?;
    }
    write_tui_registration(&tui_path, added_tui_entry)?;

    let manifest = OpenCodeInstallManifest {
        scope: scope.as_str().into(),
        version: INSTALL_VERSION.into(),
        files: vec![
            OPENCODE_SERVER_FILE.into(),
            OPENCODE_SHARED_FILE.into(),
            OPENCODE_TUI_FILE.into(),
        ],
        tui_config: tui_path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        added_tui_entry,
    };
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?)
        .map_err(|error| format!("write {}: {error}", manifest_path.display()))?;

    let mut result = json!({
        "host": "opencode",
        "scope": scope.as_str(),
        "status": "installed",
        "path": root,
        "tuiConfig": tui_path,
        "version": INSTALL_VERSION,
    });
    if scope == OpenCodeScope::Global && project_dir.is_some_and(|directory| has_legacy_project_install(&directory.join(".opencode"))) {
        result["warning"] = json!("This project already has a local System1Router plugin; avoid loading both local and global copies here.");
    }
    Ok(result)
}

fn uninstall_opencode_at(root: &Path, scope: OpenCodeScope) -> Result<Value, String> {
    let manifest_path = root.join(OPENCODE_MANIFEST);
    if !manifest_path.exists() {
        if scope == OpenCodeScope::Project && has_legacy_project_install(root) {
            return Ok(json!({ "host": "opencode", "scope": scope.as_str(), "status": "source-managed", "path": root }));
        }
        return Ok(json!({ "host": "opencode", "scope": scope.as_str(), "status": "not-installed", "path": root }));
    }
    let manifest: OpenCodeInstallManifest = serde_json::from_str(
        &fs::read_to_string(&manifest_path).map_err(|error| format!("read {}: {error}", manifest_path.display()))?,
    ).map_err(|error| format!("parse {}: {error}", manifest_path.display()))?;
    if manifest.scope != scope.as_str() { return Err(format!("{} is registered as a {}-scope install", root.display(), manifest.scope)); }

    let mut files = Vec::new();
    for relative in &manifest.files {
        let path = root.join(relative);
        if !path.exists() { continue; }
        let contents = fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
        if !contents.starts_with(OPENCODE_MARKER) { return Err(format!("refusing to remove modified plugin file {}; restore its installer marker or remove it manually", path.display())); }
        files.push(path);
    }
    let tui_path = root.join(&manifest.tui_config);
    let tui_contents = if manifest.added_tui_entry && tui_path.exists() {
        let contents = fs::read_to_string(&tui_path).map_err(|error| format!("read {}: {error}", tui_path.display()))?;
        if !contents.starts_with(OPENCODE_MARKER) { return Err(format!("refusing to remove modified TUI config {}; remove {OPENCODE_TUI_SPEC} manually", tui_path.display())); }
        if contents != tui_config_text()? { return Err(format!("TUI config {} has user edits; remove {OPENCODE_TUI_SPEC} manually", tui_path.display())); }
        Some(contents)
    } else { None };

    for path in files { fs::remove_file(&path).map_err(|error| format!("remove {}: {error}", path.display()))?; }
    if tui_contents.is_some() { fs::remove_file(&tui_path).map_err(|error| format!("remove {}: {error}", tui_path.display()))?; }
    fs::remove_file(&manifest_path).map_err(|error| format!("remove {}: {error}", manifest_path.display()))?;
    Ok(json!({ "host": "opencode", "scope": scope.as_str(), "status": "uninstalled", "path": root }))
}

pub fn doctor(config: &Config) -> Value {
    let all = [Host::OpenCode, Host::ClaudeCode, Host::Codex, Host::CopilotCli, Host::CopilotVsCode, Host::Generic];
    // Claude Code has no catalog: only Anthropic targets can be suggested there.
    let rules_for = |host: Host| config.rules.iter().filter(|rule| rule.host.as_deref().is_none_or(|name| Host::parse(name) == Some(host))
        && (host != Host::ClaudeCode || rule.target.provider.as_deref().is_none_or(|provider| provider == "anthropic"))).count();
    let hosts = all.into_iter().map(|host| json!({ "host": host.as_str(), "apiVersion": host.api_version(), "capability": host.capability(), "configPath": default_config_path(host), "installed": host_installation_present(host), "rules": rules_for(host) })).collect::<Vec<_>>();
    let summary = config.safe_summary();
    let mut problems: Vec<String> = Vec::new();
    if !config.routing.enabled { problems.push("routing is disabled: set routing.enabled to true (or use /s1router_toggle in OpenCode)".into()); }
    let provider_ready = match config.providers.selected.as_str() { "jev" => summary["jevConfigured"] == true, "laya" => summary["layaConfigured"] == true, _ => false };
    if !provider_ready { problems.push(format!("decision provider `{}` is not usable: select jev or laya and set its endpoint/credential (see README)", config.providers.selected)); }
    if config.rules.is_empty() { problems.push("no routing rules: add one rule per routing class you want to route".into()); }
    for host in all.into_iter().filter(|host| host_installation_present(*host)) {
        if rules_for(host) == 0 { problems.push(format!("{} is installed but no rule applies to it (rules with \"host\": \"{}\" or no host)", host.as_str(), host.as_str())); }
    }
    for rule in config.rules.iter().filter(|rule| rule.host.as_deref().is_some_and(|name| Host::parse(name).is_none())) {
        problems.push(format!("rule {} names unknown host {:?}", rule.id, rule.host.as_deref().unwrap_or_default()));
    }
    let mut advice: Vec<String> = Vec::new();
    if let Some(table) = crate::prices::load() {
        let priced: Vec<_> = config.rules.iter().filter_map(|rule| {
            let class = rule.routing_class.clone()?;
            let cost = crate::prices::lookup(&table, rule.target.provider.as_deref().unwrap_or_default(), rule.target.model.as_deref()?)?;
            Some((rule, class, cost))
        }).collect();
        for (easy, easy_class, easy_cost) in &priced {
            for (hard, hard_class, hard_cost) in &priced {
                if easy_class.is_code() == hard_class.is_code() && easy_class.tier() < hard_class.tier() && easy_cost.input > hard_cost.input * 1.2 {
                    advice.push(format!("rule {} ({easy_class}) targets a model priced ${:.2}/M input, more than rule {} ({hard_class}) at ${:.2}/M; simpler requests usually belong on the cheaper model", easy.id, easy_cost.input, hard.id, hard_cost.input));
                }
            }
        }
    }
    json!({ "version": INSTALL_VERSION, "protocolVersion": PROTOCOL_VERSION, "ready": problems.is_empty(), "problems": problems, "advice": advice, "hosts": hosts, "config": summary })
}

fn installation_hash(host: Host, command: &str) -> String { let mut hash = 2_166_136_261_u64; for byte in format!("{}:{command}:{INSTALL_VERSION}", host.as_str()).bytes() { hash = (hash ^ u64::from(byte)).wrapping_mul(16_777_619); } format!("{hash:016x}") }
fn is_owned_installation(path: &Path) -> bool { let Ok(contents) = fs::read_to_string(path) else { return false; }; let Ok(value) = serde_json::from_str::<Value>(&contents) else { return false; }; contents.contains(OWNER_KEY) || value.get(OWNER_KEY).is_some() }
fn host_installation_present(host: Host) -> bool {
    if host == Host::ClaudeCode {
        let project = env::current_dir().ok().map(|directory| crate::claude_code::settings_path(false, &project_root(&directory)));
        return [Some(default_config_path(host)), project].into_iter().flatten().any(|path| crate::claude_code::is_installed(&path));
    }
    if host != Host::OpenCode { return is_owned_installation(&default_config_path(host)); }
    let global = opencode_config_dir();
    let project = env::current_dir().ok().map(|directory| project_root(&directory).join(".opencode"));
    [Some(global), project].into_iter().flatten().any(|root| {
        root.join(OPENCODE_MANIFEST).is_file() || has_legacy_project_install(&root)
    })
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> { if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| format!("create host config directory: {e}"))?; } let bytes = serde_json::to_vec_pretty(value).map_err(|e| format!("serialize host config: {e}"))?; fs::write(path, bytes).map_err(|e| format!("write host config: {e}")) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn installation_is_idempotent_and_preserves_fields() { let directory = tempfile::tempdir().unwrap(); let path = directory.path().join("settings.json"); fs::write(&path, r#"{"other":true}"#).unwrap(); install(Host::ClaudeCode, Some(&path), Some("system1router")).unwrap(); install(Host::ClaudeCode, Some(&path), Some("system1router")).unwrap(); let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap(); assert_eq!(value["other"], true); assert_eq!(value["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1); assert!(!value.to_string().contains(OWNER_KEY)); }

    #[test]
    fn opencode_global_install_is_repeatable_and_preserves_existing_tui_settings() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let tui_json = root.join("tui.json");
        let original = r#"{"$schema":"https://opencode.ai/tui.json","theme":"tokyo"}"#;
        fs::write(&tui_json, original).unwrap();

        let installed = install_opencode_at(root, OpenCodeScope::Global, None).unwrap();
        assert_eq!(installed["status"], "installed");
        let server = fs::read_to_string(root.join(OPENCODE_SERVER_FILE)).unwrap();
        assert!(server.starts_with(OPENCODE_MARKER));
        assert!(server.contains("../system1router-shared"));
        let shared = fs::read_to_string(root.join(OPENCODE_SHARED_FILE)).unwrap();
        assert!(shared.starts_with(OPENCODE_MARKER));
        let tui = fs::read_to_string(root.join(OPENCODE_TUI_FILE)).unwrap();
        assert!(tui.contains("./system1router-shared"));
        assert_eq!(fs::read_to_string(&tui_json).unwrap(), original);
        let managed_tui = fs::read_to_string(root.join("tui.jsonc")).unwrap();
        assert!(managed_tui.starts_with(OPENCODE_MARKER));
        let parsed_tui: Value = serde_json::from_str(managed_tui.strip_prefix(OPENCODE_MARKER).unwrap()).unwrap();
        assert_eq!(parsed_tui["plugin"][0], OPENCODE_TUI_SPEC);

        install_opencode_at(root, OpenCodeScope::Global, None).unwrap();
        assert_eq!(fs::read_to_string(&tui_json).unwrap(), original);
        uninstall_opencode_at(root, OpenCodeScope::Global).unwrap();
        assert_eq!(fs::read_to_string(&tui_json).unwrap(), original);
        assert!(!root.join("tui.jsonc").exists());
        assert!(!root.join(OPENCODE_SERVER_FILE).exists());
        assert!(!root.join(OPENCODE_SHARED_FILE).exists());
        assert!(!root.join(OPENCODE_TUI_FILE).exists());
    }

    #[test]
    fn opencode_install_refuses_to_overwrite_an_unmanaged_plugin() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let plugin_dir = root.join("plugins");
        fs::create_dir_all(&plugin_dir).unwrap();
        let server = root.join(OPENCODE_SERVER_FILE);
        fs::write(&server, "user plugin").unwrap();
        assert!(install_opencode_at(root, OpenCodeScope::Global, None).is_err());
        assert_eq!(fs::read_to_string(server).unwrap(), "user plugin");
        assert!(!root.join("tui.json").exists());
    }

    #[test]
    fn opencode_project_install_uses_the_project_config_directory() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join(".opencode");
        let installed = install_opencode_at(&root, OpenCodeScope::Project, None).unwrap();
        assert_eq!(installed["scope"], "project");
        assert!(root.join(OPENCODE_SERVER_FILE).is_file());
        assert!(root.join(OPENCODE_TUI_FILE).is_file());
        assert!(root.join("tui.json").is_file());
        assert_eq!(uninstall_opencode_at(&root, OpenCodeScope::Project).unwrap()["status"], "uninstalled");
    }

    #[test]
    fn existing_repository_install_is_not_duplicated() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join(".opencode");
        let plugin = root.join("plugins/system1router");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("server.ts"), "server").unwrap();
        fs::write(plugin.join("tui.ts"), "tui").unwrap();
        fs::write(root.join("opencode.json"), "{}") .unwrap();
        fs::write(root.join("tui.json"), "{}") .unwrap();
        let result = install_opencode_at(&root, OpenCodeScope::Project, None).unwrap();
        assert_eq!(result["status"], "already-installed");
        assert!(!root.join(OPENCODE_SERVER_FILE).exists());
    }
}
