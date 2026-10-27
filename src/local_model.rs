//! Local effort model (`providers.selected: "local"`, Ettin by default).
//!
//! One process per user holds the model, whatever the number of Claude Code
//! and OpenCode sessions: hook calls ask it over a Unix socket and start it
//! when it is not running. It remembers the assistant processes that asked
//! (the caller's parent, past a shell) and exits once none is alive, or after
//! `idleMinutes` without a request.
use crate::{classification::ProviderError, config::{Config, ClassSelection, LocalConfig}, protocol::{effort_rank, ClassificationConfidence, NormalizedEvent, ProviderMetadata, RoutingClass, RoutingClassification}, redaction::project_event};
use serde_json::{json, Value};
use std::{collections::HashSet, env, fs, io::{BufRead, BufReader, Write}, os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}, process::CommandExt}, path::{Path, PathBuf}, process::{Command, Stdio}, sync::{Arc, Mutex}, thread, time::{Duration, Instant}};

/// Without a live assistant, the model waits this long for one to ask again.
const GRACE: Duration = Duration::from_secs(30);
/// Classes in the order an effort is matched to one: code work first, as most prompts are.
const CLASS_ORDER: [RoutingClass; 6] = [RoutingClass::CodeSimple, RoutingClass::CodeModerate, RoutingClass::CodeHard, RoutingClass::QuestionSimple, RoutingClass::QuestionModerate, RoutingClass::QuestionHard];

pub fn data_directory() -> Option<PathBuf> {
    if let Ok(base) = env::var("XDG_DATA_HOME") { return Some(PathBuf::from(base).join("system1router")); }
    env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local/share/system1router"))
}

pub fn model_directory(local: &LocalConfig) -> Option<PathBuf> { data_directory().map(|base| base.join("models").join(&local.model)) }

pub fn socket_path() -> Option<PathBuf> {
    if let Ok(base) = env::var("XDG_RUNTIME_DIR") { return Some(PathBuf::from(base).join("system1router-model.sock")); }
    crate::control::state_directory().map(|base| base.join("model.sock"))
}

pub fn installed(local: &LocalConfig) -> bool { model_directory(local).is_some_and(|dir| ["model.onnx", "tokenizer.json", "model.json"].iter().all(|file| dir.join(file).is_file())) }

/// The repository's `models/` folder; GitHub serves its Git LFS files from here.
pub const MODELS_URL: &str = "https://github.com/Valkea/System1Router/raw/main/models";

/// Fetches `<name>.json`, `<name>.onnx` and `<name>.tokenizer.json` from `from`: a
/// URL or a local folder. Without one, from `providers.local.url`, then from the
/// `models/` folder (Git LFS) of the checkout the binary was built from, then
/// from that folder on GitHub.
pub fn install(local: &LocalConfig, from: Option<&str>) -> Result<PathBuf, String> {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    // A clone without Git LFS holds pointers, not the weights: download those instead.
    let bundled = fs::metadata(checkout.join(format!("{}.onnx", local.model))).is_ok_and(|file| file.len() > 1 << 20).then(|| checkout.to_string_lossy().into_owned());
    let source = from.map(str::to_owned).or_else(|| local.url.clone()).or(bundled).unwrap_or_else(|| MODELS_URL.into());
    let dir = model_directory(local).ok_or("no data directory (HOME is not set)")?;
    fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let name = &local.model;
    let manifest: Value = serde_json::from_slice(&fetch(&source, &format!("{name}.json"))?).map_err(|error| format!("{name}.json: {error}"))?;
    for (kind, target) in [("onnx", "model.onnx"), ("tokenizer.json", "tokenizer.json")] {
        let file = manifest["files"][kind]["name"].as_str().ok_or(format!("{name}.json lists no {kind} file"))?;
        let bytes = fetch(&source, file)?;
        if bytes.starts_with(b"version https://git-lfs") { return Err(format!("{file} is a Git LFS pointer, not the file: run `git lfs pull` in the checkout")); }
        if manifest["files"][kind]["bytes"].as_u64().is_some_and(|size| size != bytes.len() as u64) { return Err(format!("{file}: got {} bytes, the manifest says {}", bytes.len(), manifest["files"][kind]["bytes"])); }
        write_private(&dir.join(target), &bytes)?;
    }
    write_private(&dir.join("model.json"), &serde_json::to_vec_pretty(&manifest).unwrap_or_default())?;
    Ok(dir)
}

