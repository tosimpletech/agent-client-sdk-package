//! Claude Code profile discovery.

use claude_code::{ClaudeAgentOptions, Prompt, SubprocessCliTransport};

use crate::profile::DiscoveryData;

const MODEL_COMMANDS: &[&[&str]] = &[
    &["models", "list", "--json"],
    &["models", "--json"],
    &["model", "list", "--json"],
    &["models", "list"],
];

const REASONING_COMMANDS: &[&[&str]] = &[
    &["effort", "list", "--json"],
    &["reasoning", "list", "--json"],
    &["models", "reasoning", "--json"],
    &["effort", "list"],
];

pub async fn discover() -> DiscoveryData {
    let claude_program = match resolve_cli_path() {
        Some(path) => path,
        None => {
            return DiscoveryData {
                models: Vec::new(),
                reasoning_levels: default_reasoning_levels(),
            }
        }
    };

    let models = crate::profile::discover_from_commands(
        &claude_program,
        MODEL_COMMANDS,
        crate::profile::parse_models_output,
    )
    .await
    .unwrap_or_default();

    let reasoning_levels = crate::profile::discover_from_commands(
        &claude_program,
        REASONING_COMMANDS,
        crate::profile::parse_reasoning_output,
    )
    .await
    .unwrap_or_else(default_reasoning_levels);

    DiscoveryData {
        models,
        reasoning_levels,
    }
}

fn resolve_cli_path() -> Option<String> {
    SubprocessCliTransport::new(Prompt::Messages, ClaudeAgentOptions::default())
        .ok()
        .map(|transport| transport.cli_path.clone())
}

fn default_reasoning_levels() -> Vec<String> {
    ["low", "medium", "high", "max"]
        .into_iter()
        .map(str::to_string)
        .collect()
}
