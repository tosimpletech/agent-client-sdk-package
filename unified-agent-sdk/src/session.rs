//! Session management

use chrono::{DateTime, Utc};
use std::path::PathBuf;

use crate::{
    error::Result,
    types::{ExecutorType, ExitStatus},
};

/// Session metadata for persistence
#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub session_id: String,
    pub executor_type: ExecutorType,
    pub created_at: DateTime<Utc>,
    pub last_message_id: Option<String>,
    pub working_dir: PathBuf,
}

/// Session resume information
#[derive(Debug, Clone)]
pub struct SessionResume {
    pub session_id: String,
    pub reset_to_message: Option<String>,
}

/// Active agent session
pub struct AgentSession {
    pub session_id: String,
    pub executor_type: ExecutorType,
    pub working_dir: PathBuf,
    // Internal process handle (implementation-specific)
}

impl AgentSession {
    pub fn metadata(&self) -> SessionMetadata {
        SessionMetadata {
            session_id: self.session_id.clone(),
            executor_type: self.executor_type,
            created_at: Utc::now(),
            last_message_id: None,
            working_dir: self.working_dir.clone(),
        }
    }

    pub async fn wait(&mut self) -> Result<ExitStatus> {
        // TODO: implement wait
        Ok(ExitStatus {
            code: Some(0),
            success: true,
        })
    }

    pub async fn cancel(&mut self) -> Result<()> {
        // TODO: implement cancel
        Ok(())
    }
}
