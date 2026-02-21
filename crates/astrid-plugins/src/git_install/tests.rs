#[cfg(test)]
pub mod tests {
    use crate::git_install::*;
    use std::path::Path;
    use tempfile::TempDir;
    use crate::error::{PluginError, PluginResult};
    fn parse_github_simple() {
        let src = GitSource::parse("github:unicitynetwork/openclaw-unicity").unwrap();
        assert_eq!(
            src,
            GitSource::GitHub {
                org: "unicitynetwork".to_string(),
                repo: "openclaw-unicity".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_github_with_ref() {
        let src = GitSource::parse("github:unicitynetwork/openclaw-unicity@v0.3.9").unwrap();
        assert_eq!(
            src,
            GitSource::GitHub {
                org: "unicitynetwork".to_string(),
                repo: "openclaw-unicity".to_string(),
                git_ref: Some("v0.3.9".to_string()),
            }
        );
    }

    #[test]
    fn parse_git_url_https() {
        let src = GitSource::parse("git:https://gitlab.com/org/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://gitlab.com/org/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_git_url_with_ref() {
        let src = GitSource::parse("git:https://gitlab.com/org/repo.git@main").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://gitlab.com/org/repo.git".to_string(),
                git_ref: Some("main".to_string()),
            }
        );
    }

    #[test]
    fn parse_git_url_ssh() {
        let src = GitSource::parse("git:ssh://git@github.com/org/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "ssh://git@github.com/org/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn reject_file_scheme() {
        let err = GitSource::parse("git:file:///etc/passwd").unwrap_err();
        assert!(err.to_string().contains("blocked URL scheme"));
    }

    #[test]
    fn reject_javascript_scheme() {
        let err = GitSource::parse("git:javascript:alert(1)").unwrap_err();
        assert!(err.to_string().contains("blocked URL scheme"));
    }

    #[test]
    fn reject_invalid_format() {
        let err = GitSource::parse("npm:some-package").unwrap_err();
        assert!(err.to_string().contains("invalid git source"));
    }

    #[test]
    fn reject_empty_org_or_repo() {
        assert!(GitSource::parse("github:/repo").is_err());
        assert!(GitSource::parse("github:org/").is_err());
        assert!(GitSource::parse("github:noslash").is_err());
    }

    #[test]
    fn plugin_id_hint_github() {
        let src = GitSource::GitHub {
            org: "unicity".to_string(),
            repo: "openclaw-unicity".to_string(),
            git_ref: None,
        };
        assert_eq!(src.plugin_id_hint(), "openclaw-unicity");
    }

    #[test]
    fn plugin_id_hint_git_url() {
        let src = GitSource::GitUrl {
            url: "https://gitlab.com/org/My_Plugin.git".to_string(),
            git_ref: None,
        };
        assert_eq!(src.plugin_id_hint(), "my-plugin");
    }

    #[test]
    fn display_source_github() {
        let src = GitSource::GitHub {
            org: "org".to_string(),
            repo: "repo".to_string(),
            git_ref: Some("v1.0".to_string()),
        };
        assert_eq!(src.display_source(), "github:org/repo@v1.0");
    }

    #[test]
    fn display_source_git_url() {
        let src = GitSource::GitUrl {
            url: "https://example.com/repo.git".to_string(),
            git_ref: None,
        };
        assert_eq!(src.display_source(), "git:https://example.com/repo.git");
    }

    #[test]
    fn strip_first_component_works() {
        let p = std::path::Path::new("org-repo-abc123/src/main.js");
        let stripped = strip_first_component(p);
        assert_eq!(stripped, PathBuf::from("src/main.js"));
    }

    #[test]
    fn strip_first_component_single() {
        let p = std::path::Path::new("only-one");
        let stripped = strip_first_component(p);
        assert_eq!(stripped, PathBuf::from(""));
    }

    #[test]
    fn validate_url_scheme_allowed() {
        assert!(validate_url_scheme("https://github.com/org/repo").is_ok());
        assert!(validate_url_scheme("ssh://git@github.com/org/repo").is_ok());
    }

    #[test]
    fn validate_url_scheme_blocked() {
        assert!(validate_url_scheme("file:///etc/passwd").is_err());
        assert!(validate_url_scheme("http://insecure.com/repo").is_err());
        assert!(validate_url_scheme("ftp://files.com/repo").is_err());
    }

    #[test]
    fn split_ref_ssh_url_without_ref() {
        // The '@' in ssh://git@github.com must NOT be treated as a ref delimiter
        let (url, git_ref) = split_ref("ssh://git@github.com/org/repo.git");
        assert_eq!(url, "ssh://git@github.com/org/repo.git");
        assert_eq!(git_ref, None);
    }

    #[test]
    fn split_ref_ssh_url_with_ref() {
        let (url, git_ref) = split_ref("ssh://git@github.com/org/repo.git@v1.0.0");
        assert_eq!(url, "ssh://git@github.com/org/repo.git");
        assert_eq!(git_ref, Some("v1.0.0".to_string()));
    }

    #[test]
    fn split_ref_https_url_with_ref() {
        let (url, git_ref) = split_ref("https://github.com/org/repo.git@main");
        assert_eq!(url, "https://github.com/org/repo.git");
        assert_eq!(git_ref, Some("main".to_string()));
    }

    #[test]
    fn split_ref_https_url_without_ref() {
        let (url, git_ref) = split_ref("https://github.com/org/repo.git");
        assert_eq!(url, "https://github.com/org/repo.git");
        assert_eq!(git_ref, None);
    }

    #[test]
    fn reject_github_org_with_slashes() {
        assert!(validate_github_component("org/evil", "org").is_err());
    }

    #[test]
    fn reject_github_org_with_url_injection() {
        assert!(validate_github_component("org/../admin", "org").is_err());
    }

    #[test]
    fn reject_github_component_leading_dash() {
        assert!(validate_github_component("-evil", "org").is_err());
    }

    #[test]
    fn reject_github_component_leading_dot() {
        assert!(validate_github_component(".hidden", "org").is_err());
    }

    #[test]
    fn accept_valid_github_components() {
        assert!(validate_github_component("my-org", "org").is_ok());
        assert!(validate_github_component("my.repo_name", "repo").is_ok());
        assert!(validate_github_component("CamelCase123", "org").is_ok());
    }

    #[test]
    fn reject_git_ref_with_double_dot() {
        assert!(validate_git_ref("main..evil").is_err());
    }

    #[test]
    fn reject_git_ref_with_control_chars() {
        assert!(validate_git_ref("main\x00evil").is_err());
        assert!(validate_git_ref("v1.0;rm -rf /").is_err());
    }

    #[test]
    fn reject_git_ref_too_long() {
        let long_ref = "a".repeat(257);
        assert!(validate_git_ref(&long_ref).is_err());
    }

    #[test]
    fn accept_valid_git_refs() {
        assert!(validate_git_ref("main").is_ok());
        assert!(validate_git_ref("v1.0.0").is_ok());
        assert!(validate_git_ref("feature/my-branch").is_ok());
        assert!(validate_git_ref("abc123def456").is_ok());
    }

    #[test]
    fn reject_github_source_with_invalid_org() {
        let err = GitSource::parse("github:org/slashes/not-allowed").unwrap_err();
        assert!(err.to_string().contains("invalid characters"));
    }

    #[test]
    fn reject_github_source_with_bad_ref() {
        let err = GitSource::parse("github:org/repo@main..evil").unwrap_err();
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn reject_github_component_double_dot() {
        assert!(validate_github_component("..", "org").is_err());
        assert!(validate_github_component(".", "org").is_err());
    }

    #[test]
    fn reject_github_component_leading_trailing_dot() {
        assert!(validate_github_component(".hidden", "org").is_err());
        assert!(validate_github_component("trailing.", "repo").is_err());
    }

    #[test]
    fn reject_git_ref_starting_with_slash() {
        assert!(validate_git_ref("/main").is_err());
    }

    #[test]
    fn reject_git_ref_ending_with_slash() {
        assert!(validate_git_ref("feature/").is_err());
    }

    #[test]
    fn split_ref_url_no_path() {
        let (url, git_ref) = split_ref("https://example.com");
        assert_eq!(url, "https://example.com");
        assert_eq!(git_ref, None);
    }

    #[test]
    fn reject_git_ref_leading_dash() {
        assert!(validate_git_ref("-evil").is_err());
        assert!(validate_git_ref("--double").is_err());
    }

    #[test]
    fn reject_git_ref_dot_lock_extension() {
        assert!(validate_git_ref("refs/heads/main.lock").is_err());
        assert!(validate_git_ref("branch.Lock").is_err());
    }

    #[test]
    fn plugin_id_hint_degenerate_all_special() {
        let src = GitSource::GitUrl {
            url: "https://example.com/___.git".to_string(),
            git_ref: None,
        };
        assert_eq!(src.plugin_id_hint(), "git-plugin");
    }

    #[test]
    fn plugin_id_hint_degenerate_empty_segment() {
        let src = GitSource::GitUrl {
            url: "https://example.com/.git".to_string(),
            git_ref: None,
        };
        assert_eq!(src.plugin_id_hint(), "git-plugin");
    }

    #[test]
    fn split_ref_multiple_at_in_path() {
        // Should split on the LAST @ in the path
        let (url, git_ref) = split_ref("https://host/p@th/repo@v1.0");
        assert_eq!(url, "https://host/p@th/repo");
        assert_eq!(git_ref, Some("v1.0".to_string()));
    }

    #[test]
    fn parse_bare_https_github() {
        let src = GitSource::parse("https://github.com/user/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://github.com/user/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_bare_https_github_no_dot_git() {
        let src = GitSource::parse("https://github.com/user/repo").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://github.com/user/repo".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_bare_https_github_with_ref() {
        let src = GitSource::parse("https://github.com/user/repo.git@main").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://github.com/user/repo.git".to_string(),
                git_ref: Some("main".to_string()),
            }
        );
    }

    #[test]
    fn parse_bare_https_gitlab() {
        let src = GitSource::parse("https://gitlab.com/org/repo").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://gitlab.com/org/repo".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_bare_https_dot_git_suffix() {
        let src = GitSource::parse("https://custom-host.com/org/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://custom-host.com/org/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_ssh_scp_style() {
        let src = GitSource::parse("git@github.com:user/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "ssh://git@github.com/user/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_ssh_scp_style_with_ref() {
        let src = GitSource::parse("git@github.com:user/repo.git@v2.0").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "ssh://git@github.com/user/repo.git".to_string(),
                git_ref: Some("v2.0".to_string()),
            }
        );
    }

    #[test]
    fn reject_bare_https_unknown_host() {
        // https:// without known host or .git suffix should not match
        assert!(GitSource::parse("https://example.com/something").is_err());
    }

    #[test]
    fn reject_spoofed_github_in_path() {
        // github.com in the *path* (not host) must NOT be accepted
        assert!(GitSource::parse("https://evil.com/github.com/payload").is_err());
    }

    #[test]
    fn reject_spoofed_gitlab_in_path() {
        assert!(GitSource::parse("https://evil.com/gitlab.com/payload").is_err());
    }

    #[test]
    fn parse_bare_https_custom_host_with_ref() {
        // .git extension with @ref on a non-github/gitlab host
        let src = GitSource::parse("https://custom-host.com/org/repo.git@main").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://custom-host.com/org/repo.git".to_string(),
                git_ref: Some("main".to_string()),
            }
        );
    }

    #[test]
    fn reject_ssh_path_traversal() {
        assert!(GitSource::parse("git@github.com:../../etc/passwd").is_err());
    }

    #[test]
    fn reject_ssh_host_with_spaces() {
        assert!(GitSource::parse("git@evil host:org/repo").is_err());
    }

    #[test]
    fn reject_ssh_host_with_slashes() {
        assert!(GitSource::parse("git@evil/host:org/repo").is_err());
    }

    #[test]
    fn reject_ssh_empty_host() {
        assert!(GitSource::parse("git@:org/repo").is_err());
    }

    #[test]
    fn reject_ssh_empty_path() {
        assert!(GitSource::parse("git@host:").is_err());
    }

    #[test]
    fn reject_ssh_no_colon() {
        assert!(GitSource::parse("git@host-no-colon").is_err());
    }
}
