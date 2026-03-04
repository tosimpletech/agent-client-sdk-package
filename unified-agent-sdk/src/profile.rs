//! Profile and configuration management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

use crate::{
    error::{ExecutorError, Result},
    types::{ExecutorType, PermissionPolicy},
};

/// Profile identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId {
    pub executor: ExecutorType,
    pub variant: Option<String>,
}

impl ProfileId {
    pub fn new(executor: ExecutorType, variant: Option<String>) -> Self {
        Self { executor, variant }
    }
}

/// Executor configuration with overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorConfig {
    pub profile_id: ProfileId,
    pub model_override: Option<String>,
    pub reasoning_override: Option<String>,
    pub permission_policy: Option<PermissionPolicy>,
}

/// Resolved configuration
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub permission_policy: Option<PermissionPolicy>,
}

/// Profile manager
pub struct ProfileManager {
    config_path: PathBuf,
    cache: RwLock<ProfileCache>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileData {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
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
    pub fn new() -> Self {
        Self::with_path(default_profile_path())
    }

    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: path.into(),
            cache: RwLock::new(ProfileCache::default()),
        }
    }

    pub fn load(&self, id: &ProfileId) -> Result<ProfileData> {
        self.reload_if_needed(false)?;

        let mut cache = self
            .cache
            .write()
            .map_err(|_| ExecutorError::Other("profile cache lock poisoned".to_string()))?;

        if let Some(data) = cache.resolved.get(id) {
            return Ok(data.clone());
        }

        let data = resolve_profile_data(&cache.profiles, id);
        cache.resolved.insert(id.clone(), data.clone());
        Ok(data)
    }

    pub fn resolve(&self, config: &ExecutorConfig) -> Result<ResolvedConfig> {
        let profile = self.load(&config.profile_id)?;

        Ok(ResolvedConfig {
            model: config.model_override.clone().or(profile.model),
            reasoning: config.reasoning_override.clone().or(profile.reasoning),
            permission_policy: config.permission_policy.or(profile.permission_policy),
        })
    }

    /// Force reload profiles from disk and clear resolved cache.
    pub fn reload(&self) -> Result<()> {
        self.reload_if_needed(true)
    }

    fn reload_if_needed(&self, force: bool) -> Result<()> {
        let disk_mtime = read_modified_time(&self.config_path)?;

        let mut cache = self
            .cache
            .write()
            .map_err(|_| ExecutorError::Other("profile cache lock poisoned".to_string()))?;

        if !force && cache.loaded && cache.file_mtime == disk_mtime {
            return Ok(());
        }

        cache.profiles = load_profiles_from_disk(&self.config_path)?;
        cache.resolved.clear();
        cache.file_mtime = disk_mtime;
        cache.loaded = true;
        Ok(())
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
        ExecutorError::InvalidConfig(format!(
            "failed to parse profile config at {}: {err}",
            path.display()
        ))
    })?;

    let mut profiles: ProfilesFile = HashMap::new();
    for (executor, variants) in parsed {
        let executor_key = normalize_executor_key(&executor);
        let profile_map = profiles.entry(executor_key).or_default();

        for (variant, data) in variants {
            let permission_policy = parse_permission_policy(data.permission_policy.as_deref())
                .map_err(|err| {
                    ExecutorError::InvalidConfig(format!(
                        "invalid permission_policy for profile {}.{}: {err}",
                        executor, variant
                    ))
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
            return Err(ExecutorError::InvalidConfig(format!(
                "unsupported permission_policy value: {raw}"
            )));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::thread;
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

    #[test]
    fn load_merges_default_and_variant_profiles() {
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
            .expect("profile should load");

        assert_eq!(data.model.as_deref(), Some("gpt-4"));
        assert_eq!(data.reasoning.as_deref(), Some("high"));
        assert_eq!(data.permission_policy, Some(PermissionPolicy::Prompt));
    }

    #[test]
    fn resolve_applies_runtime_overrides() {
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
            .expect("resolved config should be built");

        assert_eq!(resolved.model.as_deref(), Some("gpt-5"));
        assert_eq!(resolved.reasoning.as_deref(), Some("high"));
        assert_eq!(resolved.permission_policy, Some(PermissionPolicy::Deny));
    }

    #[test]
    fn load_auto_reloads_when_profile_file_changes() {
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

        let first = manager.load(&id).expect("first load should succeed");
        assert_eq!(first.model.as_deref(), Some("gpt-4"));

        thread::sleep(Duration::from_millis(1100));
        fixture.write(
            r#"{
  "codex": {
    "default": { "model": "gpt-5" }
  }
}"#,
        );

        let second = manager
            .load(&id)
            .expect("reload after file update should work");
        assert_eq!(second.model.as_deref(), Some("gpt-5"));
    }
}
