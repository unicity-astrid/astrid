use astrid_core::SessionId;
use astrid_core::dirs::{AstridHome, WorkspaceDir};
use std::io;
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

/// RAII Guard for an active Git Worktree tied to an agent session.
///
/// This ensures that when the session finishes or the process exits normally,
/// the physical worktree directory is automatically deleted to save disk space,
/// but any uncommitted WIP changes are auto-committed to the session's branch
/// first to prevent data loss.
#[derive(Debug)]
pub(crate) struct ActiveWorktree {
    /// The physical path to the main repository.
    repo_path: PathBuf,
    /// The physical path to the temporary worktree.
    worktree_path: PathBuf,
    /// The name of the dedicated git branch for this session.
    branch_name: String,
}

impl ActiveWorktree {
    /// Creates a new active worktree for the given session.
    ///
    /// The worktree will be located at `~/.astrid/sessions/<workspace_id>/<session_id>`.
    ///
    /// # Errors
    ///
    /// Returns an error if git commands fail or directories cannot be resolved.
    pub(crate) fn new(
        workspace: &WorkspaceDir,
        home: &AstridHome,
        session_id: &SessionId,
    ) -> io::Result<Self> {
        let repo_path = workspace.root().to_path_buf();
        let workspace_id = workspace.workspace_id()?;

        let branch_name = format!("astrid-session-{}", session_id.0);
        let worktree_path = home
            .sessions_dir()
            .join(workspace_id.to_string())
            .join(session_id.0.to_string());

        // Ensure the parent sessions directory for this workspace exists.
        if let Some(parent) = worktree_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 1. Check if the branch already exists in the main repo.
        let branch_exists = Command::new("git")
            .current_dir(&repo_path)
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch_name}"),
            ])
            .status()?
            .success();

        // 2. Add the worktree
        let mut add_cmd = Command::new("git");
        add_cmd.current_dir(&repo_path).arg("worktree").arg("add");

        if branch_exists {
            // Branch exists, just check it out
            add_cmd.arg(&worktree_path).arg(&branch_name);
        } else {
            // Branch doesn't exist, create it (-b)
            add_cmd.arg("-b").arg(&branch_name).arg(&worktree_path);
        }

        let output = add_cmd.output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            error!("Failed to create git worktree: {}", err);
            return Err(io::Error::other(format!(
                "Failed to create git worktree: {err}"
            )));
        }

        info!("Created active worktree at {}", worktree_path.display());

        Ok(Self {
            repo_path,
            worktree_path,
            branch_name,
        })
    }

    /// Returns the physical path to the worktree.
    #[must_use]
    pub(crate) fn path(&self) -> &PathBuf {
        &self.worktree_path
    }

    /// Returns the name of the branch.
    #[must_use]
    pub(crate) fn branch(&self) -> &str {
        &self.branch_name
    }
}

impl Drop for ActiveWorktree {
    fn drop(&mut self) {
        debug!("Dropping ActiveWorktree for branch: {}", self.branch_name);

        // 1. Auto-commit any WIP changes before deleting the physical files.
        let add_status = Command::new("git")
            .current_dir(&self.worktree_path)
            .args(["add", "-A"])
            .status();

        if let Ok(status) = add_status
            && status.success()
        {
            // Check if there's actually anything staged to commit
            let diff_status = Command::new("git")
                .current_dir(&self.worktree_path)
                .args(["diff-index", "--quiet", "--cached", "HEAD"])
                .status();

            if let Ok(diff) = diff_status
                && !diff.success()
            {
                // There are changes, commit them
                let commit_status = Command::new("git")
                    .current_dir(&self.worktree_path)
                    .args(["commit", "-m", "[Astrid] WIP: Auto-saved session state"])
                    .output();

                if let Ok(commit) = commit_status {
                    if commit.status.success() {
                        info!("Auto-saved uncommitted work to branch {}", self.branch_name);
                    } else {
                        error!(
                            "Failed to auto-commit work in worktree: {}\n{}",
                            String::from_utf8_lossy(&commit.stdout),
                            String::from_utf8_lossy(&commit.stderr)
                        );
                    }
                }
            }
        }

        // 2. Remove the physical worktree to reclaim disk space.
        let remove_status = Command::new("git")
            .current_dir(&self.repo_path)
            .args([
                "worktree",
                "remove",
                "--force",
                &self.worktree_path.to_string_lossy(),
            ])
            .status();

        match remove_status {
            Ok(status) if status.success() => {
                info!(
                    "Cleanly removed physical worktree at {}",
                    self.worktree_path.display()
                );
            },
            Ok(_) => {
                error!(
                    "git worktree remove failed for {}",
                    self.worktree_path.display()
                );
            },
            Err(e) => {
                error!("Failed to execute git worktree remove: {}", e);
            },
        }
    }
}
