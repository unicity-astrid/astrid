use super::*;
use crate::action::SensitiveAction;
use astrid_core::types::Permission;

// ---------------------------------------------------------------------------
// AllowancePattern display tests
// ---------------------------------------------------------------------------

#[test]
fn test_allowance_pattern_display() {
    let pattern = AllowancePattern::ExactTool {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    assert_eq!(pattern.to_string(), "mcp://filesystem/read_file");

    let pattern = AllowancePattern::ServerTools {
        server: "github".to_string(),
    };
    assert_eq!(pattern.to_string(), "mcp://github/*");

    let pattern = AllowancePattern::NetworkHost {
        host: "api.example.com".to_string(),
        ports: Some(vec![443, 8080]),
    };
    assert_eq!(pattern.to_string(), "net:api.example.com:[443,8080]");

    let pattern = AllowancePattern::NetworkHost {
        host: "api.example.com".to_string(),
        ports: None,
    };
    assert_eq!(pattern.to_string(), "net:api.example.com:*");
}

#[test]
fn test_allowance_pattern_serialization() {
    let pattern = AllowancePattern::FilePattern {
        pattern: "/home/user/docs/*".to_string(),
        permission: Permission::Read,
    };
    let json = serde_json::to_string(&pattern).unwrap();
    let deserialized: AllowancePattern = serde_json::from_str(&json).unwrap();
    assert_eq!(pattern.to_string(), deserialized.to_string());
}

// ---------------------------------------------------------------------------
// ExactTool matching tests
// ---------------------------------------------------------------------------

#[test]
fn test_exact_tool_matches() {
    let pattern = AllowancePattern::ExactTool {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    assert!(pattern.matches(&action, None));
}

#[test]
fn test_exact_tool_wrong_tool() {
    let pattern = AllowancePattern::ExactTool {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    let action = SensitiveAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "write_file".to_string(),
    };
    assert!(!pattern.matches(&action, None));
}

#[test]
fn test_exact_tool_wrong_action_type() {
    let pattern = AllowancePattern::ExactTool {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
    };
    let action = SensitiveAction::FileDelete {
        path: "/tmp/test".to_string(),
    };
    assert!(!pattern.matches(&action, None));
}

// ---------------------------------------------------------------------------
// ServerTools matching tests
// ---------------------------------------------------------------------------

#[test]
fn test_server_tools_matches_any_tool() {
    let pattern = AllowancePattern::ServerTools {
        server: "filesystem".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::McpToolCall {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
        },
        None
    ));
    assert!(pattern.matches(
        &SensitiveAction::McpToolCall {
            server: "filesystem".to_string(),
            tool: "write_file".to_string(),
        },
        None
    ));
    assert!(!pattern.matches(
        &SensitiveAction::McpToolCall {
            server: "github".to_string(),
            tool: "create_issue".to_string(),
        },
        None
    ));
}

// ---------------------------------------------------------------------------
// FilePattern matching tests
// ---------------------------------------------------------------------------

#[test]
fn test_file_pattern_delete() {
    let pattern = AllowancePattern::FilePattern {
        pattern: "/tmp/**".to_string(),
        permission: Permission::Delete,
    };
    assert!(pattern.matches(
        &SensitiveAction::FileDelete {
            path: "/tmp/build/output.o".to_string(),
        },
        None
    ));
    assert!(!pattern.matches(
        &SensitiveAction::FileDelete {
            path: "/home/user/important.txt".to_string(),
        },
        None
    ));
}

#[test]
fn test_file_pattern_write() {
    let pattern = AllowancePattern::FilePattern {
        pattern: "/home/user/docs/*".to_string(),
        permission: Permission::Write,
    };
    assert!(pattern.matches(
        &SensitiveAction::FileWriteOutsideSandbox {
            path: "/home/user/docs/report.txt".to_string(),
        },
        None
    ));
    assert!(!pattern.matches(
        &SensitiveAction::FileWriteOutsideSandbox {
            path: "/etc/passwd".to_string(),
        },
        None
    ));
}

#[test]
fn test_file_pattern_permission_mismatch() {
    // Write pattern does not match FileDelete
    let pattern = AllowancePattern::FilePattern {
        pattern: "/tmp/**".to_string(),
        permission: Permission::Write,
    };
    assert!(!pattern.matches(
        &SensitiveAction::FileDelete {
            path: "/tmp/file.txt".to_string(),
        },
        None
    ));
}

#[test]
fn test_file_pattern_rejects_path_traversal() {
    let pattern = AllowancePattern::FilePattern {
        pattern: "/home/user/**".to_string(),
        permission: Permission::Delete,
    };
    assert!(!pattern.matches(
        &SensitiveAction::FileDelete {
            path: "/home/user/../../etc/passwd".to_string(),
        },
        None
    ));
}

// ---------------------------------------------------------------------------
// NetworkHost matching tests
// ---------------------------------------------------------------------------

#[test]
fn test_network_host_matches() {
    let pattern = AllowancePattern::NetworkHost {
        host: "api.example.com".to_string(),
        ports: None,
    };
    assert!(pattern.matches(
        &SensitiveAction::NetworkRequest {
            host: "api.example.com".to_string(),
            port: 443,
        },
        None
    ));
    assert!(pattern.matches(
        &SensitiveAction::NetworkRequest {
            host: "api.example.com".to_string(),
            port: 8080,
        },
        None
    ));
}

#[test]
fn test_network_host_with_ports() {
    let pattern = AllowancePattern::NetworkHost {
        host: "api.example.com".to_string(),
        ports: Some(vec![443, 8443]),
    };
    assert!(pattern.matches(
        &SensitiveAction::NetworkRequest {
            host: "api.example.com".to_string(),
            port: 443,
        },
        None
    ));
    assert!(!pattern.matches(
        &SensitiveAction::NetworkRequest {
            host: "api.example.com".to_string(),
            port: 80,
        },
        None
    ));
}

#[test]
fn test_network_host_wrong_host() {
    let pattern = AllowancePattern::NetworkHost {
        host: "api.example.com".to_string(),
        ports: None,
    };
    assert!(!pattern.matches(
        &SensitiveAction::NetworkRequest {
            host: "evil.com".to_string(),
            port: 443,
        },
        None
    ));
}

// ---------------------------------------------------------------------------
// Custom pattern tests
// ---------------------------------------------------------------------------

#[test]
fn test_custom_never_matches() {
    let pattern = AllowancePattern::Custom {
        pattern: "anything".to_string(),
    };
    assert!(!pattern.matches(
        &SensitiveAction::FileDelete {
            path: "/tmp/file".to_string(),
        },
        None
    ));
    assert!(!pattern.matches(
        &SensitiveAction::McpToolCall {
            server: "anything".to_string(),
            tool: "anything".to_string(),
        },
        None
    ));
}

// ---------------------------------------------------------------------------
// FileRead matching tests
// ---------------------------------------------------------------------------

#[test]
fn test_file_pattern_read() {
    let pattern = AllowancePattern::FilePattern {
        pattern: "/home/user/docs/**".to_string(),
        permission: Permission::Read,
    };
    assert!(pattern.matches(
        &SensitiveAction::FileRead {
            path: "/home/user/docs/report.txt".to_string(),
        },
        None
    ));
    assert!(!pattern.matches(
        &SensitiveAction::FileRead {
            path: "/etc/passwd".to_string(),
        },
        None
    ));
}

#[test]
fn test_file_pattern_read_does_not_match_write() {
    let pattern = AllowancePattern::FilePattern {
        pattern: "/home/user/**".to_string(),
        permission: Permission::Read,
    };
    // Read pattern must NOT match FileWriteOutsideSandbox
    assert!(!pattern.matches(
        &SensitiveAction::FileWriteOutsideSandbox {
            path: "/home/user/file.txt".to_string(),
        },
        None
    ));
}

#[test]
fn test_workspace_relative_read() {
    let pattern = AllowancePattern::WorkspaceRelative {
        pattern: "/project/src/**".to_string(),
        permission: Permission::Read,
    };
    assert!(pattern.matches(
        &SensitiveAction::FileRead {
            path: "/project/src/main.rs".to_string(),
        },
        None
    ));
    assert!(!pattern.matches(
        &SensitiveAction::FileRead {
            path: "/other/path/file.rs".to_string(),
        },
        None
    ));
}

#[test]
fn test_file_pattern_read_rejects_path_traversal() {
    let pattern = AllowancePattern::FilePattern {
        pattern: "/home/user/**".to_string(),
        permission: Permission::Read,
    };
    assert!(!pattern.matches(
        &SensitiveAction::FileRead {
            path: "/home/user/../../etc/passwd".to_string(),
        },
        None
    ));
}

// ---------------------------------------------------------------------------
// CommandPattern matching tests
// ---------------------------------------------------------------------------

#[test]
fn test_command_pattern_exact_match() {
    let pattern = AllowancePattern::CommandPattern {
        command: "cargo".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::ExecuteCommand {
            command: "cargo".to_string(),
            args: vec!["build".to_string()],
        },
        None
    ));
}

#[test]
fn test_command_pattern_glob_match() {
    let pattern = AllowancePattern::CommandPattern {
        command: "cargo*".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::ExecuteCommand {
            command: "cargo".to_string(),
            args: vec![],
        },
        None
    ));
}

#[test]
fn test_command_pattern_no_match() {
    let pattern = AllowancePattern::CommandPattern {
        command: "cargo".to_string(),
    };
    assert!(!pattern.matches(
        &SensitiveAction::ExecuteCommand {
            command: "sudo".to_string(),
            args: vec![],
        },
        None
    ));
}

#[test]
fn test_command_pattern_does_not_match_other_action_types() {
    let pattern = AllowancePattern::CommandPattern {
        command: "cargo".to_string(),
    };
    assert!(!pattern.matches(
        &SensitiveAction::FileRead {
            path: "cargo".to_string(),
        },
        None
    ));
    assert!(!pattern.matches(
        &SensitiveAction::McpToolCall {
            server: "cargo".to_string(),
            tool: "build".to_string(),
        },
        None
    ));
}

#[test]
fn test_command_pattern_display() {
    let pattern = AllowancePattern::CommandPattern {
        command: "cargo".to_string(),
    };
    assert_eq!(pattern.to_string(), "cmd:cargo");
}
