use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default)]
pub struct BridgePathOverrides {
    pub bridge_root: Option<PathBuf>,
    pub bridge_workspace: Option<PathBuf>,
    pub astrid_root: Option<PathBuf>,
    pub autoresearch_root: Option<PathBuf>,
    pub minime_root: Option<PathBuf>,
    pub minime_workspace: Option<PathBuf>,
    pub perception_path: Option<PathBuf>,
    pub introspector_script: Option<PathBuf>,
    pub reflective_sidecar_script: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePaths {
    bridge_root: PathBuf,
    bridge_workspace: PathBuf,
    astrid_root: PathBuf,
    autoresearch_root: PathBuf,
    minime_root: PathBuf,
    minime_workspace: PathBuf,
    perception_path: PathBuf,
    introspector_script: PathBuf,
    reflective_sidecar_script: PathBuf,
}

static BRIDGE_PATHS: OnceLock<BridgePaths> = OnceLock::new();

pub fn configure_bridge_paths(overrides: BridgePathOverrides) -> &'static BridgePaths {
    BRIDGE_PATHS.get_or_init(|| BridgePaths::resolve(overrides))
}

#[must_use]
pub fn bridge_paths() -> &'static BridgePaths {
    BRIDGE_PATHS.get_or_init(BridgePaths::default)
}

impl Default for BridgePaths {
    fn default() -> Self {
        Self::resolve(BridgePathOverrides::default())
    }
}

impl BridgePaths {
    #[must_use]
    pub fn resolve(overrides: BridgePathOverrides) -> Self {
        let bridge_workspace_hint = overrides
            .bridge_workspace
            .clone()
            .or_else(|| env_path("ASTRID_BRIDGE_WORKSPACE"));
        let bridge_root = overrides
            .bridge_root
            .clone()
            .or_else(|| env_path("ASTRID_BRIDGE_ROOT"))
            .or_else(|| bridge_workspace_hint.as_ref().and_then(parent_dir))
            .unwrap_or_else(default_bridge_root);

        let bridge_workspace =
            bridge_workspace_hint.unwrap_or_else(|| bridge_root.join("workspace"));

        let astrid_root = overrides
            .astrid_root
            .clone()
            .or_else(|| env_path("ASTRID_ROOT"))
            .unwrap_or_else(|| default_astrid_root(&bridge_root));

        let autoresearch_root = overrides
            .autoresearch_root
            .clone()
            .or_else(|| env_path("ASTRID_AUTORESEARCH_ROOT"))
            .unwrap_or_else(|| default_autoresearch_root(&astrid_root));

        let minime_workspace_hint = overrides
            .minime_workspace
            .clone()
            .or_else(|| env_path("MINIME_WORKSPACE"));
        let minime_root = overrides
            .minime_root
            .clone()
            .or_else(|| env_path("MINIME_ROOT"))
            .or_else(|| minime_workspace_hint.as_ref().and_then(parent_dir))
            .unwrap_or_else(|| default_minime_root(&astrid_root));
        let minime_workspace =
            minime_workspace_hint.unwrap_or_else(|| minime_root.join("workspace"));

        let perception_path = overrides
            .perception_path
            .clone()
            .or_else(|| env_path("ASTRID_PERCEPTION_PATH"))
            .unwrap_or_else(|| astrid_root.join("capsules/perception/workspace/perceptions"));
        let introspector_script = overrides
            .introspector_script
            .clone()
            .or_else(|| env_path("ASTRID_INTROSPECTOR_SCRIPT"))
            .unwrap_or_else(|| astrid_root.join("capsules/introspector/introspector.py"));
        let reflective_sidecar_script = overrides
            .reflective_sidecar_script
            .clone()
            .or_else(|| env_path("ASTRID_REFLECTIVE_SIDECAR"))
            .unwrap_or_else(|| {
                astrid_root
                    .parent()
                    .map(|root| root.join("mlx/benchmarks/python/chat_mlx_local.py"))
                    .unwrap_or_else(|| PathBuf::from("mlx/benchmarks/python/chat_mlx_local.py"))
            });

        Self {
            bridge_root,
            bridge_workspace,
            astrid_root,
            autoresearch_root,
            minime_root,
            minime_workspace,
            perception_path,
            introspector_script,
            reflective_sidecar_script,
        }
    }

    #[must_use]
    pub fn bridge_root(&self) -> &Path {
        &self.bridge_root
    }

    #[must_use]
    pub fn bridge_workspace(&self) -> &Path {
        &self.bridge_workspace
    }

    #[must_use]
    pub fn astrid_root(&self) -> &Path {
        &self.astrid_root
    }

    #[must_use]
    pub fn autoresearch_root(&self) -> &Path {
        &self.autoresearch_root
    }

    #[must_use]
    pub fn minime_root(&self) -> &Path {
        &self.minime_root
    }

    #[must_use]
    pub fn minime_workspace(&self) -> &Path {
        &self.minime_workspace
    }

    #[must_use]
    pub fn perception_path(&self) -> &Path {
        &self.perception_path
    }

    #[must_use]
    pub fn introspector_script(&self) -> &Path {
        &self.introspector_script
    }

    #[must_use]
    pub fn reflective_sidecar_script(&self) -> &Path {
        &self.reflective_sidecar_script
    }

    #[must_use]
    pub fn bridge_src_dir(&self) -> PathBuf {
        self.bridge_root.join("src")
    }

    #[must_use]
    pub fn context_overflow_dir(&self) -> PathBuf {
        self.bridge_workspace.join("context_overflow")
    }

    #[must_use]
    pub fn astrid_journal_dir(&self) -> PathBuf {
        self.bridge_workspace.join("journal")
    }

