//! Profile and configuration management

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::timeout;

use claude_code::{ClaudeAgentOptions, Prompt, SubprocessCliTransport};
use codex::{CodexExec, ModelReasoningEffort};

use crate::{
    error::{ExecutorError, Result},
    types::{ExecutorType, PermissionPolicy},
};

const DISCOVERY_TTL: Duration = Duration::from_secs(5 * 60);
const DISCOVERY_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

const CODEX_MODEL_DISCOVERY_COMMANDS: &[&[&str]] = &[
    &["models", "list", "--json"],
    &["models", "--json"],
    &["model", "list", "--json"],
    &["models", "list"],
];

const CODEX_REASONING_DISCOVERY_COMMANDS: &[&[&str]] = &[
    &["reasoning", "list", "--json"],
    &["reasoning", "--json"],
    &["models", "reasoning", "--json"],
    &["reasoning", "list"],
];

const CLAUDE_MODEL_DISCOVERY_COMMANDS: &[&[&str]] = &[
    &["models", "list", "--json"],
    &["models", "--json"],
    &["model", "list", "--json"],
    &["models", "list"],
];

const CLAUDE_REASONING_DISCOVERY_COMMANDS: &[&[&str]] = &[
    &["effort", "list", "--json"],
    &["reasoning", "list", "--json"],
    &["models", "reasoning", "--json"],
    &["effort", "list"],
];

/// Profile identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId {
    /// Target executor backend.
    pub executor: ExecutorType,
    /// Optional variant (for example `default`, `plan`, `fast`).
    pub variant: Option<String>,
}

impl ProfileId {
    /// Creates a profile identifier.
    pub fn new(executor: ExecutorType, variant: Option<String>) -> Self {
        Self { executor, variant }
    }
}

/// Executor configuration with overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorConfig {
    /// Profile key used to load base configuration.
    pub profile_id: ProfileId,
    /// Runtime model override.
    pub model_override: Option<String>,
    /// Runtime reasoning override.
    pub reasoning_override: Option<String>,
    /// Runtime permission override.
    pub permission_policy: Option<PermissionPolicy>,
}

/// Resolved configuration
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Resolved model value.
    pub model: Option<String>,
    /// Resolved reasoning value.
    pub reasoning: Option<String>,
    /// Resolved permission policy.
    pub permission_policy: Option<PermissionPolicy>,
}

/// Dynamic discovery data for one executor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryData {
    /// Models discovered from CLI/SDK probing.
    pub models: Vec<String>,
    /// Reasoning levels discovered from CLI/SDK probing.
    pub reasoning_levels: Vec<String>,
}

#[derive(Debug, Clone)]
struct CachedDiscovery {
    data: DiscoveryData,
    expires_at: Instant,
}

/// Profile manager
pub struct ProfileManager {
    config_path: PathBuf,
    cache: RwLock<ProfileCache>,
    discovery_cache: RwLock<HashMap<ExecutorType, CachedDiscovery>>,
    discovery_ttl: Duration,
}

/// Profile values loaded from profile configuration files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileData {
    #[serde(default)]
    /// Optional default model.
    pub model: Option<String>,
    #[serde(default)]
    /// Optional default reasoning level.
    pub reasoning: Option<String>,
    #[serde(default)]
    /// Optional default permission policy.
    pub permission_policy: Option<PermissionPolicy>,
}

#[derive(Debug, Default)]
struct ProfileCache {
    loaded: bool,
    file_mtime: Option<SystemTime>,
    profiles: ProfilesFile,
    resolved: HashMap<ProfileId, ProfileData>,
}

type ProfilesFile = HashMap<String, HashMap<String, ProfileData>>;

#[derive(Debug, Deserialize)]
struct RawProfileData {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    permission_policy: Option<String>,
}

type RawProfilesFile = HashMap<String, HashMap<String, RawProfileData>>;