fn fetch(source: &str, file: &str) -> Result<Vec<u8>, String> {
    if !source.starts_with("http://") && !source.starts_with("https://") { return fs::read(Path::new(source).join(file)).map_err(|error| format!("read {file}: {error}")); }
    let url = format!("{}/{file}", source.trim_end_matches('/'));
    let response = ureq::get(&url).timeout(Duration::from_secs(600)).call().map_err(|error| format!("download {url}: {error}"))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(response.into_reader(), 1 << 31), &mut bytes).map_err(|error| format!("download {url}: {error}"))?;
    Ok(bytes)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let partial = path.with_extension("partial");
    fs::write(&partial, bytes).and_then(|_| fs::rename(&partial, path)).map_err(|error| format!("write {}: {error}", path.display()))
}

/// The routing class a model effort stands for: the first class `effort.levels`
/// maps to it, so the policy resolves back to the same effort (or the nearest).
pub fn class_for(effort: &str, config: &Config) -> RoutingClass {
    let rank = effort_rank(effort).unwrap_or(0) as i64;
    CLASS_ORDER.iter().min_by_key(|class| (effort_rank(&config.effort.level(class)).unwrap_or(0) as i64 - rank).abs()).cloned().unwrap_or(RoutingClass::CodeModerate)
}

/// Upper selection: the highest effort holding at least this share of the mass at or above it.
const UPPER_SHARE: f32 = 0.2;

/// The effort index `routing.classSelection` picks: the top one, the median, or the upper tail.
pub fn pick(probabilities: &[f32], selection: &ClassSelection) -> usize {
    let argmax = probabilities.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(index, _)| index);
    match selection {
        ClassSelection::Argmax => argmax,
        ClassSelection::Median => (0..probabilities.len()).find(|&index| probabilities[..=index].iter().sum::<f32>() >= 0.5).unwrap_or(argmax),
        ClassSelection::Upper => (0..probabilities.len()).rev().find(|&index| probabilities[index..].iter().sum::<f32>() >= UPPER_SHARE).unwrap_or(argmax),
    }
}

pub fn classify(event: &NormalizedEvent, config: &Config) -> Result<RoutingClassification, ProviderError> {
    let started = Instant::now();
    let local = &config.providers.local;
    if !installed(local) { return Err(ProviderError::Unavailable(format!("the local model {} is not installed: run `system1router provider local`", local.model))); }
    let input = project_event(event, &config.redaction);
    let request = json!({ "pid": host_pid(), "query": input.query, "context": render_context(&input.context) });
    let deadline = started + Duration::from_millis(config.timeouts.provider_ms);
    let reply = ask(&request, deadline, Some(local)).map_err(|error| if Instant::now() >= deadline { ProviderError::Timeout } else { ProviderError::Unavailable(error) })?;
    let efforts: Vec<String> = serde_json::from_value(reply["efforts"].clone()).map_err(|_| ProviderError::Malformed("no efforts in the model's reply".into()))?;
    let probabilities: Vec<f32> = serde_json::from_value(reply["probabilities"].clone()).ok().filter(|values: &Vec<f32>| values.len() == efforts.len() && !values.is_empty()).ok_or_else(|| ProviderError::Malformed("no probabilities in the model's reply".into()))?;
    let chosen = pick(&probabilities, &config.routing.class_selection);
    let routing_class = class_for(&efforts[chosen], config);
    let task_type = if routing_class.as_str().starts_with("code") { "code" } else { "question" };
    Ok(RoutingClassification { routing_class, task_type: task_type.into(), context_signals: Vec::new(), class_probabilities: Some(efforts.iter().cloned().zip(probabilities.iter().copied()).collect()), confidence: ClassificationConfidence { routing_class: probabilities[chosen], task_type: 1.0, context: 0.0 }, provider_metadata: ProviderMetadata { provider: "local".into(), version: env!("CARGO_PKG_VERSION").into(), model: Some(local.model.clone()), latency_ms: Some(started.elapsed().as_millis() as u64) }, correlation_id: crate::protocol::correlation_id(event) })
}

/// The context as the training data rendered it (experiments/dataset_extract.py):
/// files in play, tools used, then the newest conversation.
pub fn render_context(context: &crate::protocol::BoundedContext) -> String {
    let recent = |values: &[String], count: usize| { let mut seen = HashSet::new(); values.iter().rev().filter(|value| seen.insert(value.as_str())).take(count).cloned().collect::<Vec<_>>().join(", ") };
    let mut parts = Vec::new();
    if !context.files.is_empty() { parts.push(format!("Files in play: {}", recent(&context.files, 8))); }
    if !context.tools.is_empty() { parts.push(format!("Tools used: {}", recent(&context.tools, 10))); }
    if let Some(conversation) = context.conversation_summary.as_deref().filter(|text| !text.trim().is_empty()) {
        let count = conversation.chars().count();
        parts.push(format!("Recent conversation:\n{}", conversation.chars().skip(count.saturating_sub(3000)).collect::<String>()));
    }
    parts.join("\n")
}