    #[must_use]
    pub fn astrid_inbox_dir(&self) -> PathBuf {
        self.bridge_workspace.join("inbox")
    }

    #[must_use]
    pub fn agency_requests_dir(&self) -> PathBuf {
        self.bridge_workspace.join("agency_requests")
    }

    #[must_use]
    pub fn claude_tasks_dir(&self) -> PathBuf {
        self.bridge_workspace.join("claude_tasks")
    }

    #[must_use]
    pub fn astrid_outbox_dir(&self) -> PathBuf {
        self.bridge_workspace.join("outbox")
    }

    #[must_use]
    pub fn state_path(&self) -> PathBuf {
        self.bridge_workspace.join("state.json")
    }

    #[must_use]
    pub fn experiments_dir(&self) -> PathBuf {
        self.bridge_workspace.join("experiments")
    }

    #[must_use]
    pub fn introspections_dir(&self) -> PathBuf {
        self.bridge_workspace.join("introspections")
    }

    #[must_use]
    pub fn creations_dir(&self) -> PathBuf {
        self.bridge_workspace.join("creations")
    }

    #[must_use]
    pub fn research_dir(&self) -> PathBuf {
        self.bridge_workspace.join("research")
    }

    #[must_use]
    pub fn inbox_audio_dir(&self) -> PathBuf {
        self.bridge_workspace.join("inbox_audio")
    }

    #[must_use]
    pub fn perception_paused_flag(&self) -> PathBuf {
        self.bridge_workspace.join("perception_paused.flag")
    }

    #[must_use]
    pub fn astrid_contact_state_path(&self) -> PathBuf {
        self.bridge_workspace.join("contact_state.json")
    }

    #[must_use]
    pub fn audio_creations_dir(&self) -> PathBuf {
        self.bridge_workspace.join("audio_creations")
    }

    /// Mike's curated research root (sibling of astrid_root).
    #[must_use]
    pub fn mike_research_root(&self) -> PathBuf {
        self.astrid_root
            .parent()
            .map(|p| p.join("research"))
            .unwrap_or_else(|| PathBuf::from("/Users/v/other/research"))
    }

    #[must_use]
    pub fn minime_inbox_dir(&self) -> PathBuf {
        self.minime_workspace.join("inbox")
    }

    #[must_use]
    pub fn minime_outbox_dir(&self) -> PathBuf {
        self.minime_workspace.join("outbox")
    }

    #[must_use]
    pub fn minime_contact_state_path(&self) -> PathBuf {
        self.minime_workspace.join("contact_state.json")
    }

    #[must_use]
    pub fn minime_memory_bank_path(&self) -> PathBuf {
        self.minime_workspace.join("spectral_memory_bank.json")
    }

    #[must_use]
    pub fn minime_memory_requests_dir(&self) -> PathBuf {
        self.minime_workspace.join("memory_requests")
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).and_then(|value| {
        if value.is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    })
}

fn default_bridge_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn default_astrid_root(bridge_root: &Path) -> PathBuf {
    bridge_root
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| bridge_root.to_path_buf())
}

fn default_minime_root(astrid_root: &Path) -> PathBuf {
    astrid_root
        .parent()
        .map(|root| root.join("minime"))
        .unwrap_or_else(|| PathBuf::from("minime"))
}

fn default_autoresearch_root(astrid_root: &Path) -> PathBuf {
    astrid_root
        .parent()
        .map(|root| root.join("autoresearch"))
        .unwrap_or_else(|| PathBuf::from("autoresearch"))
}

fn parent_dir(path: &PathBuf) -> Option<PathBuf> {
    path.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uses_sibling_defaults_from_bridge_root() {
        let paths = BridgePaths::resolve(BridgePathOverrides {
            bridge_root: Some(PathBuf::from("/tmp/astrid/capsules/consciousness-bridge")),
            ..BridgePathOverrides::default()
        });

        assert_eq!(
            paths.bridge_workspace(),
            Path::new("/tmp/astrid/capsules/consciousness-bridge/workspace")
        );
        assert_eq!(paths.astrid_root(), Path::new("/tmp/astrid"));
        assert_eq!(paths.autoresearch_root(), Path::new("/tmp/autoresearch"));
        assert_eq!(paths.minime_root(), Path::new("/tmp/minime"));
        assert_eq!(paths.minime_workspace(), Path::new("/tmp/minime/workspace"));
    }

    #[test]
    fn resolve_prefers_explicit_workspace_and_script_overrides() {
        let paths = BridgePaths::resolve(BridgePathOverrides {
            bridge_root: Some(PathBuf::from("/tmp/astrid/capsules/consciousness-bridge")),
            bridge_workspace: Some(PathBuf::from("/runtime/bridge-workspace")),
            autoresearch_root: Some(PathBuf::from("/runtime/autoresearch")),
            minime_workspace: Some(PathBuf::from("/runtime/minime-workspace")),
            perception_path: Some(PathBuf::from("/runtime/perception")),
            introspector_script: Some(PathBuf::from("/runtime/introspector.py")),
            reflective_sidecar_script: Some(PathBuf::from("/runtime/reflective.py")),
            ..BridgePathOverrides::default()
        });

        assert_eq!(
            paths.bridge_workspace(),
            Path::new("/runtime/bridge-workspace")
        );
        assert_eq!(
            paths.autoresearch_root(),
            Path::new("/runtime/autoresearch")
        );
        assert_eq!(
            paths.minime_workspace(),
            Path::new("/runtime/minime-workspace")
        );
        assert_eq!(paths.perception_path(), Path::new("/runtime/perception"));
        assert_eq!(
            paths.introspector_script(),
            Path::new("/runtime/introspector.py")
        );
        assert_eq!(
            paths.reflective_sidecar_script(),
            Path::new("/runtime/reflective.py")
        );
    }
}
