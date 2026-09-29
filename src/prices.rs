//! Model prices from the LiteLLM price table, used when the host reports none
//! (OpenCode reports zero for subscription models; Claude Code reports none).
//!
//! The table is ~3 MB, so it is downloaded outside the request path and
//! stored trimmed to per-million-token prices. A stale or missing cache
//! triggers a detached background refresh; the request uses what is cached.
use crate::protocol::ModelCost;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs, path::PathBuf, time::{Duration, SystemTime}};

pub const DEFAULT_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/refs/heads/main/model_prices_and_context_window.json";
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PriceEntry {
    pub provider: String,
    /// USD per million tokens.
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
}

pub type PriceTable = BTreeMap<String, PriceEntry>;

pub fn cache_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("SYSTEM1ROUTER_PRICES") { return Some(PathBuf::from(path)); }
    if let Ok(base) = env::var("XDG_STATE_HOME") { return Some(PathBuf::from(base).join("system1router/model-prices.json")); }
    #[cfg(windows)]
    { env::var("APPDATA").ok().map(|base| PathBuf::from(base).join("system1router/model-prices.json")) }
    #[cfg(not(windows))]
    { env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local/state/system1router/model-prices.json")) }
}

pub fn load() -> Option<PriceTable> {
    serde_json::from_slice(&fs::read(cache_path()?).ok()?).ok()
}

pub fn is_stale() -> bool {
    let Some(path) = cache_path() else { return false; };
    fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_none_or(|age| age > MAX_AGE)
}

/// Starts `system1router __refresh-prices` detached so the current request is not delayed.
pub fn refresh_in_background() {
    let Ok(executable) = env::current_exe() else { return; };
    let _ = std::process::Command::new(executable).arg("__refresh-prices")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn();
}

/// Downloads and stores the trimmed table. Returns the number of models.
pub fn refresh(url: &str) -> Result<usize, String> {
    let response = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build().get(url).call().map_err(|error| format!("download prices: {error}"))?;
    let raw: Value = response.into_json().map_err(|error| format!("parse prices: {error}"))?;
    let table = trim(&raw);
    if table.is_empty() { return Err("price table contained no usable entries".into()); }
    let path = cache_path().ok_or("no state directory for the price cache")?;
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?; }
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temporary, serde_json::to_vec(&table).map_err(|error| error.to_string())?).map_err(|error| format!("write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &path).map_err(|error| format!("replace {}: {error}", path.display()))?;
    Ok(table.len())
}

pub fn trim(raw: &Value) -> PriceTable {
    let Some(entries) = raw.as_object() else { return PriceTable::new(); };
    entries.iter().filter_map(|(name, entry)| {
        let per_million = |key: &str| entry.get(key).and_then(Value::as_f64).map(|value| value * 1e6);
        let input = per_million("input_cost_per_token")?;
        Some((name.clone(), PriceEntry {
            provider: entry.get("litellm_provider").and_then(Value::as_str).unwrap_or_default().into(),
            input,
            output: per_million("output_cost_per_token").unwrap_or(0.0),
            cache_read: per_million("cache_read_input_token_cost").unwrap_or(0.0),
        }))
    }).collect()
}

/// Finds a model by `provider/model`, then by bare name on the same provider.
/// Claude aliases (`opus`, `sonnet`, `haiku`) resolve to the newest undated id.
pub fn lookup(table: &PriceTable, provider: &str, model: &str) -> Option<ModelCost> {
    let same_provider = |entry: &PriceEntry| provider.is_empty() || provider == "default" || entry.provider == provider;
    let entry = table.get(&format!("{provider}/{model}"))
        .or_else(|| table.get(model).filter(|entry| same_provider(entry)))
        .or_else(|| {
            let prefix = format!("claude-{model}-");
            table.iter()
                .filter(|(name, entry)| name.starts_with(&prefix) && entry.provider == "anthropic" && !name.rsplit('-').next().is_some_and(|last| last.len() == 8 && last.chars().all(|c| c.is_ascii_digit())))
                .max_by(|(a, _), (b, _)| a.cmp(b))
                .map(|(_, entry)| entry)
        })?;
    Some(ModelCost { input: entry.input, output: entry.output, cache_read: entry.cache_read })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table() -> PriceTable {
        trim(&json!({
            "sample_spec": { "input_cost_per_token": "doc" },
            "gpt-6-luna": { "input_cost_per_token": 1e-7, "output_cost_per_token": 5e-7, "cache_read_input_token_cost": 1e-8, "litellm_provider": "openai" },
            "azure/gpt-6-luna": { "input_cost_per_token": 9e-7, "litellm_provider": "azure" },
            "claude-opus-4-5": { "input_cost_per_token": 5e-6, "litellm_provider": "anthropic" },
            "claude-opus-5-5": { "input_cost_per_token": 4e-6, "cache_read_input_token_cost": 2e-7, "litellm_provider": "anthropic" },
            "claude-opus-5-5-20260101": { "input_cost_per_token": 9e-6, "litellm_provider": "anthropic" }
        }))
    }

    #[test]
    fn trims_to_per_million_prices() {
        let table = table();
        assert!(!table.contains_key("sample_spec"));
        let luna = &table["gpt-6-luna"];
        assert!((luna.input - 0.1).abs() < 1e-9 && (luna.cache_read - 0.01).abs() < 1e-9);
    }

    #[test]
    fn lookup_respects_provider_and_aliases() {
        let table = table();
        assert!((lookup(&table, "openai", "gpt-6-luna").unwrap().input - 0.1).abs() < 1e-9);
        assert!((lookup(&table, "azure", "gpt-6-luna").unwrap().input - 0.9).abs() < 1e-9);
        assert!(lookup(&table, "anthropic", "gpt-6-luna").is_none());
        assert!((lookup(&table, "anthropic", "opus").unwrap().input - 4.0).abs() < 1e-9, "newest undated alias");
        assert!((lookup(&table, "anthropic", "claude-opus-5-5").unwrap().cache_read - 0.2).abs() < 1e-9);
    }
}
