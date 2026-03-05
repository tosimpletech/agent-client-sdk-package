//! Session history helpers aligned with the Python SDK.
//!
//! This module scans Claude session transcript files under
//! `~/.claude/projects/` (or `CLAUDE_CONFIG_DIR`) and exposes:
//!
//! - [`list_sessions`] for lightweight session metadata
//! - [`get_session_messages`] for reconstructed conversation messages

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::types::{SDKSessionInfo, SessionMessage};

const MAX_SANITIZED_LENGTH: usize = 200;

#[derive(Debug, Clone)]
struct TranscriptEntry {
    entry_type: String,
    uuid: String,
    parent_uuid: Option<String>,
    session_id: Option<String>,
    message: Option<Value>,
    is_sidechain: bool,
    is_meta: bool,
    team_name: Option<String>,
}

fn validate_uuid(maybe_uuid: &str) -> bool {
    if maybe_uuid.len() != 36 {
        return false;
    }
    for (i, ch) in maybe_uuid.chars().enumerate() {
        let is_dash = matches!(i, 8 | 13 | 18 | 23);
        if is_dash {
            if ch != '-' {
                return false;
            }
        } else if !ch.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn simple_hash(input: &str) -> String {
    let mut hash: i32 = 0;
    for ch in input.chars() {
        hash = hash
            .wrapping_shl(5)
            .wrapping_sub(hash)
            .wrapping_add(ch as i32);
    }
    let mut value = hash.unsigned_abs() as u64;
    if value == 0 {
        return "0".to_string();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while value > 0 {
        out.push(digits[(value % 36) as usize] as char);
        value /= 36;
    }
    out.iter().rev().collect()
}

fn sanitize_path(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.len() <= MAX_SANITIZED_LENGTH {
        return out;
    }
    format!("{}-{}", &out[..MAX_SANITIZED_LENGTH], simple_hash(name))
}

fn claude_config_home_dir() -> PathBuf {
    if let Ok(path) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".claude");
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        return PathBuf::from(home).join(".claude");
    }
    PathBuf::from(".claude")
}

fn projects_dir() -> PathBuf {
    claude_config_home_dir().join("projects")
}

fn canonicalize_dir(directory: &str) -> String {
    fs::canonicalize(directory)
        .unwrap_or_else(|_| PathBuf::from(directory))
        .to_string_lossy()
        .to_string()
}

fn find_project_dir(project_path: &str) -> Option<PathBuf> {
    let sanitized = sanitize_path(project_path);
    let exact = projects_dir().join(&sanitized);
    if exact.is_dir() {
        return Some(exact);
    }

    if sanitized.len() <= MAX_SANITIZED_LENGTH {
        return None;
    }

    let prefix = &sanitized[..MAX_SANITIZED_LENGTH];
    let entries = fs::read_dir(projects_dir()).ok()?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str()
            && name.starts_with(&(prefix.to_string() + "-"))
        {
            return Some(entry.path());
        }
    }
    None
}

fn parse_jsonl(content: &str) -> Vec<Value> {
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn extract_command_name(text: &str) -> Option<String> {
    let start_tag = "<command-name>";
    let end_tag = "</command-name>";
    let start = text.find(start_tag)?;
    let after_start = start + start_tag.len();
    let end = text[after_start..].find(end_tag)?;
    Some(text[after_start..after_start + end].trim().to_string())
}

fn is_skipped_first_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed.starts_with("<local-command-stdout>")
        || trimmed.starts_with("<session-start-hook>")
        || trimmed.starts_with("<tick>")
        || trimmed.starts_with("<goal>")
        || trimmed.starts_with("[Request interrupted by user")
        || (trimmed.starts_with("<ide_opened_file>") && trimmed.ends_with("</ide_opened_file>"))
        || (trimmed.starts_with("<ide_selection>") && trimmed.ends_with("</ide_selection>"))
}