impl ProfileManager {
    /// Creates a profile manager using the default profile path.
    pub fn new() -> Self {
        Self::with_path(default_profile_path())
    }

    /// Creates a profile manager bound to a specific profile JSON file path.
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: path.into(),
            cache: RwLock::new(ProfileCache::default()),
            discovery_cache: RwLock::new(HashMap::new()),
            discovery_ttl: DISCOVERY_TTL,
        }
    }

    /// Loads profile data by id, with automatic file-change reloading.
    pub async fn load(&self, id: &ProfileId) -> Result<ProfileData> {
        self.reload_if_needed(false).await?;

        let mut cache = self.cache.write().await;
        if let Some(data) = cache.resolved.get(id) {
            return Ok(data.clone());
        }

        let data = resolve_profile_data(&cache.profiles, id);
        cache.resolved.insert(id.clone(), data.clone());
        Ok(data)
    }

    /// Returns models and reasoning levels discovered from the underlying SDK/CLI.
    ///
    /// Discovery is cached with a 5-minute TTL to avoid frequent subprocess calls.
    /// Any discovery error gracefully falls back to built-in defaults.
    pub async fn discover(&self, executor: ExecutorType) -> DiscoveryData {
        {
            let cache = self.discovery_cache.read().await;
            if let Some(entry) = cache.get(&executor)
                && entry.expires_at > Instant::now()
            {
                return entry.data.clone();
            }
        }

        let data = self.discover_fresh(executor).await;
        let mut cache = self.discovery_cache.write().await;
        cache.insert(
            executor,
            CachedDiscovery {
                data: data.clone(),
                expires_at: Instant::now() + self.discovery_ttl,
            },
        );

        data
    }

    /// Resolves runtime configuration by merging:
    /// profile defaults -> variant overrides -> runtime overrides -> discovered fallbacks.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use unified_agent_sdk::{ExecutorConfig, ExecutorType, ProfileId, ProfileManager};
    ///
    /// # async fn run() -> unified_agent_sdk::Result<()> {
    /// let manager = ProfileManager::new();
    /// let resolved = manager
    ///     .resolve(&ExecutorConfig {
    ///         profile_id: ProfileId::new(ExecutorType::Codex, Some("default".to_string())),
    ///         model_override: None,
    ///         reasoning_override: Some("high".to_string()),
    ///         permission_policy: None,
    ///     })
    ///     .await?;
    ///
    /// let _model = resolved.model;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resolve(&self, config: &ExecutorConfig) -> Result<ResolvedConfig> {
        let profile = self.load(&config.profile_id).await?;
        let should_discover_model = config.model_override.is_none() && profile.model.is_none();
        let should_discover_reasoning =
            config.reasoning_override.is_none() && profile.reasoning.is_none();

        let discovery = if should_discover_model || should_discover_reasoning {
            Some(self.discover(config.profile_id.executor).await)
        } else {
            None
        };

        let discovered_model = discovery
            .as_ref()
            .and_then(|data| preferred_model(&data.models));
        let discovered_reasoning = discovery
            .as_ref()
            .and_then(|data| preferred_reasoning_level(&data.reasoning_levels));

        Ok(ResolvedConfig {
            model: config
                .model_override
                .clone()
                .or(profile.model)
                .or(discovered_model),
            reasoning: config
                .reasoning_override
                .clone()
                .or(profile.reasoning)
                .or(discovered_reasoning),
            permission_policy: config.permission_policy.or(profile.permission_policy),
        })
    }

    /// Force reload profiles from disk and clear resolved cache.
    pub async fn reload(&self) -> Result<()> {
        self.reload_if_needed(true).await
    }

    async fn reload_if_needed(&self, force: bool) -> Result<()> {
        let disk_mtime = read_modified_time(&self.config_path)?;

        let mut cache = self.cache.write().await;
        if !force && cache.loaded && cache.file_mtime == disk_mtime {
            return Ok(());
        }

        cache.profiles = load_profiles_from_disk(&self.config_path)?;
        cache.resolved.clear();
        cache.file_mtime = disk_mtime;
        cache.loaded = true;
        Ok(())
    }

    async fn discover_fresh(&self, executor: ExecutorType) -> DiscoveryData {
        match executor {
            ExecutorType::Codex => discover_codex().await,
            ExecutorType::ClaudeCode => discover_claude_code().await,
        }
    }
}

