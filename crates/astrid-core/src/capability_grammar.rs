//! Shared grammar and validation for static capability strings.
//!
//! Layer 5 of the multi-tenancy work (see issue #670) uses a colon-delimited
//! identifier namespace for policy capabilities:
//!
//! ```text
//! capability  := segment (':' segment)*
//! segment     := '*' | [a-zA-Z0-9_-]+
//! ```
//!
//! This is a **different namespace** from the runtime
//! [`CapabilityToken`](../../../astrid-capabilities/src/token.rs) resource
//! patterns (URI-based `mcp://server:tool`, globset-powered). Static
//! capabilities identify role membership and are stored in principal
//! profiles and group configs; runtime tokens gate individual tool calls.
//!
//! The grammar is deliberately restrictive — ASCII-only, no shell
//! metacharacters, no double-glob — so that capability strings round-trip
//! through TOML and the audit log without escaping surprises.

use thiserror::Error;

/// Upper bound on the total length of a capability pattern, in bytes.
///
/// A single capability identifier never legitimately approaches this
/// limit; the cap exists purely to reject pathological entries before
/// they reach the matcher.
pub const MAX_CAPABILITY_LEN: usize = 256;

/// Errors raised by [`validate_capability`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CapabilityGrammarError {
    /// Capability string is empty.
    #[error("capability must not be empty")]
    Empty,
    /// Capability string exceeds [`MAX_CAPABILITY_LEN`] bytes.
    #[error("capability exceeds {MAX_CAPABILITY_LEN} bytes")]
    TooLong,
    /// Capability contains the double-glob sequence `**` (reserved).
    #[error("capability may not contain '**' (double glob is reserved)")]
    DoubleStar,
    /// A segment between `:` separators is empty (leading, trailing,
    /// or consecutive colons).
    #[error("capability contains an empty segment (leading, trailing, or consecutive ':')")]
    EmptySegment,
    /// A segment contains a character outside the allowed grammar.
    #[error(
        "capability segment {segment:?} contains invalid character {bad:?} (allowed: a-z, A-Z, 0-9, -, _, or literal '*')"
    )]
    InvalidCharacter {
        /// Segment that failed validation.
        segment: String,
        /// First offending character.
        bad: char,
    },
    /// A segment mixes `*` with other characters (e.g. `foo*`).
    #[error(
        "capability segment {segment:?} mixes '*' with other characters; '*' must stand alone in a segment"
    )]
    PartialStar {
        /// Segment that failed validation.
        segment: String,
    },
}

/// Validate a capability string against the Layer 5 grammar.
///
/// Accepts both exact identifiers (`system:shutdown`) and patterns
/// (`self:*`, `a:*:b`, `*`). Rejects empty segments, double-globs,
/// non-ASCII characters, shell metacharacters, and segments that mix
/// `*` with literal characters.
///
/// # Errors
///
/// Returns the first [`CapabilityGrammarError`] encountered; rule order
/// is not part of the public contract.
pub fn validate_capability(cap: &str) -> Result<(), CapabilityGrammarError> {
    if cap.is_empty() {
        return Err(CapabilityGrammarError::Empty);
    }
    if cap.len() > MAX_CAPABILITY_LEN {
        return Err(CapabilityGrammarError::TooLong);
    }
    if cap.contains("**") {
        return Err(CapabilityGrammarError::DoubleStar);
    }
    for segment in cap.split(':') {
        if segment.is_empty() {
            return Err(CapabilityGrammarError::EmptySegment);
        }
        if segment == "*" {
            continue;
        }
        if segment.contains('*') {
            return Err(CapabilityGrammarError::PartialStar {
                segment: segment.to_string(),
            });
        }
        if let Some(bad) = segment
            .chars()
            .find(|c| !c.is_ascii_alphanumeric() && *c != '-' && *c != '_')
        {
            return Err(CapabilityGrammarError::InvalidCharacter {
                segment: segment.to_string(),
                bad,
            });
        }
    }
    Ok(())
}

