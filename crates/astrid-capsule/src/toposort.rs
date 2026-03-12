//! Topological sort for capsule dependency ordering.
//!
//! Implements Kahn's algorithm to order capsules so that dependencies load
//! before their dependents. Cycles are detected and reported.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::PathBuf;

use crate::manifest::CapsuleManifest;

/// Error returned when the dependency graph contains a cycle.
#[derive(Debug, Clone)]
pub struct CycleError {
    /// Names of capsules involved in the cycle.
    pub cycle: Vec<String>,
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dependency cycle detected: {}", self.cycle.join(" -> "))
    }
}

impl std::error::Error for CycleError {}

/// Sort capsule manifests in dependency order using Kahn's algorithm.
///
/// Capsules with no dependencies come first. Dependencies are resolved
/// by matching `manifest.dependencies` keys against capsule package names.
///
/// Missing dependencies (names not in the input set) are logged as warnings
/// and treated as satisfied - the capsule still loads, it just won't have
/// that dependency guaranteed to be loaded first.
///
/// # Errors
///
/// Returns [`CycleError`] if the dependency graph contains a cycle.
pub fn toposort_manifests(
    manifests: Vec<(CapsuleManifest, PathBuf)>,
) -> Result<Vec<(CapsuleManifest, PathBuf)>, CycleError> {
    if manifests.len() <= 1 {
        return Ok(manifests);
    }

    let name_to_idx: HashMap<&str, usize> = manifests
        .iter()
        .enumerate()
        .map(|(i, (m, _))| (m.package.name.as_str(), i))
        .collect();

    let len = manifests.len();
    // adjacency[i] = list of indices that depend on i (i.e., i must load before them)
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); len];
    let mut in_degree: Vec<usize> = vec![0; len];

    for (idx, (manifest, _)) in manifests.iter().enumerate() {
        for dep_name in manifest.dependencies.keys() {
            if let Some(&dep_idx) = name_to_idx.get(dep_name.as_str()) {
                // dep_idx must load before idx
                dependents[dep_idx].push(idx);
                in_degree[idx] += 1;
            } else {
                tracing::warn!(
                    capsule = %manifest.package.name,
                    dependency = %dep_name,
                    "Capsule declares dependency on unknown capsule, ignoring"
                );
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
        return Err(CycleError { cycle });
    }

    // Reorder manifests according to topological order.
    // Convert to Option so we can take() by index without cloning.
    let mut slots: Vec<Option<(CapsuleManifest, PathBuf)>> =
        manifests.into_iter().map(Some).collect();
    let sorted = order
        .into_iter()
        .map(|i| slots[i].take().expect("each index visited exactly once"))
        .collect();

    Ok(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn manifest(name: &str, deps: &[&str]) -> (CapsuleManifest, PathBuf) {
        let dependencies: HashMap<String, String> = deps
            .iter()
            .map(|d| ((*d).to_string(), "*".to_string()))
            .collect();

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
            dependencies,
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
        };
        (m, PathBuf::from(format!("/capsules/{name}")))
    }

    fn names(manifests: &[(CapsuleManifest, PathBuf)]) -> Vec<&str> {
        manifests
            .iter()
            .map(|(m, _)| m.package.name.as_str())
            .collect()
    }

    #[test]
    fn empty_graph() {
        let result = toposort_manifests(vec![]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn single_node() {
        let input = vec![manifest("a", &[])];
        let result = toposort_manifests(input).unwrap();
        assert_eq!(names(&result), vec!["a"]);
    }

    #[test]
    fn linear_chain() {
        // c depends on b, b depends on a => order: a, b, c
        let input = vec![
            manifest("c", &["b"]),
            manifest("a", &[]),
            manifest("b", &["a"]),
        ];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        assert!(
            n.iter().position(|&x| x == "a").unwrap() < n.iter().position(|&x| x == "b").unwrap()
        );
        assert!(
            n.iter().position(|&x| x == "b").unwrap() < n.iter().position(|&x| x == "c").unwrap()
        );
    }

    #[test]
    fn diamond_dependency() {
        // d depends on b and c, both depend on a => a first, d last
        let input = vec![
            manifest("d", &["b", "c"]),
            manifest("b", &["a"]),
            manifest("c", &["a"]),
            manifest("a", &[]),
        ];
        let result = toposort_manifests(input).unwrap();
        let n = names(&result);
        let pos = |name: &str| n.iter().position(|&x| x == name).unwrap();
        assert!(pos("a") < pos("b"));
        assert!(pos("a") < pos("c"));
        assert!(pos("b") < pos("d"));
        assert!(pos("c") < pos("d"));
    }

    #[test]
    fn cycle_detected() {
        let input = vec![manifest("a", &["b"]), manifest("b", &["a"])];
        let err = toposort_manifests(input).unwrap_err();
        assert!(err.cycle.contains(&"a".to_string()));
        assert!(err.cycle.contains(&"b".to_string()));
        assert!(err.to_string().contains("dependency cycle detected"));
    }

    #[test]
    fn three_node_cycle() {
        let input = vec![
            manifest("a", &["c"]),
            manifest("b", &["a"]),
            manifest("c", &["b"]),
        ];
        let err = toposort_manifests(input).unwrap_err();
        assert_eq!(err.cycle.len(), 3);
    }

    #[test]
    fn missing_dependency_succeeds() {
        // b depends on "missing" which isn't in the set
        let input = vec![manifest("a", &[]), manifest("b", &["missing"])];
        let result = toposort_manifests(input).unwrap();
        assert_eq!(names(&result).len(), 2);
    }

    #[test]
    fn no_dependencies_preserves_all() {
        let input = vec![manifest("x", &[]), manifest("y", &[]), manifest("z", &[])];
        let result = toposort_manifests(input).unwrap();
        assert_eq!(result.len(), 3);
    }
}
