use std::collections::HashMap;

use claude_code::AgentDefinition;
use serde_json::json;

#[test]
fn test_agent_definition_serialization_full() {
    let agent = AgentDefinition {
        description: "A specialized code reviewer".to_string(),
        prompt: "Review the code for bugs and best practices".to_string(),
        tools: Some(vec!["Read".to_string(), "Grep".to_string()]),
        model: Some("sonnet".to_string()),
    };

    let serialized = serde_json::to_value(&agent).expect("serialize");
    assert_eq!(serialized["description"], "A specialized code reviewer");
    assert_eq!(
        serialized["prompt"],
        "Review the code for bugs and best practices"
    );
    assert_eq!(serialized["tools"], json!(["Read", "Grep"]));
    assert_eq!(serialized["model"], "sonnet");
}

#[test]
fn test_agent_definition_serialization_minimal() {
    let agent = AgentDefinition {
        description: "A helper agent".to_string(),
        prompt: "Help with tasks".to_string(),
        tools: None,
        model: None,
    };

    let serialized = serde_json::to_value(&agent).expect("serialize");
    assert_eq!(serialized["description"], "A helper agent");
    assert_eq!(serialized["prompt"], "Help with tasks");
    // Optional fields should be absent (skip_serializing_if).
    assert!(serialized.get("tools").is_none());
    assert!(serialized.get("model").is_none());
}

#[test]
fn test_agent_definition_deserialization() {
    let json = json!({
        "description": "A test agent",
        "prompt": "Do testing",
        "tools": ["Bash", "Write"],
        "model": "haiku"
    });

    let agent: AgentDefinition = serde_json::from_value(json).expect("deserialize");
    assert_eq!(agent.description, "A test agent");
    assert_eq!(agent.prompt, "Do testing");
    assert_eq!(agent.tools.as_deref(), Some(&["Bash".to_string(), "Write".to_string()][..]));
    assert_eq!(agent.model.as_deref(), Some("haiku"));
}

#[test]
fn test_agent_definitions_map_serialization() {
    let mut agents = HashMap::new();
    agents.insert(
        "code_reviewer".to_string(),
        AgentDefinition {
            description: "Reviews code".to_string(),
            prompt: "Review carefully".to_string(),
            tools: Some(vec!["Read".to_string()]),
            model: None,
        },
    );
    agents.insert(
        "test_writer".to_string(),
        AgentDefinition {
            description: "Writes tests".to_string(),
            prompt: "Write comprehensive tests".to_string(),
            tools: None,
            model: Some("opus".to_string()),
        },
    );

    let serialized = serde_json::to_value(&agents).expect("serialize");
    assert!(serialized["code_reviewer"]["description"]
        .as_str()
        .unwrap()
        .contains("Reviews code"));
    assert!(serialized["test_writer"]["model"]
        .as_str()
        .unwrap()
        .contains("opus"));

    // Round-trip deserialization.
    let deserialized: HashMap<String, AgentDefinition> =
        serde_json::from_value(serialized).expect("deserialize");
    assert_eq!(deserialized.len(), 2);
    assert_eq!(deserialized["code_reviewer"].description, "Reviews code");
    assert_eq!(deserialized["test_writer"].model, Some("opus".to_string()));
}
