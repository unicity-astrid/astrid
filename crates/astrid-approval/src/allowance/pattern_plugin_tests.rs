use super::*;
use crate::action::SensitiveAction;
use astrid_core::types::Permission;

// ---------------------------------------------------------------------------
// CapsuleCapability matching tests
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_capability_matches_execution() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "weather".to_string(),
        capability: "config_read".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::CapsuleExecution {
            capsule_id: "weather".to_string(),
            capability: "config_read".to_string(),
        },
        None
    ));
}

#[test]
fn test_plugin_capability_wrong_plugin() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "weather".to_string(),
        capability: "config_read".to_string(),
    };
    assert!(!pattern.matches(
        &SensitiveAction::CapsuleExecution {
            capsule_id: "other".to_string(),
            capability: "config_read".to_string(),
        },
        None
    ));
}

#[test]
fn test_plugin_capability_wrong_capability() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "weather".to_string(),
        capability: "config_read".to_string(),
    };
    assert!(!pattern.matches(
        &SensitiveAction::CapsuleExecution {
            capsule_id: "weather".to_string(),
            capability: "config_write".to_string(),
        },
        None
    ));
}

#[test]
fn test_plugin_capability_matches_http_request() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "weather".to_string(),
        capability: "http_request".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::CapsuleHttpRequest {
            capsule_id: "weather".to_string(),
            url: "https://api.weather.com".to_string(),
            method: "GET".to_string(),
        },
        None
    ));
}

#[test]
fn test_plugin_capability_wrong_cap_for_http() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "weather".to_string(),
        capability: "file_read".to_string(),
    };
    assert!(!pattern.matches(
        &SensitiveAction::CapsuleHttpRequest {
            capsule_id: "weather".to_string(),
            url: "https://api.weather.com".to_string(),
            method: "GET".to_string(),
        },
        None
    ));
}

#[test]
fn test_plugin_capability_matches_file_read() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "cache".to_string(),
        capability: "file_read".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::CapsuleFileAccess {
            capsule_id: "cache".to_string(),
            path: "/tmp/data".to_string(),
            mode: Permission::Read,
        },
        None
    ));
}

#[test]
fn test_plugin_capability_matches_file_write() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "cache".to_string(),
        capability: "file_write".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::CapsuleFileAccess {
            capsule_id: "cache".to_string(),
            path: "/tmp/data".to_string(),
            mode: Permission::Write,
        },
        None
    ));
}

#[test]
fn test_plugin_capability_matches_file_delete() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "cache".to_string(),
        capability: "file_delete".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::CapsuleFileAccess {
            capsule_id: "cache".to_string(),
            path: "/tmp/data".to_string(),
            mode: Permission::Delete,
        },
        None
    ));
}

#[test]
fn test_plugin_capability_file_mode_mismatch() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "cache".to_string(),
        capability: "file_read".to_string(),
    };
    // file_read pattern should NOT match Write mode
    assert!(!pattern.matches(
        &SensitiveAction::CapsuleFileAccess {
            capsule_id: "cache".to_string(),
            path: "/tmp/data".to_string(),
            mode: Permission::Write,
        },
        None
    ));
}

// ---------------------------------------------------------------------------
// PluginWildcard matching tests
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_wildcard_matches_execution() {
    let pattern = AllowancePattern::PluginWildcard {
        capsule_id: "weather".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::CapsuleExecution {
            capsule_id: "weather".to_string(),
            capability: "anything".to_string(),
        },
        None
    ));
}

#[test]
fn test_plugin_wildcard_matches_http() {
    let pattern = AllowancePattern::PluginWildcard {
        capsule_id: "weather".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::CapsuleHttpRequest {
            capsule_id: "weather".to_string(),
            url: "https://example.com".to_string(),
            method: "GET".to_string(),
        },
        None
    ));
}

#[test]
fn test_plugin_wildcard_matches_file() {
    let pattern = AllowancePattern::PluginWildcard {
        capsule_id: "weather".to_string(),
    };
    assert!(pattern.matches(
        &SensitiveAction::CapsuleFileAccess {
            capsule_id: "weather".to_string(),
            path: "/tmp/file".to_string(),
            mode: Permission::Read,
        },
        None
    ));
}

#[test]
fn test_plugin_wildcard_wrong_plugin() {
    let pattern = AllowancePattern::PluginWildcard {
        capsule_id: "weather".to_string(),
    };
    assert!(!pattern.matches(
        &SensitiveAction::CapsuleExecution {
            capsule_id: "other".to_string(),
            capability: "anything".to_string(),
        },
        None
    ));
}

#[test]
fn test_plugin_patterns_dont_match_non_plugin_actions() {
    let cap_pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "test".to_string(),
        capability: "read".to_string(),
    };
    let wildcard_pattern = AllowancePattern::PluginWildcard {
        capsule_id: "test".to_string(),
    };
    let non_plugin = SensitiveAction::McpToolCall {
        server: "test".to_string(),
        tool: "read".to_string(),
    };
    assert!(!cap_pattern.matches(&non_plugin, None));
    assert!(!wildcard_pattern.matches(&non_plugin, None));

    let file_action = SensitiveAction::FileDelete {
        path: "/tmp/file".to_string(),
    };
    assert!(!cap_pattern.matches(&file_action, None));
    assert!(!wildcard_pattern.matches(&file_action, None));
}

#[test]
fn test_plugin_pattern_display() {
    let pattern = AllowancePattern::CapsuleCapability {
        capsule_id: "weather".to_string(),
        capability: "http_request".to_string(),
    };
    assert_eq!(pattern.to_string(), "capsule://weather:http_request");

    let pattern = AllowancePattern::PluginWildcard {
        capsule_id: "weather".to_string(),
    };
    assert_eq!(pattern.to_string(), "capsule://weather:*");
}

#[test]
fn test_plugin_pattern_serialization_roundtrip() {
    let patterns = vec![
        AllowancePattern::CapsuleCapability {
            capsule_id: "p1".to_string(),
            capability: "cap1".to_string(),
        },
        AllowancePattern::PluginWildcard {
            capsule_id: "p2".to_string(),
        },
    ];
    for pattern in patterns {
        let json = serde_json::to_string(&pattern).unwrap();
        let deserialized: AllowancePattern = serde_json::from_str(&json).unwrap();
        assert_eq!(pattern.to_string(), deserialized.to_string());
    }
}