impl Default for ProfileManager {
    fn default() -> Self {
        Self::new()
    }
}

fn default_profile_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".unified-agent")
            .join("profiles.json");
    }

    PathBuf::from(".unified-agent").join("profiles.json")
}

fn read_modified_time(path: &Path) -> Result<Option<SystemTime>> {
    if !path.exists() {
        return Ok(None);
    }

    let modified_time = fs::metadata(path)?.modified()?;
    Ok(Some(modified_time))
}

fn load_profiles_from_disk(path: &Path) -> Result<ProfilesFile> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let parsed: RawProfilesFile = serde_json::from_str(&raw).map_err(|err| {
        ExecutorError::invalid_config(
            format!("failed to parse profile config at {}", path.display()),
            err,
        )
    })?;

    let mut profiles: ProfilesFile = HashMap::new();
    for (executor, variants) in parsed {
        let executor_key = normalize_executor_key(&executor);
        let profile_map = profiles.entry(executor_key).or_default();

        for (variant, data) in variants {
            let permission_policy = parse_permission_policy(data.permission_policy.as_deref())
                .map_err(|err| {
                    ExecutorError::invalid_config(
                        format!("invalid permission_policy for profile {executor}.{variant}"),
                        err,
                    )
                })?;

            profile_map.insert(
                normalize_variant_key(&variant),
                ProfileData {
                    model: data.model,
                    reasoning: data.reasoning,
                    permission_policy,
                },
            );
        }
    }

    Ok(profiles)
}

fn parse_permission_policy(value: Option<&str>) -> Result<Option<PermissionPolicy>> {
    let Some(raw) = value else {
        return Ok(None);
    };

    let normalized = raw.trim().to_ascii_lowercase();
    let policy = match normalized.as_str() {
        "bypass" => PermissionPolicy::Bypass,
        "prompt" => PermissionPolicy::Prompt,
        "deny" => PermissionPolicy::Deny,
        _ => {
            return Err(ExecutorError::invalid_config(
                "failed to parse permission_policy",
                format!("unsupported value: {raw}"),
            ));
        }
    };

    Ok(Some(policy))
}

fn resolve_profile_data(profiles: &ProfilesFile, id: &ProfileId) -> ProfileData {
    let Some(variants) = profiles.get(executor_key(id.executor)) else {
        return ProfileData::default();
    };

    let default_profile = variants.get("default").cloned().unwrap_or_default();
    let Some(variant) = id.variant.as_deref() else {
        return default_profile;
    };

    let variant_key = normalize_variant_key(variant);
    if variant_key == "default" {
        return default_profile;
    }

    match variants.get(&variant_key) {
        Some(variant_profile) => merge_profile_data(&default_profile, variant_profile),
        None => default_profile,
    }
}

fn merge_profile_data(base: &ProfileData, variant: &ProfileData) -> ProfileData {
    ProfileData {
        model: variant.model.clone().or_else(|| base.model.clone()),
        reasoning: variant.reasoning.clone().or_else(|| base.reasoning.clone()),
        permission_policy: variant.permission_policy.or(base.permission_policy),
    }
}

fn executor_key(executor: ExecutorType) -> &'static str {
    match executor {
        ExecutorType::ClaudeCode => "claude-code",
        ExecutorType::Codex => "codex",
    }
}

