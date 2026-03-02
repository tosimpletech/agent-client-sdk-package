use claude_code::{ClaudeAgentOptions, PermissionMode, SettingSource};
use serde_json::json;

#[test]
fn test_setting_source_serialization() {
    let sources = vec![
        SettingSource::User,
        SettingSource::Project,
        SettingSource::Local,
    ];

    let serialized = serde_json::to_value(&sources).expect("serialize");
    assert_eq!(serialized, json!(["user", "project", "local"]));
}

#[test]
fn test_setting_source_deserialization() {
    let json = json!(["user", "project"]);
    let sources: Vec<SettingSource> = serde_json::from_value(json).expect("deserialize");
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0], SettingSource::User);
    assert_eq!(sources[1], SettingSource::Project);
}

#[test]
fn test_options_default_has_no_setting_sources() {
    let options = ClaudeAgentOptions::default();
    assert!(options.setting_sources.is_none());
}

#[test]
fn test_options_with_setting_sources() {
    let options = ClaudeAgentOptions {
        setting_sources: Some(vec![SettingSource::User, SettingSource::Project]),
        ..Default::default()
    };

    let sources = options.setting_sources.as_ref().expect("has sources");
    assert_eq!(sources.len(), 2);
    assert!(sources.contains(&SettingSource::User));
    assert!(sources.contains(&SettingSource::Project));
}

#[test]
fn test_permission_mode_serialization_roundtrip() {
    let modes = vec![
        (PermissionMode::Default, "default"),
        (PermissionMode::AcceptEdits, "acceptEdits"),
        (PermissionMode::Plan, "plan"),
        (PermissionMode::BypassPermissions, "bypassPermissions"),
    ];

    for (mode, expected_str) in modes {
        let serialized = serde_json::to_value(&mode).expect("serialize");
        assert_eq!(serialized, json!(expected_str));

        let deserialized: PermissionMode = serde_json::from_value(serialized).expect("deserialize");
        assert_eq!(deserialized, mode);
    }
}

#[test]
fn test_permission_mode_in_options() {
    let options = ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::BypassPermissions),
        ..Default::default()
    };

    assert_eq!(
        options.permission_mode,
        Some(PermissionMode::BypassPermissions)
    );
}
