//! Topological sort for capsule dependency ordering.
//!
//! Implements Kahn's algorithm to order capsules so that dependencies load
//! before their dependents. Edges are derived from capability-based
//! `requires`/`provides` declarations rather than package names.

use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;

use crate::manifest::CapsuleManifest;

/// A manifest paired with its capsule directory path.
pub(crate) type ManifestEntry = (CapsuleManifest, PathBuf);

/// Error returned when the dependency graph contains a cycle.
#[derive(Debug, Clone)]
pub struct CycleError {
    /// Names of capsules involved in the cycle.
    pub cycle: Vec<String>,
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dependency cycle detected (involved nodes): {}",
            self.cycle.join(", ")
        )
    }
}

impl std::error::Error for CycleError {}

/// Match a capability requirement against a provided capability.
///
/// Both sides may contain `*` wildcards that match a single dot-separated
/// segment. The type prefix (before `:`) must match exactly; only the body
/// (after `:`) supports wildcard matching.
///
/// # Examples
///
/// - `topic:llm.stream.*` matches `topic:llm.stream.anthropic`
/// - `topic:foo` does NOT match `tool:foo` (type prefix mismatch)
/// - `topic:a.b` does NOT match `topic:a.b.c` (segment count mismatch)
pub fn capability_matches(requirement: &str, provided: &str) -> bool {
    let (req_type, req_body) = requirement.split_once(':').unwrap_or(("", requirement));
    let (prov_type, prov_body) = provided.split_once(':').unwrap_or(("", provided));
    if req_type != prov_type {
        return false;
    }
    // Zero-alloc: zip iterators directly instead of collecting into Vecs.
    // After zip exhausts the shorter side, check both are fully consumed
    // to enforce equal segment count.
    let mut req_segs = req_body.split('.');
    let mut prov_segs = prov_body.split('.');
    let all_matched = (&mut req_segs)
        .zip(&mut prov_segs)
        .all(|(r, p)| r == "*" || p == "*" || r == p);
    all_matched && req_segs.next().is_none() && prov_segs.next().is_none()
}