fn normalize_executor_key(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn normalize_variant_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

async fn discover_codex() -> DiscoveryData {
    let fallback = fallback_discovery(ExecutorType::Codex);
    let codex_program = match CodexExec::new(None, None, None) {
        Ok(exec) => exec.executable_path().to_string(),
        Err(_) => return fallback,
    };

    let models = discover_from_commands(
        &codex_program,
        CODEX_MODEL_DISCOVERY_COMMANDS,
        parse_models_output,
    )
    .await
    .unwrap_or_default();

    let reasoning_levels = discover_from_commands(
        &codex_program,
        CODEX_REASONING_DISCOVERY_COMMANDS,
        parse_reasoning_output,
    )
    .await
    .unwrap_or_else(codex_reasoning_levels);

    DiscoveryData {
        models,
        reasoning_levels,
    }
}

async fn discover_claude_code() -> DiscoveryData {
    let fallback = fallback_discovery(ExecutorType::ClaudeCode);
    let claude_program = match resolve_claude_cli_path() {
        Some(path) => path,
        None => return fallback,
    };

    let models = discover_from_commands(
        &claude_program,
        CLAUDE_MODEL_DISCOVERY_COMMANDS,
        parse_models_output,
    )
    .await
    .unwrap_or_default();

    let reasoning_levels = discover_from_commands(
        &claude_program,
        CLAUDE_REASONING_DISCOVERY_COMMANDS,
        parse_reasoning_output,
    )
    .await
    .unwrap_or_else(claude_reasoning_levels);

    DiscoveryData {
        models,
        reasoning_levels,
    }
}

fn fallback_discovery(executor: ExecutorType) -> DiscoveryData {
    match executor {
        ExecutorType::Codex => DiscoveryData {
            models: Vec::new(),
            reasoning_levels: codex_reasoning_levels(),
        },
        ExecutorType::ClaudeCode => DiscoveryData {
            models: Vec::new(),
            reasoning_levels: claude_reasoning_levels(),
        },
    }
}

fn resolve_claude_cli_path() -> Option<String> {
    SubprocessCliTransport::new(Prompt::Messages, ClaudeAgentOptions::default())
        .ok()
        .map(|transport| transport.cli_path.clone())
}

fn codex_reasoning_levels() -> Vec<String> {
    [
        ModelReasoningEffort::Minimal,
        ModelReasoningEffort::Low,
        ModelReasoningEffort::Medium,
        ModelReasoningEffort::High,
        ModelReasoningEffort::XHigh,
    ]
    .into_iter()
    .map(model_reasoning_effort_to_string)
    .collect()
}

fn model_reasoning_effort_to_string(level: ModelReasoningEffort) -> String {
    let value = match level {
        ModelReasoningEffort::Minimal => "minimal",
        ModelReasoningEffort::Low => "low",
        ModelReasoningEffort::Medium => "medium",
        ModelReasoningEffort::High => "high",
        ModelReasoningEffort::XHigh => "xhigh",
    };
    value.to_string()
}

fn claude_reasoning_levels() -> Vec<String> {
    ["low", "medium", "high", "max"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

async fn discover_from_commands(
    program: &str,
    command_candidates: &[&[&str]],
    parse_output: fn(&str) -> Vec<String>,
) -> Option<Vec<String>> {
    for args in command_candidates {
        if let Some(output) = run_discovery_command(program, args).await {
            let values = parse_output(&output);
            if !values.is_empty() {
                return Some(values);
            }
        }
    }

    None
}

async fn run_discovery_command(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command
        .kill_on_drop(true)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb");

    let output = timeout(DISCOVERY_COMMAND_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return None;
    }

    Some(stdout)
}

fn parse_models_output(output: &str) -> Vec<String> {
    let mut models = parse_json_list(
        output,
        &[
            "models",
            "data",
            "items",
            "available_models",
            "availableModels",
        ],
        &["id", "name", "model", "slug"],
    );

    if models.is_empty() {
        models = parse_plain_list(output);
    }

    dedupe_strings(models)
        .into_iter()
        .filter(|value| looks_like_model_name(value))
        .collect()
}

fn parse_reasoning_output(output: &str) -> Vec<String> {
    let mut values = parse_json_list(
        output,
        &[
            "reasoning",
            "reasoning_levels",
            "reasoningLevels",
            "effort",
            "efforts",
            "levels",
        ],
        &["reasoning", "effort", "level", "name", "id"],
    );

    if values.is_empty() {
        values = parse_plain_list(output);
    }

    dedupe_strings(
        values
            .into_iter()
            .filter_map(|value| normalize_reasoning_level(&value))
            .collect(),
    )
}

fn parse_json_list(output: &str, nested_keys: &[&str], field_keys: &[&str]) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return Vec::new();
    };

    let mut values = Vec::new();
    collect_json_strings(&value, nested_keys, field_keys, &mut values);
    values
}

fn collect_json_strings(
    value: &Value,
    nested_keys: &[&str],
    field_keys: &[&str],
    values: &mut Vec<String>,
) {
    match value {
        Value::String(text) => {
            values.push(text.to_string());
        }
        Value::Array(items) => {
            for item in items {
                collect_json_strings(item, nested_keys, field_keys, values);
            }
        }
        Value::Object(map) => {
            for key in field_keys {
                if let Some(text) = map.get(*key).and_then(Value::as_str) {
                    values.push(text.to_string());
                }
            }

            let mut matched_nested = false;
            for key in nested_keys {
                if let Some(nested_value) = map.get(*key) {
                    matched_nested = true;
                    collect_json_strings(nested_value, nested_keys, field_keys, values);
                }
            }

            if !matched_nested {
                for nested_value in map.values() {
                    if nested_value.is_array() || nested_value.is_object() {
                        collect_json_strings(nested_value, nested_keys, field_keys, values);
                    }
                }
            }
        }
        _ => {}
    }
}

fn parse_plain_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            if trimmed.chars().all(|ch| ch == '-' || ch == '=') {
                return None;
            }

            let cleaned = trimmed
                .trim_start_matches(['-', '*', '|'])
                .split_whitespace()
                .next()?
                .trim_matches(['|', ',', ';']);

            if cleaned.is_empty() {
                return None;
            }
            Some(cleaned.to_string())
        })
        .collect()
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for value in values {
        let normalized = value.trim().trim_matches(['"', '\'']).trim();
        if normalized.is_empty() {
            continue;
        }

        let key = normalized.to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(normalized.to_string());
        }
    }

    deduped
}

