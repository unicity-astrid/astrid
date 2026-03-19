//! # Astrid User Identity
//!
//! Provides [`AstridUserId`], the canonical internal user identity across all
//! platforms, and [`FrontendLink`], a mapping from platform-specific identities
//! to Astrid users.

/// Core identity types.
pub mod types;

pub use types::{AstridUserId, FrontendLink, normalize_platform};

#[cfg(test)]
mod tests {
    use super::types::AstridUserId;
    use super::*;

    #[test]
    fn test_astrid_user_id_creation() {
        let user1 = AstridUserId::new();
        let user2 = AstridUserId::new();
        assert_ne!(user1.id, user2.id);
    }

    #[test]
    fn test_astrid_user_id_default_principal() {
        let user = AstridUserId::new();
        assert_eq!(user.principal.as_str(), "default");
    }

    #[test]
    fn test_astrid_user_id_display() {
        let user = AstridUserId::new();
        let display = user.to_string();
        assert!(display.starts_with("user:"));

        let user_with_name = AstridUserId::new().with_display_name("Alice");
        let display = user_with_name.to_string();
        assert!(display.starts_with("Alice("));
    }

    #[test]
    fn test_principal_derived_from_display_name() {
        let user = AstridUserId::new().with_display_name("Josh Bouw");
        assert_eq!(user.principal.as_str(), "josh-bouw");
    }

    #[test]
    fn test_principal_derived_unicode_fallback() {
        // All non-ASCII chars → hyphens → collapsed → empty → fallback.
        let user = AstridUserId::new().with_display_name("日本語");
        assert!(
            user.principal.as_str().starts_with("user-"),
            "expected uuid fallback, got: {}",
            user.principal.as_str()
        );
    }

    #[test]
    fn test_principal_derived_empty_string() {
        let user = AstridUserId::new().with_display_name("");
        assert!(
            user.principal.as_str().starts_with("user-"),
            "expected uuid fallback, got: {}",
            user.principal.as_str()
        );
    }

    #[test]
    fn test_principal_derived_special_chars() {
        let user = AstridUserId::new().with_display_name("alice@example.com");
        assert_eq!(user.principal.as_str(), "alice-example-com");
    }

    #[test]
    fn test_principal_derived_truncation() {
        let long_name = "a".repeat(100);
        let user = AstridUserId::new().with_display_name(&long_name);
        assert!(user.principal.as_str().len() <= 64);
    }

    #[test]
    fn test_principal_explicit_override() {
        let principal = crate::PrincipalId::new("custom-principal").unwrap();
        let user = AstridUserId::new()
            .with_display_name("Alice")
            .with_principal(principal);
        assert_eq!(user.principal.as_str(), "custom-principal");
    }

    #[test]
    fn normalize_platform_trims_and_lowercases() {
        assert_eq!(normalize_platform("Discord"), "discord");
        assert_eq!(normalize_platform("  TELEGRAM  "), "telegram");
        assert_eq!(normalize_platform("matrix"), "matrix");
        assert_eq!(normalize_platform("  Matrix  "), "matrix");
    }
}
