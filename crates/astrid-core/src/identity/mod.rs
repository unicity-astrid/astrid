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
    fn test_astrid_user_id_display() {
        let user = AstridUserId::new();
        let display = user.to_string();
        assert!(display.starts_with("user:"));

        let user_with_name = AstridUserId::new().with_display_name("Alice");
        let display = user_with_name.to_string();
        assert!(display.starts_with("Alice("));
    }

    #[test]
    fn normalize_platform_trims_and_lowercases() {
        assert_eq!(normalize_platform("Discord"), "discord");
        assert_eq!(normalize_platform("  TELEGRAM  "), "telegram");
        assert_eq!(normalize_platform("matrix"), "matrix");
        assert_eq!(normalize_platform("  Matrix  "), "matrix");
    }
}
