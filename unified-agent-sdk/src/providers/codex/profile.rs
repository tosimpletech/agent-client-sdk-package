//! Codex profile discovery.

use codex::ModelReasoningEffort;

use crate::profile::DiscoveryData;

const MODEL_COMMANDS: &[&[&str]] = &[
    &["models", "list", "--json"],
    &["models", "--json"],
    &["model", "list", "--json"],
    &["models", "list"],
];

const REASONING_COMMANDS: &[&[&str]] = &[
    &["reasoning", "list", "--json"],
    &["reasoning", "--json"],
    &["models", "reasoning", "--json"],
    &["reasoning", "list"],
];

/// Discovers Codex models and reasoning levels from the Codex CLI.
///
/// Discovery is best-effort and gracefully degrades when the CLI is missing or does
/// not expose structured output. In degraded mode, model list is empty and reasoning
/// falls back to known defaults.
pub async fn discover() -> DiscoveryData {
    let codex_program = match which::which("codex") {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => {
            return DiscoveryData {
                models: Vec::new(),
                reasoning_levels: default_reasoning_levels(),
            };
        }
    };

    let models = crate::profile::discover_from_commands(
        &codex_program,
        MODEL_COMMANDS,
        crate::profile::parse_models_output,
    )
    .await
    .unwrap_or_default();

    let reasoning_levels = crate::profile::discover_from_commands(
        &codex_program,
        REASONING_COMMANDS,
        crate::profile::parse_reasoning_output,
    )
    .await
    .filter(|levels| !levels.is_empty())
    .unwrap_or_else(default_reasoning_levels);

    DiscoveryData {
        models,
        reasoning_levels,
    }
}

fn default_reasoning_levels() -> Vec<String> {
    [
        ModelReasoningEffort::Minimal,
        ModelReasoningEffort::Low,
        ModelReasoningEffort::Medium,
        ModelReasoningEffort::High,
        ModelReasoningEffort::XHigh,
    ]
    .into_iter()
    .map(|level| match level {
        ModelReasoningEffort::Minimal => "minimal",
        ModelReasoningEffort::Low => "low",
        ModelReasoningEffort::Medium => "medium",
        ModelReasoningEffort::High => "high",
        ModelReasoningEffort::XHigh => "xhigh",
    })
    .map(str::to_string)
    .collect()
}
