//! Codex profile discovery.

use codex::ModelReasoningEffort;
use std::path::{Path, PathBuf};

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
    let codex_program = match resolve_cli_path() {
        Some(path) => path,
        None => {
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

fn resolve_cli_path() -> Option<String> {
    if let Ok(path) = which::which("codex") {
        return Some(path.to_string_lossy().into_owned());
    }

    let cwd = std::env::current_dir().ok();
    let home = home_dir();
    find_codex_path_from(cwd.as_deref(), home.as_deref())
}

fn find_codex_path_from(start_dir: Option<&Path>, home_dir: Option<&Path>) -> Option<String> {
    if let Some(start_dir) = start_dir {
        for dir in start_dir.ancestors() {
            let local_bin = dir
                .join("node_modules")
                .join(".bin")
                .join(codex_binary_name());
            if local_bin.is_file() {
                return Some(local_bin.to_string_lossy().into_owned());
            }

            if let Some(vendor_path) = local_vendor_binary_path(dir) {
                return Some(vendor_path.to_string_lossy().into_owned());
            }
        }
    }

    for path in common_global_locations(home_dir) {
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
    }

    None
}

fn local_vendor_binary_path(base_dir: &Path) -> Option<PathBuf> {
    let target_triple = platform_target_triple()?;
    let package = platform_package_for_target(target_triple)?;

    let candidate = base_dir
        .join("node_modules")
        .join(package)
        .join("vendor")
        .join(target_triple)
        .join("codex")
        .join(codex_binary_name());

    candidate.is_file().then_some(candidate)
}

fn common_global_locations(home_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut locations = Vec::new();
    if let Some(home) = home_dir {
        locations.push(
            home.join(".npm-global")
                .join("bin")
                .join(codex_binary_name()),
        );
        locations.push(home.join(".local").join("bin").join(codex_binary_name()));
        locations.push(
            home.join("node_modules")
                .join(".bin")
                .join(codex_binary_name()),
        );
        locations.push(home.join(".yarn").join("bin").join(codex_binary_name()));
        locations.push(home.join(".codex").join("local").join(codex_binary_name()));
    }
    locations.push(PathBuf::from("/usr/local/bin").join(codex_binary_name()));
    locations
}

fn codex_binary_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn platform_target_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("android", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("android", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        _ => None,
    }
}

fn platform_package_for_target(target_triple: &str) -> Option<&'static str> {
    match target_triple {
        "x86_64-unknown-linux-musl" => Some("@openai/codex-linux-x64"),
        "aarch64-unknown-linux-musl" => Some("@openai/codex-linux-arm64"),
        "x86_64-apple-darwin" => Some("@openai/codex-darwin-x64"),
        "aarch64-apple-darwin" => Some("@openai/codex-darwin-arm64"),
        "x86_64-pc-windows-msvc" => Some("@openai/codex-windows-x64"),
        "aarch64-pc-windows-msvc" => Some("@openai/codex-windows-arm64"),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "unified-agent-sdk-codex-profile-test-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock should be monotonic")
                    .as_nanos(),
                seq
            ));
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }

        fn touch(&self, relative_path: &Path) -> PathBuf {
            let full_path = self.path.join(relative_path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).expect("parent dirs should be created");
            }
            fs::write(&full_path, b"#!/bin/sh\n").expect("fixture file should be written");
            full_path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn finds_codex_in_ancestor_node_modules_bin() {
        let fixture = TestDir::new();
        let nested_dir = fixture.path.join("workspace").join("crate");
        fs::create_dir_all(&nested_dir).expect("nested workdir should exist");
        let expected = fixture.touch(
            Path::new("node_modules/.bin")
                .join(codex_binary_name())
                .as_path(),
        );

        let found = find_codex_path_from(Some(&nested_dir), None).expect("path should be found");
        assert_eq!(PathBuf::from(found), expected);
    }

    #[test]
    fn finds_codex_in_home_global_locations() {
        let fixture = TestDir::new();
        let expected = fixture.touch(
            Path::new(".npm-global/bin")
                .join(codex_binary_name())
                .as_path(),
        );

        let found = find_codex_path_from(None, Some(&fixture.path)).expect("path should be found");
        assert_eq!(PathBuf::from(found), expected);
    }
}