/// Sort capsule manifests in dependency order using Kahn's algorithm.
///
/// Capsules with no requirements come first. Dependencies are resolved by
/// matching each capsule's `requires` against other capsules' effective
/// `provides` using [`capability_matches`] for wildcard support.
///
/// Uses any-satisfies semantics: a requirement is met when ANY single
/// capsule provides a matching capability.
///
/// Unsatisfied requirements are logged as warnings and treated as met -
/// the capsule still loads, it just won't have that capability guaranteed.
///
/// # Errors
///
/// Returns [`CycleError`] paired with the original unsorted vector if the
/// dependency graph contains a cycle. This avoids cloning the input as a
/// fallback buffer.
pub fn toposort_manifests(
    manifests: Vec<ManifestEntry>,
) -> Result<Vec<ManifestEntry>, (CycleError, Vec<ManifestEntry>)> {
    if manifests.len() <= 1 {
        return Ok(manifests);
    }

    let len = manifests.len();
    // adjacency[i] = list of indices that depend on i (i.e., i must load before them)
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); len];
    let mut in_degree: Vec<usize> = vec![0; len];

    // Build capability-based adjacency list in a scoped block so the
    // borrows on `manifests` are released before the error path needs
    // to return ownership.
    {
        let all_provides: Vec<&[String]> = manifests
            .iter()
            .map(|(m, _)| m.effective_provides())
            .collect();

        for (idx, (manifest, _)) in manifests.iter().enumerate() {
            for req in &manifest.dependencies.requires {
                let mut satisfied = false;
                for (prov_idx, provides) in all_provides.iter().enumerate() {
                    if prov_idx == idx {
                        continue;
                    }
                    if provides.iter().any(|p| capability_matches(req, p)) {
                        // prov_idx must load before idx
                        dependents[prov_idx].push(idx);
                        in_degree[idx] += 1;
                        satisfied = true;
                        // Continue: all providers get an ordering edge.
                    }
                }
                if !satisfied {
                    tracing::warn!(
                        capsule = %manifest.package.name,
                        requirement = %req,
                        "Required capability not provided by any loaded capsule"
                    );
                }
            }
        }
    }

    // BFS from zero-in-degree nodes
    let mut queue: VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|(_, d)| **d == 0)
        .map(|(i, _)| i)
        .collect();

    let mut order: Vec<usize> = Vec::with_capacity(len);

    while let Some(idx) = queue.pop_front() {
        order.push(idx);
        for &dependent in &dependents[idx] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }

    if order.len() != len {
        // Extract cycle members (nodes with remaining in-degree > 0)
        let cycle: Vec<String> = in_degree
            .iter()
            .enumerate()
            .filter(|(_, d)| **d > 0)
            .map(|(i, _)| manifests[i].0.package.name.clone())
            .collect();
        return Err((CycleError { cycle }, manifests));
    }

    // Reorder manifests according to topological order.
    // Convert to Option so we can take() by index without cloning.
    let mut slots: Vec<Option<ManifestEntry>> = manifests.into_iter().map(Some).collect();
    let sorted = order
        .into_iter()
        .map(|i| slots[i].take().expect("each index visited exactly once"))
        .collect();

    Ok(sorted)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::manifest::{CapabilitiesDef, DependenciesDef, ToolDef};

    /// Create a manifest with explicit provides and requires capabilities.
    fn manifest_with_caps(name: &str, provides: &[&str], requires: &[&str]) -> ManifestEntry {
        let m = CapsuleManifest {
            package: crate::manifest::PackageDef {
                name: name.to_string(),
                version: "0.1.0".to_string(),
                description: None,
                authors: Vec::new(),
                repository: None,
                homepage: None,
                documentation: None,
                license: None,
                license_file: None,
                readme: None,
                keywords: Vec::new(),
                categories: Vec::new(),
                astrid_version: None,
                publish: None,
                include: None,
                exclude: None,
                metadata: None,
            },
            components: Vec::new(),
            dependencies: DependenciesDef {
                provides: provides.iter().map(|s| (*s).to_string()).collect(),
                requires: requires.iter().map(|s| (*s).to_string()).collect(),
            },
            capabilities: Default::default(),
            env: HashMap::new(),
            context_files: Vec::new(),
            commands: Vec::new(),
            mcp_servers: Vec::new(),
            skills: Vec::new(),
            uplinks: Vec::new(),
            llm_providers: Vec::new(),
            interceptors: Vec::new(),
            cron_jobs: Vec::new(),
            tools: Vec::new(),
            effective_provides_cache: std::sync::OnceLock::new(),
        };
        (m, PathBuf::from(format!("/capsules/{name}")))
    }

    /// Create a manifest with no explicit provides/requires (auto-derive test).
    fn manifest_bare(name: &str) -> ManifestEntry {
        manifest_with_caps(name, &[], &[])
    }

    fn names(manifests: &[ManifestEntry]) -> Vec<&str> {
        manifests
            .iter()
            .map(|(m, _)| m.package.name.as_str())
            .collect()
    }

    // -- capability_matches tests --

    #[test]
    fn capability_matches_exact() {
        assert!(capability_matches("topic:foo.bar", "topic:foo.bar"));
    }

    #[test]
    fn capability_matches_wildcard_in_requirement() {
        assert!(capability_matches(
            "topic:llm.stream.*",
            "topic:llm.stream.anthropic"
        ));
    }

    #[test]
    fn capability_matches_wildcard_in_provider() {
        assert!(capability_matches(
            "topic:llm.request.generate.anthropic",
            "topic:llm.request.generate.*"
        ));
    }

    #[test]
    fn capability_matches_type_mismatch() {
        assert!(!capability_matches("topic:foo", "tool:foo"));
    }

    #[test]
    fn capability_matches_segment_count_mismatch() {
        assert!(!capability_matches("topic:a.b", "topic:a.b.c"));
    }

    #[test]
    fn capability_matches_no_prefix() {
        assert!(capability_matches("foo", "foo"));
        assert!(!capability_matches("foo", "bar"));
    }

    #[test]
    fn capability_matches_both_wildcards() {
        assert!(capability_matches("topic:*.stream", "topic:llm.*"));
    }

    #[test]
    fn capability_matches_middle_wildcard() {
        assert!(capability_matches(
            "topic:llm.*.anthropic",
            "topic:llm.stream.anthropic"
        ));
        assert!(!capability_matches(
            "topic:llm.*.anthropic",
            "topic:llm.stream.openai"
        ));
    }

    #[test]
    fn capability_matches_empty_strings() {
        // capability_matches("", "") returns true as an implementation artifact:
        // split_once(':') on "" gives None, so both types are ""; split('.')
        // on "" gives [""], so both bodies match. This is only safe because
        // manifest validation (load_manifest) rejects empty strings before any
        // call to toposort_manifests. Tests that bypass manifest validation
        // must not pass empty strings here.
        assert!(capability_matches("", ""));
    }

    #[test]
    fn capability_matches_empty_body_after_prefix() {
        // "topic:" has an empty body, which splits to [""]. This matches
        // another "topic:" with empty body. Again, manifest validation
        // prevents this from occurring in practice.
        assert!(capability_matches("topic:", "topic:"));
        assert!(!capability_matches("topic:", "tool:"));
    }

    // -- toposort tests --

    #[test]
    fn empty_graph() {
        let result = toposort_manifests(vec![]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn single_node() {
        let input = vec![manifest_bare("a")];
        let result = toposort_manifests(input).unwrap();
        assert_eq!(names(&result), vec!["a"]);
    }

    #[test]
    fn capability_edge() {
        // B requires topic:foo, A provides topic:foo => A before B
        let input = vec![
            manifest_with_caps("b", &[], &["topic:foo"]),
            manifest_with_caps("a", &["topic:foo"], &[]),
        ];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        assert!(
            n.iter().position(|&x| x == "a").unwrap() < n.iter().position(|&x| x == "b").unwrap()
        );
    }

    #[test]
    fn wildcard_requires() {
        // B requires topic:llm.stream.*, A provides topic:llm.stream.anthropic => A before B
        let input = vec![
            manifest_with_caps("b", &[], &["topic:llm.stream.*"]),
            manifest_with_caps("a", &["topic:llm.stream.anthropic"], &[]),
        ];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        assert!(
            n.iter().position(|&x| x == "a").unwrap() < n.iter().position(|&x| x == "b").unwrap()
        );
    }

    #[test]
    fn all_providers_ordered_before_consumer() {
        // C requires topic:llm.stream.*, both A and B provide it.
        // All providers get ordering edges, so both A and B load before C.
        let input = vec![
            manifest_with_caps("c", &[], &["topic:llm.stream.*"]),
            manifest_with_caps("a", &["topic:llm.stream.anthropic"], &[]),
            manifest_with_caps("b", &["topic:llm.stream.openai"], &[]),
        ];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        let c_pos = n.iter().position(|&x| x == "c").unwrap();
        let a_pos = n.iter().position(|&x| x == "a").unwrap();
        let b_pos = n.iter().position(|&x| x == "b").unwrap();
        assert!(a_pos < c_pos);
        assert!(b_pos < c_pos);
    }

    #[test]
    fn unsatisfied_requirement_still_succeeds() {
        // B requires topic:missing - no provider exists, B still loads
        let input = vec![
            manifest_bare("a"),
            manifest_with_caps("b", &[], &["topic:missing"]),
        ];
        let result = toposort_manifests(input).unwrap();
        assert_eq!(names(&result).len(), 2);
    }

    #[test]
    fn cycle_detected() {
        // A requires what B provides, B requires what A provides
        let input = vec![
            manifest_with_caps("a", &["topic:x"], &["topic:y"]),
            manifest_with_caps("b", &["topic:y"], &["topic:x"]),
        ];
        let (err, original) = toposort_manifests(input).unwrap_err();
        assert!(err.cycle.contains(&"a".to_string()));
        assert!(err.cycle.contains(&"b".to_string()));
        assert!(err.to_string().contains("dependency cycle detected"));
        assert_eq!(original.len(), 2);
    }

    #[test]
    fn three_node_cycle() {
        let input = vec![
            manifest_with_caps("a", &["topic:x"], &["topic:z"]),
            manifest_with_caps("b", &["topic:y"], &["topic:x"]),
            manifest_with_caps("c", &["topic:z"], &["topic:y"]),
        ];
        let (err, original) = toposort_manifests(input).unwrap_err();
        assert_eq!(err.cycle.len(), 3);
        assert_eq!(original.len(), 3);
    }

    #[test]
    fn no_dependencies_preserves_all() {
        let input = vec![manifest_bare("x"), manifest_bare("y"), manifest_bare("z")];
        let result = toposort_manifests(input).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn diamond_dependency_via_capabilities() {
        // d requires topic:b + topic:c, b and c require topic:a
        let input = vec![
            manifest_with_caps("d", &["topic:d"], &["topic:b", "topic:c"]),
            manifest_with_caps("b", &["topic:b"], &["topic:a"]),
            manifest_with_caps("c", &["topic:c"], &["topic:a"]),
            manifest_with_caps("a", &["topic:a"], &[]),
        ];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        let pos = |name: &str| n.iter().position(|&x| x == name).unwrap();
        assert!(pos("a") < pos("b"));
        assert!(pos("a") < pos("c"));
        assert!(pos("b") < pos("d"));
        assert!(pos("c") < pos("d"));
    }

    // -- effective_provides tests --

    #[test]
    fn effective_provides_auto_derives_from_ipc_publish() {
        let (mut m, _) = manifest_bare("test");
        m.capabilities = CapabilitiesDef {
            ipc_publish: vec!["foo.bar".to_string(), "baz".to_string()],
            ..Default::default()
        };
        assert_eq!(m.effective_provides(), vec!["topic:foo.bar", "topic:baz"]);
    }

    #[test]
    fn effective_provides_explicit_overrides_auto_derive() {
        let (mut m, _) = manifest_bare("test");
        m.dependencies.provides = vec!["custom:cap".to_string()];
        m.capabilities = CapabilitiesDef {
            ipc_publish: vec!["should.be.ignored".to_string()],
            ..Default::default()
        };
        assert_eq!(m.effective_provides(), vec!["custom:cap"]);
    }

    #[test]
    fn effective_provides_includes_tools() {
        let (mut m, _) = manifest_bare("test");
        m.tools = vec![ToolDef {
            name: "run_shell".to_string(),
            description: "Run shell".to_string(),
            input_schema: serde_json::json!({}),
        }];
        assert_eq!(m.effective_provides(), vec!["tool:run_shell"]);
    }

    #[test]
    fn effective_provides_empty_when_nothing_declared() {
        let (m, _) = manifest_bare("test");
        assert!(m.effective_provides().is_empty());
    }

    // -- auto-derived provides create edges in toposort --

    #[test]
    fn toposort_uses_auto_derived_provides() {
        // A has ipc_publish = ["foo.bar"] (auto-derives topic:foo.bar)
        // B requires topic:foo.bar
        let (mut a, a_path) = manifest_bare("a");
        a.capabilities = CapabilitiesDef {
            ipc_publish: vec!["foo.bar".to_string()],
            ..Default::default()
        };
        let b = manifest_with_caps("b", &[], &["topic:foo.bar"]);

        let input = vec![b, (a, a_path)];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        assert!(
            n.iter().position(|&x| x == "a").unwrap() < n.iter().position(|&x| x == "b").unwrap()
        );
    }

    #[test]
    fn wildcard_body_in_requires_matches_all_providers_of_type() {
        // "topic:*" matches any single-segment topic body.
        // This is unusual but valid - creates ordering edges to all topic providers.
        let input = vec![
            manifest_with_caps("consumer", &[], &["topic:*"]),
            manifest_with_caps("provider-a", &["topic:foo"], &[]),
            manifest_with_caps("provider-b", &["topic:bar"], &[]),
        ];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        let pos = |name: &str| n.iter().position(|&x| x == name).unwrap();
        // Both providers must load before the consumer
        assert!(pos("provider-a") < pos("consumer"));
        assert!(pos("provider-b") < pos("consumer"));
    }

    // -- shipped capsule integration tests --

    #[test]
    fn react_requires_satisfied_by_identity_and_session() {
        // Verify that the react capsule's [dependencies].requires are
        // actually satisfiable by the identity and session capsules'
        // auto-derived provides (from their ipc_publish).
        let identity = {
            let (mut m, p) = manifest_bare("astrid-capsule-identity");
            m.capabilities = CapabilitiesDef {
                ipc_publish: vec!["identity.v1.response.ready".into()],
                ..Default::default()
            };
            (m, p)
        };
        let session = {
            let (mut m, p) = manifest_bare("astrid-capsule-session");
            m.capabilities = CapabilitiesDef {
                ipc_publish: vec!["session.v1.response.get_messages".into()],
                ..Default::default()
            };
            (m, p)
        };
        let react = manifest_with_caps(
            "astrid-capsule-react",
            &[],
            &[
                "topic:identity.v1.response.ready",
                "topic:session.v1.response.get_messages",
            ],
        );

        let input = vec![react, identity, session];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        let pos = |name: &str| n.iter().position(|&x| x == name).unwrap();

        // React must load after both identity and session
        assert!(pos("astrid-capsule-identity") < pos("astrid-capsule-react"));
        assert!(pos("astrid-capsule-session") < pos("astrid-capsule-react"));
    }
}