fn looks_like_model_name(value: &str) -> bool {
    let normalized = value.trim();
    if normalized.is_empty() {
        return false;
    }

    let lower = normalized.to_ascii_lowercase();
    if matches!(lower.as_str(), "model" | "models" | "name" | "id") {
        return false;
    }

    normalized.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn normalize_reasoning_level(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    let normalized = match normalized.as_str() {
        "x-high" | "extra-high" | "extra_high" => "xhigh".to_string(),
        _ => normalized,
    };

    if normalized.len() > 16 {
        return None;
    }
    if !normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch == '_' || ch == '-')
    {
        return None;
    }

    Some(normalized)
}

fn preferred_model(models: &[String]) -> Option<String> {
    models.first().cloned()
}

fn preferred_reasoning_level(levels: &[String]) -> Option<String> {
    for preferred in ["medium", "high", "low", "minimal", "max", "xhigh"] {
        if let Some(level) = levels.iter().find(|level| level.as_str() == preferred) {
            return Some(level.clone());
        }
    }
    levels.first().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{Duration, UNIX_EPOCH};

    struct TestConfigFile {
        path: PathBuf,
    }

    impl TestConfigFile {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "unified-agent-sdk-profile-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock should be monotonic")
                    .as_nanos()
            ));
            fs::create_dir_all(&base).expect("temp test dir should be created");
            Self {
                path: base.join("profiles.json"),
            }
        }

        fn write(&self, content: &str) {
            let mut file =
                fs::File::create(&self.path).expect("profile config file should be writable");
            file.write_all(content.as_bytes())
                .expect("profile config content should be written");
            file.sync_all().expect("profile config should be flushed");
        }
    }

    impl Drop for TestConfigFile {
        fn drop(&mut self) {
            if let Some(parent) = self.path.parent() {
                let _ = fs::remove_file(&self.path);
                let _ = fs::remove_dir(parent);
            }
        }
    }

    #[tokio::test]
    async fn load_merges_default_and_variant_profiles() {
        let fixture = TestConfigFile::new();
        fixture.write(
            r#"{
  "codex": {
    "default": { "model": "gpt-4", "permission_policy": "prompt" },
    "plan": { "reasoning": "high" }
  }
}"#,
        );

        let manager = ProfileManager::with_path(&fixture.path);
        let data = manager
            .load(&ProfileId::new(
                ExecutorType::Codex,
                Some("plan".to_string()),
            ))
            .await
            .expect("profile should load");

        assert_eq!(data.model.as_deref(), Some("gpt-4"));
        assert_eq!(data.reasoning.as_deref(), Some("high"));
        assert_eq!(data.permission_policy, Some(PermissionPolicy::Prompt));
    }

    #[tokio::test]
    async fn resolve_applies_runtime_overrides() {
        let fixture = TestConfigFile::new();
        fixture.write(
            r#"{
  "codex": {
    "default": { "model": "gpt-4", "reasoning": "medium" },
    "plan": { "reasoning": "high" }
  }
}"#,
        );

        let manager = ProfileManager::with_path(&fixture.path);
        let resolved = manager
            .resolve(&ExecutorConfig {
                profile_id: ProfileId::new(ExecutorType::Codex, Some("plan".to_string())),
                model_override: Some("gpt-5".to_string()),
                reasoning_override: None,
                permission_policy: Some(PermissionPolicy::Deny),
            })
            .await
            .expect("resolved config should be built");

        assert_eq!(resolved.model.as_deref(), Some("gpt-5"));
        assert_eq!(resolved.reasoning.as_deref(), Some("high"));
        assert_eq!(resolved.permission_policy, Some(PermissionPolicy::Deny));
    }

    #[tokio::test]
    async fn resolve_uses_discovered_reasoning_when_models_are_unavailable() {
        let fixture = TestConfigFile::new();
        fixture.write("{}");

        let manager = ProfileManager::with_path(&fixture.path);
        {
            let mut cache = manager.discovery_cache.write().await;
            cache.insert(
                ExecutorType::Codex,
                CachedDiscovery {
                    data: DiscoveryData {
                        models: Vec::new(),
                        reasoning_levels: vec!["high".to_string()],
                    },
                    expires_at: Instant::now() + Duration::from_secs(60),
                },
            );
        }

        let resolved = manager
            .resolve(&ExecutorConfig {
                profile_id: ProfileId::new(ExecutorType::Codex, None),
                model_override: None,
                reasoning_override: None,
                permission_policy: None,
            })
            .await
            .expect("resolved config should use discovery fallback values");

        assert_eq!(resolved.model, None);
        assert_eq!(resolved.reasoning.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn load_auto_reloads_when_profile_file_changes() {
        let fixture = TestConfigFile::new();
        fixture.write(
            r#"{
  "codex": {
    "default": { "model": "gpt-4" }
  }
}"#,
        );

        let manager = ProfileManager::with_path(&fixture.path);
        let id = ProfileId::new(ExecutorType::Codex, None);

        let first = manager.load(&id).await.expect("first load should succeed");
        assert_eq!(first.model.as_deref(), Some("gpt-4"));

        tokio::time::sleep(Duration::from_millis(1100)).await;
        fixture.write(
            r#"{
  "codex": {
    "default": { "model": "gpt-5" }
  }
}"#,
        );

        let second = manager
            .load(&id)
            .await
            .expect("reload after file update should work");
        assert_eq!(second.model.as_deref(), Some("gpt-5"));
    }
}
