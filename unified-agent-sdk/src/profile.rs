//! Profile and configuration management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

use crate::{
    error::Result,
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
    cache: RwLock<HashMap<ProfileId, ProfileData>>,
}

#[derive(Debug, Clone)]
pub struct ProfileData {
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub permission_policy: Option<PermissionPolicy>,
}

impl ProfileManager {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn load(&self, id: &ProfileId) -> Result<ProfileData> {
        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(data) = cache.get(id) {
                return Ok(data.clone());
            }
        }

        // TODO: Load from file system
        let data = ProfileData {
            model: None,
            reasoning: None,
            permission_policy: None,
        };

        // Cache it
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(id.clone(), data.clone());
        }

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
}

impl Default for ProfileManager {
    fn default() -> Self {
        Self::new()
    }
}
