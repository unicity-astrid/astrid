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
pub struct ActiveWorktree {
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
    pub fn new(
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
    pub fn path(&self) -> &PathBuf {
        &self.worktree_path
    }

    /// Returns the name of the branch.
    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch_name
    }

    /// Performs Garbage Collection (GC) on orphaned worktrees.
    ///
    /// This should be called *only* during the daemon's initial boot sequence.
    /// Because the daemon starts with zero active sessions in memory, any directory
    /// found in `~/.astrid/sessions/` is guaranteed to be an orphaned worktree from
    /// a hard crash (e.g. power loss or SIGKILL) where the `Drop` handler didn't fire.
    ///
    /// It forcefully removes the physical directories and relies on the user or
    /// subsequent agent commands to run `git worktree prune` if needed, ensuring
    /// we instantly reclaim gigabytes of disk space safely.
    ///
    /// # Errors
    ///
    /// Returns an error if the sessions directory exists but cannot be read.
    pub fn cleanup_orphaned_worktrees(home: &AstridHome) -> io::Result<()> {
        let sessions_dir = home.sessions_dir();

        if !sessions_dir.exists() {
            return Ok(());
        }

        info!(
            "Scanning for orphaned worktrees in {}",
            sessions_dir.display()
        );

        let mut count: u64 = 0;
        let mut freed_bytes: u64 = 0;

        // Iterate through workspace_id directories
        if let Ok(workspace_entries) = std::fs::read_dir(&sessions_dir) {
            for ws_entry in workspace_entries.flatten() {
                if let Ok(ws_file_type) = ws_entry.file_type()
                    && ws_file_type.is_dir()
                {
                    // Iterate through session_id directories within the workspace
                    if let Ok(session_entries) = std::fs::read_dir(ws_entry.path()) {
                        for session_entry in session_entries.flatten() {
                            if let Ok(session_file_type) = session_entry.file_type()
                                && session_file_type.is_dir()
                            {
                                let worktree_path = session_entry.path();

                                // Optional: Calculate size before deletion to log reclaimed space
                                let size = match fs_extra::dir::get_size(&worktree_path) {
                                    Ok(s) => s,
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to calculate size of orphaned worktree {}: {}",
                                            worktree_path.display(),
                                            e
                                        );
                                        0
                                    },
                                };

                                // Forcefully remove the physical directory
                                if let Err(e) = std::fs::remove_dir_all(&worktree_path) {
                                    error!(
                                        "Failed to delete orphaned worktree at {}: {}",
                                        worktree_path.display(),
                                        e
                                    );
                                } else {
                                    count = count.saturating_add(1);
                                    freed_bytes = freed_bytes.saturating_add(size);
                                    debug!(
                                        "Cleaned up orphaned worktree: {}",
                                        worktree_path.display()
                                    );
                                }
                            }
                        }
                    }

                    // Try to clean up the workspace directory if it's now empty
                    let _ = std::fs::remove_dir(ws_entry.path());
                }
            }
        }

        if count > 0 {
            let mb = freed_bytes / 1_024 / 1_024;
            info!(
                "Boot GC complete: Removed {} orphaned worktrees, reclaiming {} MB of disk space.",
                count, mb
            );
        } else {
            debug!("No orphaned worktrees found.");
        }

        Ok(())
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