/// The assistant process this call serves: the parent, or the shell's parent
/// when a shell ran the hook.
pub fn host_pid() -> u32 {
    let parent = std::os::unix::process::parent_id();
    let name = fs::read_to_string(format!("/proc/{parent}/comm")).unwrap_or_default();
    if !["sh", "bash", "zsh", "dash", "fish"].contains(&name.trim()) { return parent; }
    fs::read_to_string(format!("/proc/{parent}/stat")).ok().and_then(|stat| stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(1)?.parse().ok())).unwrap_or(parent)
}

/// Sends one request, starting the model process when none answers.
pub fn ask(request: &Value, deadline: Instant, start_with: Option<&LocalConfig>) -> Result<Value, String> {
    let socket = socket_path().ok_or("no socket path (HOME is not set)")?;
    let mut child: Option<std::process::Child> = None;
    loop {
        match exchange(&socket, request, deadline) {
            Ok(reply) => return reply.get("error").and_then(Value::as_str).map_or(Ok(reply.clone()), |error| Err(error.to_owned())),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(error) => {
                match child.as_mut() {
                    None => { let Some(local) = start_with else { return Err(error) }; child = Some(start(local)?); }
                    // A failed start is reported at once rather than at the deadline;
                    // exit 0 means another model process is starting.
                    Some(process) => if let Ok(Some(status)) = process.try_wait() { if !status.success() { return Err(format!("the model process failed to start (see {})", log_path().map_or_else(String::new, |path| path.display().to_string()))); } },
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

fn exchange(socket: &Path, request: &Value, deadline: Instant) -> Result<Value, String> {
    let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
    let mut stream = UnixStream::connect(socket).map_err(|error| format!("connect {}: {error}", socket.display()))?;
    stream.set_read_timeout(Some(left)).and_then(|_| stream.set_write_timeout(Some(left))).map_err(|error| error.to_string())?;
    stream.write_all(format!("{request}\n").as_bytes()).map_err(|error| format!("send: {error}"))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|error| format!("receive: {error}"))?;
    serde_json::from_str(&line).map_err(|error| format!("malformed reply: {error}"))
}

fn log_path() -> Option<PathBuf> { crate::control::state_directory().map(|base| base.join("model.log")) }

/// Starts `system1router model serve` in the background, detached from the
/// caller; its errors go to `model.log` in the state directory.
pub fn start(local: &LocalConfig) -> Result<std::process::Child, String> {
    let executable = env::current_exe().map_err(|error| format!("locate system1router: {error}"))?;
    let log = log_path().and_then(|path| { fs::create_dir_all(path.parent()?).ok()?; fs::File::create(path).ok() });
    let mut command = Command::new(executable);
    command.args(["model", "serve", "--model", &local.model]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log.map_or_else(Stdio::null, Stdio::from)).process_group(0);
    if let Some(threads) = local.threads { command.args(["--threads", &threads.to_string()]); }
    if let Some(minutes) = local.idle_minutes { command.args(["--idle-minutes", &minutes.to_string()]); }
    command.spawn().map_err(|error| format!("start the model process: {error}"))
}

fn alive(pid: u32) -> bool { !Path::new("/proc").is_dir() || Path::new(&format!("/proc/{pid}")).exists() }

struct Model { session: ort::session::Session, tokenizer: tokenizers::Tokenizer, efforts: Vec<String>, max_length: usize, context_chars: usize, cls: i64, sep: i64 }

impl Model {
    fn load(dir: &Path, threads: usize) -> Result<Self, String> {
        let manifest: Value = serde_json::from_slice(&fs::read(dir.join("model.json")).map_err(|error| format!("read model.json: {error}"))?).map_err(|error| format!("model.json: {error}"))?;
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|error| format!("tokenizer.json: {error}"))?;
        let environment = ort::init().with_name("system1router").build().map_err(|error| format!("ONNX Runtime: {error}"))?;
        let builder = ort::session::Session::builder(&environment).map_err(|error| format!("ONNX Runtime: {error}"))?;
        let session = builder.with_intra_threads(threads).map_err(|error| format!("ONNX Runtime: {error}"))?.commit_from_file(dir.join("model.onnx")).map_err(|error| format!("model.onnx: {error}"))?;
        let efforts = serde_json::from_value(manifest["efforts"].clone()).map_err(|_| "model.json lists no efforts")?;
        let number = |key: &str| manifest[key].as_i64().ok_or(format!("model.json has no {key}"));
        Ok(Self { session, tokenizer, efforts, max_length: number("maxLength")? as usize, context_chars: number("contextChars")? as usize, cls: number("cls")?, sep: number("sep")? })
    }

    /// `[CLS] request [SEP] newest context [SEP]`, as train_classifier.encode builds it.
    fn encode(&self, query: &str, context: &str) -> Result<Vec<i64>, String> {
        let tokens = |text: &str| self.tokenizer.encode(text, false).map(|encoding| encoding.get_ids().iter().map(|&id| id as i64).collect::<Vec<_>>()).map_err(|error| error.to_string());
        let mut request = tokens(query)?;
        request.truncate(self.max_length / 3);
        let count = context.chars().count();
        let context = tokens(&context.chars().skip(count.saturating_sub(self.context_chars)).collect::<String>())?;
        let room = self.max_length.saturating_sub(request.len() + 3);
        let context = &context[context.len().saturating_sub(room)..];
        Ok([vec![self.cls], request, vec![self.sep], context.to_vec(), vec![self.sep]].concat())
    }

    fn probabilities(&mut self, query: &str, context: &str) -> Result<Vec<f32>, String> {
        let ids = self.encode(query, context)?;
        let shape = [1usize, ids.len()];
        let mask = vec![1i64; ids.len()];
        let inputs = ort::inputs!["input_ids" => ort::value::Tensor::from_array((shape, ids)).map_err(|error| error.to_string())?, "attention_mask" => ort::value::Tensor::from_array((shape, mask)).map_err(|error| error.to_string())?];
        let outputs = self.session.run(inputs).map_err(|error| error.to_string())?;
        let (_, logits) = outputs[0].try_extract_tensor::<f32>().map_err(|error| error.to_string())?;
        let top = logits.iter().copied().fold(f32::MIN, f32::max);
        let exp: Vec<f32> = logits.iter().map(|value| (value - top).exp()).collect();
        let total: f32 = exp.iter().sum();
        Ok(exp.iter().map(|value| value / total).collect())
    }
}

struct Hosts { pids: HashSet<u32>, last_request: Instant, without_hosts: Option<Instant> }

/// `system1router model serve`: the shared model process. Returns at once when
/// another one already serves the socket.
pub fn serve(local: &LocalConfig) -> Result<(), String> {
    let socket = socket_path().ok_or("no socket path (HOME is not set)")?;
    if let Some(parent) = socket.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; }
    let lock = fs::File::create(socket.with_extension("lock")).map_err(|error| format!("lock: {error}"))?;
    if lock.try_lock().is_err() { return Ok(()); }
    let dir = model_directory(local).filter(|_| installed(local)).ok_or(format!("the local model {} is not installed: run `system1router model install`", local.model))?;
    let mut model = Model::load(&dir, local.threads.unwrap_or(2))?;
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).map_err(|error| format!("bind {}: {error}", socket.display()))?;
    let _ = fs::set_permissions(&socket, fs::Permissions::from_mode(0o600));
    let hosts = Arc::new(Mutex::new(Hosts { pids: HashSet::new(), last_request: Instant::now(), without_hosts: None }));
    let idle = Duration::from_secs(local.idle_minutes.unwrap_or(30) * 60);
    {
        let (hosts, socket) = (hosts.clone(), socket.clone());
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(2));
            let mut state = hosts.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            state.pids.retain(|&pid| alive(pid));
            if !state.pids.is_empty() { state.without_hosts = None; } else if state.without_hosts.is_none() { state.without_hosts = Some(Instant::now()); }
            if state.without_hosts.is_some_and(|since| since.elapsed() >= GRACE) || state.last_request.elapsed() >= idle {
                let _ = fs::remove_file(&socket);
                std::process::exit(0);
            }
        });
    }
    for stream in listener.incoming().flatten() {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() { continue; }
        let request: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        let reply = {
            let mut state = hosts.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            state.last_request = Instant::now();
            if let Some(pid) = request["pid"].as_u64().filter(|&pid| pid > 1) { state.pids.insert(pid as u32); }
            if request["stop"] == true { let _ = (&stream).write_all(b"{\"stopped\":true}\n"); let _ = fs::remove_file(&socket); std::process::exit(0); }
            if request.get("query").is_none() { Some(json!({ "model": local.model, "hosts": state.pids.len(), "pid": std::process::id() })) } else { None }
        };
        let reply = reply.unwrap_or_else(|| {
            let begun = Instant::now();
            match model.probabilities(request["query"].as_str().unwrap_or_default(), request["context"].as_str().unwrap_or_default()) {
                Ok(probabilities) => json!({ "efforts": model.efforts, "probabilities": probabilities, "ms": begun.elapsed().as_millis() as u64 }),
                Err(error) => json!({ "error": error }),
            }
        });
        let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{BoundedContext, NormalizedEvent};

    #[test]
    fn selection_picks_top_median_or_upper_effort() {
        let probabilities = [0.1, 0.45, 0.25, 0.2, 0.0];
        assert_eq!(pick(&probabilities, &ClassSelection::Argmax), 1);
        assert_eq!(pick(&probabilities, &ClassSelection::Median), 1);
        assert_eq!(pick(&probabilities, &ClassSelection::Upper), 3, "xhigh holds 20% of the mass");
        assert_eq!(pick(&[0.3, 0.1, 0.6, 0.0, 0.0], &ClassSelection::Median), 2);
    }

    #[test]
    fn an_effort_maps_to_a_class_the_levels_send_back_to_it() {
        let mut config = Config::default();
        for effort in ["low", "medium", "high", "xhigh"] { assert_eq!(config.effort.level(&class_for(effort, &config)), effort); }
        assert_eq!(class_for("max", &config), RoutingClass::CodeHard, "no class takes max: the nearest one");
        config.effort.levels.insert("code_simple".into(), "minimal".into());
        assert_eq!(config.effort.level(&class_for("low", &config)), "low", "question_simple still takes low");
    }

    #[test]
    fn context_renders_as_the_training_data() {
        let context = BoundedContext { files: vec!["a.rs".into(), "b.rs".into(), "a.rs".into()], tools: vec!["Read".into(), "Bash".into()], conversation_summary: Some(format!("User:\n{}", "x".repeat(4000))), ..BoundedContext::default() };
        let text = render_context(&context);
        assert!(text.starts_with("Files in play: a.rs, b.rs\nTools used: Bash, Read\nRecent conversation:\n"), "newest first, without repeats");
        assert_eq!(text.lines().nth(3).map(str::len), Some(3000), "the newest 3,000 characters");
        assert_eq!(render_context(&BoundedContext::default()), "");
    }

    // The only test that sets XDG_DATA_HOME.
    #[test]
    fn install_copies_a_release_folder_and_classify_needs_it() {
        let directory = tempfile::tempdir().unwrap();
        let previous = env::var_os("XDG_DATA_HOME");
        env::set_var("XDG_DATA_HOME", directory.path().join("data"));
        let release = directory.path().join("release");
        fs::create_dir_all(&release).unwrap();
        fs::write(release.join("m.onnx"), b"onnx").unwrap();
        fs::write(release.join("m.tokenizer.json"), b"{}").unwrap();
        fs::write(release.join("m.json"), json!({ "files": { "onnx": { "name": "m.onnx", "bytes": 4 }, "tokenizer.json": { "name": "m.tokenizer.json", "bytes": 2 } } }).to_string()).unwrap();
        let local = LocalConfig { model: "m".into(), ..LocalConfig::default() };
        let mut config = Config::default();
        config.providers.local = local.clone();
        let event: NormalizedEvent = serde_json::from_value(json!({ "protocolVersion": "1", "host": "generic", "event": "user_prompt", "sessionId": "s", "request": { "query": "hi" } })).unwrap();
        let missing = classify(&event, &config);
        let wrong_size = { fs::write(release.join("m.onnx"), b"truncated").unwrap(); let result = install(&local, release.to_str()); fs::write(release.join("m.onnx"), b"onnx").unwrap(); result };
        let pointer = { fs::write(release.join("m.onnx"), b"version https://git-lfs.github.com/spec/v1\n").unwrap(); let result = install(&local, release.to_str()); fs::write(release.join("m.onnx"), b"onnx").unwrap(); result };
        let installed_to = install(&local, release.to_str());
        match previous { Some(value) => env::set_var("XDG_DATA_HOME", value), None => env::remove_var("XDG_DATA_HOME") }
        assert!(matches!(missing, Err(ProviderError::Unavailable(_))));
        assert!(wrong_size.is_err_and(|error| error.contains("bytes")));
        assert!(pointer.is_err_and(|error| error.contains("git lfs pull")));
        let dir = installed_to.unwrap();
        assert_eq!(fs::read(dir.join("model.onnx")).unwrap(), b"onnx");
        assert!(dir.join("tokenizer.json").is_file() && dir.join("model.json").is_file());
    }
}
