use crate::codex_options::CodexOptions;
use crate::errors::Result;
use crate::exec::CodexExec;
use crate::thread::Thread;
use crate::thread_options::ThreadOptions;

/// Entry point for interacting with the Codex agent.
#[derive(Debug, Clone)]
pub struct Codex {
    exec: CodexExec,
    options: CodexOptions,
}

impl Codex {
    /// Creates a new Codex client.
    ///
    /// When `options` is `None`, default options are used and the SDK attempts
    /// to discover the `codex` executable automatically.
    pub fn new(options: Option<CodexOptions>) -> Result<Self> {
        let options = options.unwrap_or_default();
        let exec = CodexExec::new(
            options.codex_path_override.clone(),
            options.env.clone(),
            options.config.clone(),
        )?;
        Ok(Self { exec, options })
    }

    /// Starts a new thread.
    pub fn start_thread(&self, options: Option<ThreadOptions>) -> Thread {
        Thread::new(
            self.exec.clone(),
            self.options.clone(),
            options.unwrap_or_default(),
            None,
        )
    }

    /// Resumes an existing thread by id.
    pub fn resume_thread(&self, id: impl Into<String>, options: Option<ThreadOptions>) -> Thread {
        Thread::new(
            self.exec.clone(),
            self.options.clone(),
            options.unwrap_or_default(),
            Some(id.into()),
        )
    }
}
