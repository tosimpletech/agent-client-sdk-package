use claude_code::{
    CLIConnectionError, CLIJSONDecodeError, CLINotFoundError, ClaudeSDKError, ProcessError,
};

#[test]
fn test_base_error() {
    let error = ClaudeSDKError::new("Something went wrong");
    assert_eq!(error.to_string(), "Something went wrong");
}

#[test]
fn test_cli_not_found_error() {
    let error = CLINotFoundError::new("Claude Code not found", None);
    assert!(error.to_string().contains("Claude Code not found"));
}

#[test]
fn test_connection_error() {
    let error = CLIConnectionError::new("Failed to connect to CLI");
    assert!(error.to_string().contains("Failed to connect to CLI"));
}

#[test]
fn test_process_error() {
    let error = ProcessError::new(
        "Process failed",
        Some(1),
        Some("Command not found".to_string()),
    );
    assert_eq!(error.exit_code, Some(1));
    assert_eq!(error.stderr.as_deref(), Some("Command not found"));
    assert!(error.to_string().contains("Process failed"));
    assert!(error.to_string().contains("exit code: 1"));
    assert!(error.to_string().contains("Command not found"));
}

#[test]
fn test_json_decode_error() {
    let error = CLIJSONDecodeError::new("{invalid json}", "expected value");
    assert_eq!(error.line, "{invalid json}");
    assert!(error.to_string().contains("Failed to decode JSON"));
}
