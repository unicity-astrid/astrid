//! `astrid capsule deps` - visualize the capsule dependency graph.

use colored::Colorize;

use super::meta::{InstalledCapsule, scan_installed_capsules};
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Graph data types (testable core) - borrowed string slices
// ---------------------------------------------------------------------------

/// A single satisfied import edge.
#[derive(Debug)]
struct ProviderMatch<'a> {
    /// Name of the capsule that exports the interface.
    capsule_name: &'a str,
    /// The version exported.
    exported_version: &'a str,
}

/// All dependency edges for one capsule.
#[derive(Debug)]
struct CapsuleDeps<'a> {
    name: &'a str,
    edges: Vec<DepEdge<'a>>,
}

/// One import and its resolved providers.
#[derive(Debug)]
struct DepEdge<'a> {
    namespace: &'a str,
    interface: &'a str,
    version: &'a str,
    providers: Vec<ProviderMatch<'a>>,
}

/// An unsatisfied import.
#[derive(Debug)]
struct Unsatisfied<'a> {
    capsule_name: &'a str,
    namespace: &'a str,
    interface: &'a str,
    version: &'a str,
}

/// Build the dependency graph from installed capsule metadata.
///
/// For each capsule's imports, finds ALL capsules whose exports match
/// the namespace and interface name. Returns the per-capsule deps
/// and any imports that no installed capsule satisfies.
fn build_dep_graph(capsules: &[InstalledCapsule]) -> (Vec<CapsuleDeps<'_>>, Vec<Unsatisfied<'_>>) {
    let mut all_deps = Vec::new();
    let mut unsatisfied = Vec::new();

    for cap in capsules {
        let mut edges = Vec::new();

        let Some(ref meta) = cap.meta else {
            all_deps.push(CapsuleDeps {
                name: &cap.name,
                edges,
            });
            continue;
        };

        for (ns, ifaces) in &meta.imports {
            for (iface_name, version) in ifaces {
                let mut providers = Vec::new();

                for other in capsules {
                    if other.name == cap.name && other.location == cap.location {
                        continue;
                    }
                    if let Some(ref other_meta) = other.meta
                        && let Some(other_ns) = other_meta.exports.get(ns.as_str())
                        && let Some(exported_ver) = other_ns.get(iface_name.as_str())
                    {
                        providers.push(ProviderMatch {
                            capsule_name: &other.name,
                            exported_version: exported_ver,
                        });
                    }
                }

                if providers.is_empty() {
                    unsatisfied.push(Unsatisfied {
                        capsule_name: &cap.name,
                        namespace: ns,
                        interface: iface_name,
                        version,
                    });
                }

                edges.push(DepEdge {
                    namespace: ns,
                    interface: iface_name,
                    version,
                    providers,
                });
            }
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
            let iface = format!("{}/{} {}", edge.namespace, edge.interface, edge.version);
            println!("  imports {}", iface.cyan());
            if edge.providers.is_empty() {
                println!(
                    "    {}",
                    Theme::warning("no installed capsule exports this")
                );
            } else {
                for pm in &edge.providers {
                    println!(
                        "    <- {} {}",
                        pm.capsule_name.bold(),
                        Theme::dimmed(&format!("(v{})", pm.exported_version)),
                    );
                }
            }
        }
    }

    if !unsatisfied.is_empty() {
        println!();
        println!("{}", Theme::header("Unsatisfied Imports"));
        for u in &unsatisfied {
            let iface = format!("{}/{} {}", u.namespace, u.interface, u.version);
            println!("  {} imports {}", u.capsule_name.bold(), iface.cyan());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::capsule::meta::{CapsuleLocation, CapsuleMeta, InstalledCapsule};
    use std::collections::HashMap;

    fn make_capsule(
        name: &str,
        exports: &[(&str, &str, &str)],
        imports: &[(&str, &str, &str)],
    ) -> InstalledCapsule {
        let mut export_map: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (ns, iface, ver) in exports {
            export_map
                .entry(ns.to_string())
                .or_default()
                .insert(iface.to_string(), ver.to_string());
        }
        let mut import_map: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (ns, iface, ver) in imports {
            import_map
                .entry(ns.to_string())
                .or_default()
                .insert(iface.to_string(), ver.to_string());
        }
        InstalledCapsule {
            name: name.to_string(),
            meta: Some(CapsuleMeta {
                version: "1.0.0".to_string(),
                installed_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                source: None,
                imports: import_map,
                exports: export_map,
                topics: vec![],
                wasm_hash: None,
            }),
            location: CapsuleLocation::User,
        }
    }

    #[test]
    fn test_build_dep_graph_basic() {
        let capsules = vec![
            make_capsule("provider", &[("astrid", "session", "1.0.0")], &[]),
            make_capsule("consumer", &[], &[("astrid", "session", "^1.0")]),
        ];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        let consumer_deps = deps
            .iter()
            .find(|d| d.name == "consumer")
            .expect("consumer");
        assert_eq!(consumer_deps.edges.len(), 1);
        assert_eq!(consumer_deps.edges[0].interface, "session");
        assert_eq!(consumer_deps.edges[0].providers.len(), 1);
        assert_eq!(consumer_deps.edges[0].providers[0].capsule_name, "provider");
    }

    #[test]
    fn test_build_dep_graph_unsatisfied() {
        let capsules = vec![make_capsule(
            "consumer",
            &[],
            &[("astrid", "missing", "^1.0")],
        )];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert_eq!(unsatisfied.len(), 1);
        assert_eq!(unsatisfied[0].capsule_name, "consumer");
        assert_eq!(unsatisfied[0].interface, "missing");
        assert_eq!(deps[0].edges[0].providers.len(), 0);
    }

    #[test]
    fn test_build_dep_graph_multiple_providers() {
        let capsules = vec![
            make_capsule("openai", &[("astrid", "llm", "1.0.0")], &[]),
            make_capsule("ollama", &[("astrid", "llm", "1.0.0")], &[]),
            make_capsule("consumer", &[], &[("astrid", "llm", "^1.0")]),
        ];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        let consumer_deps = deps
            .iter()
            .find(|d| d.name == "consumer")
            .expect("consumer");
        assert_eq!(consumer_deps.edges[0].providers.len(), 2);
    }

    #[test]
    fn test_build_dep_graph_no_imports() {
        let capsules = vec![make_capsule(
            "standalone",
            &[("astrid", "session", "1.0.0")],
            &[],
        )];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        assert!(deps[0].edges.is_empty());
    }

    #[test]
    fn test_build_dep_graph_no_meta() {
        let capsules = vec![InstalledCapsule {
            name: "legacy".to_string(),
            meta: None,
            location: CapsuleLocation::User,
        }];
        let (deps, unsatisfied) = build_dep_graph(&capsules);

        assert!(unsatisfied.is_empty());
        assert!(deps[0].edges.is_empty());
    }
}