/// Test whether a capability `pattern` matches the concrete capability
/// `cap`. Both inputs are expected to be pre-validated via
/// [`validate_capability`]; behaviour on malformed input is unspecified.
///
/// Matching rules:
///
/// - `*` alone matches any capability.
/// - A trailing `*` segment (`self:*`) matches one-or-more remaining
///   segments (`self:capsule:install`).
/// - A `*` segment elsewhere matches exactly one segment.
/// - Otherwise segments must match literally and the segment counts
///   must agree.
#[must_use]
pub fn capability_matches(pattern: &str, cap: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    // Walk both strings segment-by-segment with iterators — no Vec
    // allocation on the hot path. The Layer 5 preamble evaluates this
    // on every admin-API request and per-group-capability, so saving
    // the two `Vec<&str>` collections is worthwhile.
    let mut pat_iter = pattern.split(':').peekable();
    let mut cap_iter = cap.split(':');

    loop {
        match (pat_iter.next(), cap_iter.next()) {
            (Some("*"), Some(_)) => {
                // Trailing `*` absorbs every remaining resource segment.
                // A middle `*` matches exactly one and we continue the loop.
                if pat_iter.peek().is_none() {
                    return true;
                }
            },
            (Some(p), Some(c)) => {
                if p != c {
                    return false;
                }
            },
            (None, None) => return true,
            // Pattern and resource had different segment counts.
            (Some(_), None) | (None, Some(_)) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_capability ─────────────────────────────────────────────

    #[test]
    fn accepts_literal() {
        validate_capability("system:shutdown").unwrap();
        validate_capability("self:capsule:install").unwrap();
        validate_capability("audit:read:alice").unwrap();
        validate_capability("agent-007").unwrap();
    }

    #[test]
    fn accepts_universal_and_prefix_patterns() {
        validate_capability("*").unwrap();
        validate_capability("self:*").unwrap();
        validate_capability("a:*:b").unwrap();
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(validate_capability(""), Err(CapabilityGrammarError::Empty));
    }

    #[test]
    fn rejects_double_glob() {
        assert_eq!(
            validate_capability("**"),
            Err(CapabilityGrammarError::DoubleStar)
        );
        assert_eq!(
            validate_capability("self:**"),
            Err(CapabilityGrammarError::DoubleStar)
        );
        assert_eq!(
            validate_capability("**:read"),
            Err(CapabilityGrammarError::DoubleStar)
        );
    }

    #[test]
    fn rejects_empty_segment() {
        assert_eq!(
            validate_capability(":read"),
            Err(CapabilityGrammarError::EmptySegment)
        );
        assert_eq!(
            validate_capability("read:"),
            Err(CapabilityGrammarError::EmptySegment)
        );
        assert_eq!(
            validate_capability("a::b"),
            Err(CapabilityGrammarError::EmptySegment)
        );
    }

    #[test]
    fn rejects_shell_metachars() {
        for bad in [
            "system:shut down",
            "system:shutdown;rm",
            "system:`whoami`",
            "system:$(pwd)",
            "system:shutdown|id",
            "self:>log",
        ] {
            assert!(
                matches!(
                    validate_capability(bad),
                    Err(CapabilityGrammarError::InvalidCharacter { .. })
                ),
                "{bad:?} should be rejected",
            );
        }
    }

    #[test]
    fn rejects_partial_star() {
        assert!(matches!(
            validate_capability("self:foo*"),
            Err(CapabilityGrammarError::PartialStar { .. })
        ));
        assert!(matches!(
            validate_capability("*foo"),
            Err(CapabilityGrammarError::PartialStar { .. })
        ));
    }

    #[test]
    fn rejects_over_length() {
        let long = "a".repeat(MAX_CAPABILITY_LEN + 1);
        assert_eq!(
            validate_capability(&long),
            Err(CapabilityGrammarError::TooLong)
        );
    }

    // ── capability_matches ──────────────────────────────────────────────

    #[test]
    fn universal_matches_everything() {
        assert!(capability_matches("*", "system:shutdown"));
        assert!(capability_matches("*", "self:capsule:install"));
        assert!(capability_matches("*", "anything"));
    }

    #[test]
    fn exact_match() {
        assert!(capability_matches("system:shutdown", "system:shutdown"));
        assert!(!capability_matches("system:shutdown", "system:status"));
        assert!(!capability_matches(
            "system:shutdown",
            "self:system:shutdown"
        ));
    }

    #[test]
    fn trailing_star_matches_one_or_more() {
        assert!(capability_matches("self:*", "self:capsule"));
        assert!(capability_matches("self:*", "self:capsule:install"));
        assert!(capability_matches("self:*", "self:capsule:install:alice"));
        assert!(!capability_matches("self:*", "self"));
        assert!(!capability_matches("self:*", "capsule:install"));
    }

    #[test]
    fn middle_star_matches_single_segment() {
        assert!(capability_matches("a:*:b", "a:x:b"));
        assert!(!capability_matches("a:*:b", "a:x:y:b"));
        assert!(!capability_matches("a:*:b", "a:b"));
    }

    #[test]
    fn mixed_patterns() {
        assert!(capability_matches("audit:read:*", "audit:read:alice"));
        assert!(!capability_matches("audit:read:*", "audit:write:alice"));
    }
}
