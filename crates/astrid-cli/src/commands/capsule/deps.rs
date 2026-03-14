//! `astrid capsule deps` - visualize the capsule dependency graph.

use colored::Colorize;

use astrid_capsule::toposort::capability_matches;

use super::meta::{InstalledCapsule, scan_installed_capsules};
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Graph data types (testable core) - borrowed string slices
// ---------------------------------------------------------------------------

/// A single satisfied requirement edge.
#[derive(Debug)]
struct ProviderMatch<'a> {
    /// Name of the capsule that provides the capability.
    capsule_name: &'a str,
    /// The specific capability string that matched.
    matched_capability: &'a str,
}

/// All dependency edges for one capsule.
#[derive(Debug)]
struct CapsuleDeps<'a> {
    name: &'a str,
    edges: Vec<DepEdge<'a>>,
}

/// One requirement and its resolved providers.
#[derive(Debug)]
struct DepEdge<'a> {
    requirement: &'a str,
    providers: Vec<ProviderMatch<'a>>,
}

/// An unsatisfied requirement.
#[derive(Debug)]
struct Unsatisfied<'a> {
    capsule_name: &'a str,
    requirement: &'a str,
}

/// Build the dependency graph from installed capsule metadata.
///
/// For each capsule's `requires`, finds ALL capsules whose `provides` satisfy
/// the requirement via [`capability_matches`]. Returns the per-capsule deps
/// and any requirements that no installed capsule satisfies.
///
/// All string data is borrowed from the input slice to avoid string allocations.
fn build_dep_graph(capsules: &[InstalledCapsule]) -> (Vec<CapsuleDeps<'_>>, Vec<Unsatisfied<'_>>) {
    let mut all_deps = Vec::new();
    let mut unsatisfied = Vec::new();

    for cap in capsules {
        let requires = cap.meta.as_ref().map_or(&[][..], |m| m.requires.as_slice());

        let mut edges = Vec::new();

        for req in requires {
            let mut providers = Vec::new();

            for other in capsules {
                if other.name == cap.name && other.location == cap.location {
                    continue;
                }
                let offered = other
                    .meta
                    .as_ref()
                    .map_or(&[][..], |m| m.provides.as_slice());

                for prov in offered {
                    if capability_matches(req, prov) {
                        providers.push(ProviderMatch {
                            capsule_name: &other.name,
                            matched_capability: prov,
                        });
                    }
                }
            }

            if providers.is_empty() {
                unsatisfied.push(Unsatisfied {
                    capsule_name: &cap.name,
                    requirement: req,
                });
            }

            edges.push(DepEdge {
                requirement: req,
                providers,
            });
        }

        all_deps.push(CapsuleDeps {
            name: &cap.name,
            edges,
        });
    }

    (all_deps, unsatisfied)
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

/// Show the capsule dependency graph.
pub(crate) fn show_deps() -> anyhow::Result<()> {
    let capsules = scan_installed_capsules()?;

    if capsules.is_empty() {
        println!("{}", Theme::info("No capsules installed."));
        return Ok(());
    }

    println!("{}", Theme::header("Capsule Dependency Graph"));
    println!("{}", Theme::separator());

    let (all_deps, unsatisfied) = build_dep_graph(&capsules);

    for (i, dep) in all_deps.iter().enumerate() {
        if i > 0 {
            println!();
        }

        if dep.edges.is_empty() {
            println!(
                "{}  {}",
                dep.name.bold(),
                Theme::dimmed("(no dependencies)")
            );
            continue;
        }

        println!("{}", dep.name.bold());
        for edge in &dep.edges {
            println!("  requires {}", edge.requirement.cyan());
            if edge.providers.is_empty() {
                println!(
                    "    {}",
                    Theme::warning("no installed capsule provides this")
                );
            } else {
                for pm in &edge.providers {
                    if pm.matched_capability == edge.requirement {
                        // Exact match - no need to show "via"
                        println!("    <- {}", pm.capsule_name.bold());
                    } else {
                        println!(
                            "    <- {} {}",
                            pm.capsule_name.bold(),
                            Theme::dimmed(&format!("(via {})", pm.matched_capability)),
                        );
                    }
                }
            }
        }
    }

    if !unsatisfied.is_empty() {
        println!();
        println!("{}", Theme::header("Unsatisfied Requirements"));
        for u in &unsatisfied {
            println!(
                "  {} requires {}",
                u.capsule_name.bold(),
                u.requirement.cyan()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::capsule::meta::{CapsuleLocation, CapsuleMeta, InstalledCapsule};

    fn make_capsule(name: &str, provides: &[&str], requires: &[&str]) -> InstalledCapsule {
        InstalledCapsule {
            name: name.to_string(),
            meta: Some(CapsuleMeta {
                version: "1.0.0".to_string(),
                installed_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                source: None,
                provides: provides.iter().map(|s| (*s).to_string()).collect(),
                requires: requires.iter().map(|s| (*s).to_string()).collect(),
                topics: vec![],
            }),
            location: CapsuleLocation::User,
        }
    }

    #[test]
    fn test_build_dep_graph_basic() {
        let capsules = vec![
            make_capsule("provider", &["topic:foo"], &[]),
            make_capsule("consumer", &[], &["topic:foo"]),
        ];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        // Consumer should have one edge pointing to provider
        let consumer_deps = deps
            .iter()
            .find(|d| d.name == "consumer")
            .expect("consumer");
        assert_eq!(consumer_deps.edges.len(), 1);
        assert_eq!(consumer_deps.edges[0].requirement, "topic:foo");
        assert_eq!(consumer_deps.edges[0].providers.len(), 1);
        assert_eq!(consumer_deps.edges[0].providers[0].capsule_name, "provider");
    }

    #[test]
    fn test_build_dep_graph_wildcard() {
        let capsules = vec![
            make_capsule("anthropic", &["topic:llm.v1.stream.anthropic"], &[]),
            make_capsule("consumer", &[], &["topic:llm.v1.stream.*"]),
        ];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        let consumer_deps = deps
            .iter()
            .find(|d| d.name == "consumer")
            .expect("consumer");
        assert_eq!(consumer_deps.edges[0].providers.len(), 1);
        assert_eq!(
            consumer_deps.edges[0].providers[0].matched_capability,
            "topic:llm.v1.stream.anthropic"
        );
    }

    #[test]
    fn test_build_dep_graph_unsatisfied() {
        let capsules = vec![make_capsule("consumer", &[], &["topic:missing"])];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert_eq!(unsatisfied.len(), 1);
        assert_eq!(unsatisfied[0].capsule_name, "consumer");
        assert_eq!(unsatisfied[0].requirement, "topic:missing");
        // Edge still exists but with no providers
        assert_eq!(deps[0].edges[0].providers.len(), 0);
    }

    #[test]
    fn test_build_dep_graph_multiple_providers() {
        let capsules = vec![
            make_capsule("anthropic", &["topic:llm.v1.stream.anthropic"], &[]),
            make_capsule("openai", &["topic:llm.v1.stream.openai"], &[]),
            make_capsule("consumer", &[], &["topic:llm.v1.stream.*"]),
        ];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        let consumer_deps = deps
            .iter()
            .find(|d| d.name == "consumer")
            .expect("consumer");
        // Both providers should be listed
        assert_eq!(consumer_deps.edges[0].providers.len(), 2);
        let provider_names: Vec<&str> = consumer_deps.edges[0]
            .providers
            .iter()
            .map(|p| p.capsule_name)
            .collect();
        assert!(provider_names.contains(&"anthropic"));
        assert!(provider_names.contains(&"openai"));
    }

    #[test]
    fn test_build_dep_graph_no_requires() {
        let capsules = vec![make_capsule("standalone", &["topic:foo"], &[])];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        assert!(deps[0].edges.is_empty());
    }

    #[test]
    fn test_build_dep_graph_no_meta() {
        // Capsule with no meta.json should have no edges
        let capsules = vec![InstalledCapsule {
            name: "legacy".to_string(),
            meta: None,
            location: CapsuleLocation::User,
        }];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        assert!(deps[0].edges.is_empty());
    }

    #[test]
    fn test_build_dep_graph_same_name_different_location() {
        // Same capsule name at user and workspace level should NOT self-exclude.
        // The workspace copy can legitimately depend on the user copy if they
        // have different capabilities.
        let capsules = vec![
            InstalledCapsule {
                name: "mycapsule".to_string(),
                meta: Some(CapsuleMeta {
                    version: "1.0.0".to_string(),
                    installed_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                    source: None,
                    provides: vec!["topic:foo".to_string()],
                    requires: vec![],
                    topics: vec![],
                }),
                location: CapsuleLocation::User,
            },
            InstalledCapsule {
                name: "mycapsule".to_string(),
                meta: Some(CapsuleMeta {
                    version: "2.0.0".to_string(),
                    installed_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                    source: None,
                    provides: vec![],
                    requires: vec!["topic:foo".to_string()],
                    topics: vec![],
                }),
                location: CapsuleLocation::Workspace,
            },
        ];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        // The workspace copy should see the user copy as a provider
        let ws_deps = deps
            .iter()
            .find(|d| d.name == "mycapsule" && !d.edges.is_empty())
            .expect("workspace copy should have edges");
        assert_eq!(ws_deps.edges[0].providers.len(), 1);
        assert_eq!(ws_deps.edges[0].providers[0].capsule_name, "mycapsule");
    }
}