fn extract_first_prompt(entries: &[Value]) -> Option<String> {
    let mut command_fallback: Option<String> = None;

    for entry in entries {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        if obj.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        if obj
            .get("isCompactSummary")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }

        let Some(message) = obj.get("message").and_then(Value::as_object) else {
            continue;
        };
        let Some(content) = message.get("content") else {
            continue;
        };

        let texts: Vec<String> = if let Some(text) = content.as_str() {
            vec![text.to_string()]
        } else if let Some(blocks) = content.as_array() {
            blocks
                .iter()
                .filter_map(|block| {
                    let block_obj = block.as_object()?;
                    if block_obj.get("type").and_then(Value::as_str) == Some("text") {
                        block_obj
                            .get("text")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        for raw in texts {
            let candidate = raw.replace('\n', " ").trim().to_string();
            if candidate.is_empty() {
                continue;
            }

            if let Some(name) = extract_command_name(&candidate) {
                if command_fallback.is_none() && !name.is_empty() {
                    command_fallback = Some(name);
                }
                continue;
            }

            if is_skipped_first_prompt(&candidate) {
                continue;
            }

            if candidate.len() > 200 {
                return Some(format!("{}...", candidate[..200].trim_end()));
            }
            return Some(candidate);
        }
    }

    command_fallback
}

fn extract_last_string_field(entries: &[Value], key: &str) -> Option<String> {
    entries.iter().rev().find_map(|entry| {
        entry
            .as_object()?
            .get(key)?
            .as_str()
            .map(ToString::to_string)
    })
}

fn extract_first_string_field(entries: &[Value], key: &str) -> Option<String> {
    entries.iter().find_map(|entry| {
        entry
            .as_object()?
            .get(key)?
            .as_str()
            .map(ToString::to_string)
    })
}

fn millis_since_epoch(modified: std::time::SystemTime) -> i64 {
    modified
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn read_sessions_from_dir(project_dir: &Path, project_path: Option<&str>) -> Vec<SDKSessionInfo> {
    let mut results = Vec::new();
    let entries = match fs::read_dir(project_dir) {
        Ok(entries) => entries,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !validate_uuid(stem) {
            continue;
        }

        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        if content.trim().is_empty() {
            continue;
        }

        let first_line = content.lines().next().unwrap_or_default();
        if first_line.contains("\"isSidechain\":true")
            || first_line.contains("\"isSidechain\": true")
        {
            continue;
        }

        let entries = parse_jsonl(&content);
        let custom_title = extract_last_string_field(&entries, "customTitle");
        let first_prompt = extract_first_prompt(&entries);
        let summary = custom_title
            .clone()
            .or_else(|| extract_last_string_field(&entries, "summary"))
            .or_else(|| first_prompt.clone());
        let Some(summary) = summary else {
            continue;
        };

        let git_branch = extract_last_string_field(&entries, "gitBranch")
            .or_else(|| extract_first_string_field(&entries, "gitBranch"));
        let cwd = extract_first_string_field(&entries, "cwd")
            .or_else(|| project_path.map(ToString::to_string));

        results.push(SDKSessionInfo {
            session_id: stem.to_string(),
            summary,
            last_modified: metadata
                .modified()
                .map(millis_since_epoch)
                .unwrap_or_default(),
            file_size: metadata.len(),
            custom_title,
            first_prompt,
            git_branch,
            cwd,
        });
    }

    results
}

fn get_worktree_paths(cwd: &str) -> Vec<String> {
    let output = match Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(cwd)
        .output()
    {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree ").map(ToString::to_string))
        .collect()
}

fn deduplicate_and_sort(
    mut sessions: Vec<SDKSessionInfo>,
    limit: Option<usize>,
) -> Vec<SDKSessionInfo> {
    let mut by_id: HashMap<String, SDKSessionInfo> = HashMap::new();
    for session in sessions.drain(..) {
        let replace = by_id
            .get(&session.session_id)
            .map(|existing| session.last_modified > existing.last_modified)
            .unwrap_or(true);
        if replace {
            by_id.insert(session.session_id.clone(), session);
        }
    }

    let mut values: Vec<SDKSessionInfo> = by_id.into_values().collect();
    values.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    if let Some(limit) = limit
        && limit > 0
    {
        values.truncate(limit);
    }
    values
}

fn list_sessions_for_project(
    directory: &str,
    limit: Option<usize>,
    include_worktrees: bool,
) -> Vec<SDKSessionInfo> {
    let canonical = canonicalize_dir(directory);
    let mut candidates = vec![canonical.clone()];
    if include_worktrees {
        for path in get_worktree_paths(&canonical) {
            if !candidates.iter().any(|candidate| candidate == &path) {
                candidates.push(path);
            }
        }
    }

    let mut all = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidates {
        let Some(project_dir) = find_project_dir(&candidate) else {
            continue;
        };
        let key = project_dir.to_string_lossy().to_string();
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        all.extend(read_sessions_from_dir(&project_dir, Some(&candidate)));
    }
    deduplicate_and_sort(all, limit)
}

/// Lists session metadata from Claude session transcript files.
pub fn list_sessions(
    directory: Option<&str>,
    limit: Option<usize>,
    include_worktrees: bool,
) -> Vec<SDKSessionInfo> {
    if let Some(directory) = directory {
        return list_sessions_for_project(directory, limit, include_worktrees);
    }

    let mut all = Vec::new();
    let entries = match fs::read_dir(projects_dir()) {
        Ok(entries) => entries,
        Err(_) => return all,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            all.extend(read_sessions_from_dir(&path, None));
        }
    }

    deduplicate_and_sort(all, limit)
}

fn parse_transcript_entries(content: &str) -> Vec<TranscriptEntry> {
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|entry| {
            let obj = entry.as_object()?;
            let entry_type = obj.get("type")?.as_str()?.to_string();
            if !matches!(
                entry_type.as_str(),
                "user" | "assistant" | "progress" | "system" | "attachment"
            ) {
                return None;
            }
            let uuid = obj.get("uuid")?.as_str()?.to_string();
            Some(TranscriptEntry {
                entry_type,
                uuid,
                parent_uuid: obj
                    .get("parentUuid")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                session_id: obj
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                message: obj.get("message").cloned(),
                is_sidechain: obj
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                is_meta: obj.get("isMeta").and_then(Value::as_bool).unwrap_or(false),
                team_name: obj
                    .get("teamName")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })
        })
        .collect()
}

fn build_conversation_chain(entries: &[TranscriptEntry]) -> Vec<TranscriptEntry> {
    if entries.is_empty() {
        return Vec::new();
    }

    let mut by_uuid = HashMap::new();
    let mut entry_index = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        by_uuid.insert(entry.uuid.clone(), entry.clone());
        entry_index.insert(entry.uuid.clone(), index);
    }

    let parent_uuids: HashSet<String> = entries
        .iter()
        .filter_map(|entry| entry.parent_uuid.clone())
        .collect();
    let terminals: Vec<TranscriptEntry> = entries
        .iter()
        .filter(|entry| !parent_uuids.contains(&entry.uuid))
        .cloned()
        .collect();

    let mut leaves = Vec::new();
    for terminal in terminals {
        let mut current = Some(terminal);
        let mut seen = HashSet::new();
        while let Some(entry) = current {
            if !seen.insert(entry.uuid.clone()) {
                break;
            }
            if matches!(entry.entry_type.as_str(), "user" | "assistant") {
                leaves.push(entry);
                break;
            }
            current = entry
                .parent_uuid
                .as_ref()
                .and_then(|uuid| by_uuid.get(uuid))
                .cloned();
        }
    }

    if leaves.is_empty() {
        return Vec::new();
    }

    let main_leaves: Vec<TranscriptEntry> = leaves
        .iter()
        .filter(|leaf| !leaf.is_sidechain && !leaf.is_meta && leaf.team_name.is_none())
        .cloned()
        .collect();

    let source = if main_leaves.is_empty() {
        &leaves
    } else {
        &main_leaves
    };
    let leaf = source
        .iter()
        .max_by_key(|entry| entry_index.get(&entry.uuid).copied().unwrap_or(0))
        .cloned();

    let Some(mut current) = leaf else {
        return Vec::new();
    };

    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(current.uuid.clone()) {
            break;
        }
        chain.push(current.clone());
        let Some(parent_uuid) = current.parent_uuid.clone() else {
            break;
        };
        let Some(parent) = by_uuid.get(&parent_uuid).cloned() else {
            break;
        };
        current = parent;
    }

    chain.reverse();
    chain
}

fn is_visible_message(entry: &TranscriptEntry) -> bool {
    matches!(entry.entry_type.as_str(), "user" | "assistant")
        && !entry.is_meta
        && !entry.is_sidechain
        && entry.team_name.is_none()
}

fn read_session_file(session_id: &str, directory: Option<&str>) -> Option<String> {
    let file_name = format!("{session_id}.jsonl");

    if let Some(directory) = directory {
        let canonical = canonicalize_dir(directory);
        let mut candidates = vec![canonical.clone()];
        for path in get_worktree_paths(&canonical) {
            if !candidates.iter().any(|candidate| candidate == &path) {
                candidates.push(path);
            }
        }

        for candidate in candidates {
            let Some(project_dir) = find_project_dir(&candidate) else {
                continue;
            };
            let path = project_dir.join(&file_name);
            if let Ok(content) = fs::read_to_string(path) {
                return Some(content);
            }
        }
        return None;
    }

    let projects = fs::read_dir(projects_dir()).ok()?;
    for project in projects.flatten() {
        let path = project.path().join(&file_name);
        if let Ok(content) = fs::read_to_string(path) {
            return Some(content);
        }
    }
    None
}

/// Returns chronological user/assistant messages for a saved Claude session.
pub fn get_session_messages(
    session_id: &str,
    directory: Option<&str>,
    limit: Option<usize>,
    offset: usize,
) -> Vec<SessionMessage> {
    if !validate_uuid(session_id) {
        return Vec::new();
    }

    let Some(content) = read_session_file(session_id, directory) else {
        return Vec::new();
    };
    let entries = parse_transcript_entries(&content);
    let chain = build_conversation_chain(&entries);
    let mut messages: Vec<SessionMessage> = chain
        .into_iter()
        .filter(is_visible_message)
        .map(|entry| SessionMessage {
            type_: entry.entry_type,
            uuid: entry.uuid,
            session_id: entry.session_id.unwrap_or_default(),
            message: entry.message.unwrap_or(Value::Null),
            parent_tool_use_id: None,
        })
        .collect();

    if offset > 0 {
        if offset >= messages.len() {
            return Vec::new();
        }
        messages = messages.split_off(offset);
    }

    if let Some(limit) = limit
        && limit > 0
        && messages.len() > limit
    {
        messages.truncate(limit);
    }

    messages
}
