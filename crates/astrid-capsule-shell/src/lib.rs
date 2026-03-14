#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! Shell execution tools capsule for Astrid OS.
//!
//! Provides the `run_shell_command` tool to agents, wrapping executions
//! securely in the host-level Escape Hatch (Seatbelt/bwrap).
//!
//! Each command is parsed to extract an approval action: the program name
//! plus subcommand tokens for known multi-command tools. Approved commands
//! create allowances at that granularity (e.g. "git push" approves all
//! `git push` variants, "docker compose up" approves all
//! `docker compose up` variants). Unknown programs use program-name-only
//! to avoid leaking positional arguments into allowance patterns.

use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use serde::Deserialize;

/// The main entry point for the Shell Tools capsule.
#[derive(Default)]
pub struct ShellTools;

/// Input arguments for the `run_shell_command` tool.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct RunShellArgs {
    /// The exact bash command to execute.
    pub command: String,
}

/// Input arguments for the `spawn_background_process` tool.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct SpawnBackgroundArgs {
    /// The exact bash command to run in the background.
    pub command: String,
}

/// Input arguments for the `read_process_logs` tool.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ReadProcessLogsArgs {
    /// The process handle ID returned by `spawn_background_process`.
    pub id: u64,
}

/// Input arguments for the `kill_process` tool.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct KillProcessArgs {
    /// The process handle ID returned by `spawn_background_process`.
    pub id: u64,
}

/// Determine the safe subcommand depth for a given program.
fn get_safe_depth(tokens: &[&str]) -> usize {
    if tokens.is_empty() {
        return 0;
    }

    // Depth 3 (Sub-sub-commands)
    if tokens.len() >= 2 {
        let prefix2 = format!("{} {}", tokens[0], tokens[1]);
        if matches!(
            prefix2.as_str(),
            "docker compose"
                | "aws s3"
                | "gcloud s3"
                | "kubectl config"
                | "git remote"
                | "npm run"
                | "yarn run"
                | "pnpm run"
                | "bun run"
                | "deno run"
        ) {
            return 3;
        }
    }

    // Depth 2 (Standard CLI subcommands)
    if matches!(
        tokens[0],
        "git"
            | "docker"
            | "kubectl"
            | "cargo"
            | "npm"
            | "npx"
            | "yarn"
            | "pnpm"
            | "pip"
            | "pip3"
            | "poetry"
            | "apt"
            | "apt-get"
            | "brew"
            | "dnf"
            | "yum"
            | "pacman"
            | "snap"
            | "flatpak"
            | "helm"
            | "terraform"
            | "ansible"
            | "vagrant"
            | "make"
            | "cmake"
            | "go"
            | "rustup"
            | "bun"
            | "deno"
            | "uv"
            | "nix"
            | "podman"
            | "gh"
            | "fly"
            | "flyctl"
            | "stripe"
            | "supabase"
            | "vercel"
            | "wrangler"
            | "firebase"
    ) {
        return 2;
    }

    // Default fallback: 0 means exact match only (no subcommands extracted).
    0
}

/// System-critical paths that must never be targets of destructive commands
/// from an AI agent. Relative paths and workspace-local paths are fine.
const PROTECTED_PATHS: &[&str] = &[
    "/",
    "/*",
    "/etc",
    "/usr",
    "/bin",
    "/sbin",
    "/var",
    "/boot",
    "/lib",
    "/lib64",
    "/opt",
    "/root",
    "/home",
    "/proc",
    "/sys",
    "/dev",
    // macOS
    "/System",
    "/Library",
    "/Applications",
    "/Users",
];

/// Commands that are unconditionally blocked regardless of arguments.
const BLOCKED_COMMANDS: &[&str] = &[
    "mkfs",
    "mkfs.ext4",
    "mkfs.xfs",
    "mkfs.btrfs",
    "mkfs.vfat",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "init",
];

/// Shell operators that chain multiple commands. We split on these to check
/// each sub-command independently for catastrophic patterns.
const SHELL_CHAIN_OPERATORS: &[&str] = &["&&", "||", ";", "|"];

/// Check if a command is catastrophic and should be hard-denied before
/// reaching the approval prompt.
///
/// Splits on shell chaining operators (`&&`, `||`, `;`, `|`) and checks
/// each sub-command independently. `ls && rm -rf /` is caught even though
/// the first command is benign.
///
/// Returns `Some(reason)` if the command is blocked, `None` if safe to proceed.
fn check_catastrophic(command: &str) -> Option<&'static str> {
    // Fork bomb patterns checked on the raw string before splitting
    if command.contains(":(){ :|:&") || command.contains(":(){:|:&") {
        return Some("Fork bombs are blocked.");
    }

    // Split on shell chaining operators and check each sub-command
    for subcmd in split_on_shell_operators(command) {
        if let Some(reason) = check_single_command_catastrophic(subcmd.trim()) {
            return Some(reason);
        }
    }

    None
}

/// Split a command string on shell chaining operators.
fn split_on_shell_operators(command: &str) -> Vec<&str> {
    // Replace multi-char operators with a single sentinel, then split.
    // We need to handle &&, ||, ;, | - but | is a substring of ||,
    // so we process longer operators first.
    let mut result = vec![command];
    for op in SHELL_CHAIN_OPERATORS {
        let mut next = Vec::new();
        for segment in result {
            for part in segment.split(op) {
                next.push(part);
            }
        }
        result = next;
    }
    result
}

/// Check a single command (no shell operators) for catastrophic patterns.
fn check_single_command_catastrophic(command: &str) -> Option<&'static str> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let program = tokens[0];

    // Unconditionally blocked commands
    if BLOCKED_COMMANDS.contains(&program) {
        return Some(
            "This command is blocked for safety. It can cause irreversible system damage.",
        );
    }

    // dd targeting block devices
    if program == "dd" && tokens.iter().any(|t| t.starts_with("of=/dev/")) {
        return Some("Writing directly to block devices via dd is blocked.");
    }

    // chmod/chown -R on system paths
    if matches!(program, "chmod" | "chown")
        && tokens.iter().any(|t| *t == "-R" || t.starts_with("-R"))
        && tokens.iter().any(|t| is_protected_path(t))
    {
        return Some("Recursive permission changes on system paths are blocked.");
    }

    // rm targeting protected paths (rm -rf /tmp/foo is fine, rm -rf / is not)
    if program == "rm" {
        for token in &tokens[1..] {
            if token.starts_with('-') {
                continue;
            }
            if is_protected_path(token) {
                return Some(
                    "Removing system-critical paths is blocked. \
                     Workspace-relative and /tmp paths are allowed.",
                );
            }
        }
    }

    None
}

/// Check if a path is a protected system path.
///
/// Matches exact paths and paths directly under protected roots.
/// Does NOT block subdirectories of allowed workspace paths.
fn is_protected_path(path: &str) -> bool {
    // Exact match against protected paths
    if PROTECTED_PATHS.contains(&path) {
        return true;
    }
    // Home directory shortcuts
    if path == "~" || path == "$HOME" || path == "${HOME}" {
        return true;
    }
    // One level under protected roots: /usr/local is protected, /tmp/build is not
    for root in PROTECTED_PATHS {
        if *root == "/" || *root == "/*" {
            continue;
        }
        if let Some(rest) = path.strip_prefix(root) {
            // /usr -> blocked, /usr/ -> blocked, /usr/local -> blocked
            // But /usrfoo -> not blocked (not a real subdirectory)
            if rest.is_empty() || rest.starts_with('/') {
                return true;
            }
        }
    }
    false
}

/// Extract the approval action from a shell command string.
///
/// Uses an exhaustive whitelist to determine how many subcommand tokens
/// are safe to include in the allowance pattern. For unknown programs,
/// returns the exact full command string.
///
/// Returning the full string for unknown programs is a critical security
/// boundary. If `rm -rf /tmp/foo` returned just `rm`, the session allowance
/// would be `rm *`, which would allow a malicious capsule to execute `rm -rf /`
/// without prompting. By returning the full string, the allowance becomes
/// safely scoped to `rm -rf /tmp/foo *`.
///
/// ```text
/// git push --force origin main      -> "git push"
/// docker compose up -d              -> "docker compose up"
/// kubectl config set-context --cur  -> "kubectl config set-context"
/// ls -la /tmp                       -> "ls -la /tmp"
/// cargo build --release             -> "cargo build"
/// python -c "code"                  -> "python -c \"code\""
/// cat /etc/passwd                   -> "cat /etc/passwd"
/// rm -rf /tmp/foo                   -> "rm -rf /tmp/foo"
/// rm /tmp/foo                       -> "rm /tmp/foo"
/// ```
fn extract_action(command: &str) -> String {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    if tokens.is_empty() || tokens[0].starts_with('-') {
        return String::new();
    }

    let depth = get_safe_depth(&tokens);
    if depth == 0 {
        // SECURITY: Unknown programs must use exact-match fallback to prevent
        // generating dangerously broad `program *` glob allowances.
        return command.to_string();
    }

    let mut parts = Vec::new();
    for token in tokens {
        if token.starts_with('-') || parts.len() == depth {
            break;
        }
        parts.push(token);
    }
    parts.join(" ")
}

#[capsule]
impl ShellTools {
    /// Executes a given shell command via the host sandbox escape hatch.
    ///
    /// Before execution, extracts the approval action (consecutive non-flag
    /// tokens, up to 3 deep), then requests human approval. If denied,
    /// returns an error without executing.
    #[astrid::tool("run_shell_command")]
    pub fn run_shell_command(&self, args: RunShellArgs) -> Result<String, SysError> {
        let trimmed = args.command.trim();
        if trimmed.is_empty() {
            return Err(SysError::ApiError("Command cannot be empty".into()));
        }

        // Hard-deny catastrophic commands before they reach the approval prompt.
        if let Some(reason) = check_catastrophic(trimmed) {
            return Err(SysError::ApiError(format!("Command blocked: {reason}")));
        }

        let action = extract_action(trimmed);

        // Request approval - blocks until the user responds or timeout.
        let result = approval::request(&action, trimmed, "high")?;
        if !result.approved {
            return Err(SysError::ApiError(format!(
                "Command '{trimmed}' was not approved by user",
            )));
        }

        // Spawn the command via the SDK Process Airlock.
        // The core OS enforces the Capability and wraps it in bwrap/Seatbelt.
        let result = process::spawn("bash", &["-c", trimmed])?;

        // If the command fails, we return the stderr as an API error so the LLM knows it failed.
        if result.exit_code != 0 {
            return Err(SysError::ApiError(format!(
                "Command failed with exit code {}: {}",
                result.exit_code, result.stderr
            )));
        }

        // Return stdout back to the LLM agent
        Ok(result.stdout)
    }

    /// Spawns a background process via the host sandbox escape hatch.
    ///
    /// Applies the same safety checks as `run_shell_command`: catastrophic
    /// command blocking, action extraction, and human approval. Returns a
    /// process handle ID that can be used with `read_process_logs` and
    /// `kill_process`.
    #[astrid::tool("spawn_background_process")]
    pub fn spawn_background_process(&self, args: SpawnBackgroundArgs) -> Result<String, SysError> {
        let trimmed = args.command.trim();
        if trimmed.is_empty() {
            return Err(SysError::ApiError("Command cannot be empty".into()));
        }

        if let Some(reason) = check_catastrophic(trimmed) {
            return Err(SysError::ApiError(format!("Command blocked: {reason}")));
        }

        let action = extract_action(trimmed);

        let result = approval::request(&action, trimmed, "high")?;
        if !result.approved {
            return Err(SysError::ApiError(format!(
                "Command '{trimmed}' was not approved by user",
            )));
        }

        let handle = process::spawn_background("bash", &["-c", trimmed])?;
        Ok(format!(
            "Background process started with id: {}. Use read_process_logs to check output and kill_process to stop it.",
            handle.id
        ))
    }

    /// Reads buffered stdout/stderr from a background process.
    ///
    /// Each call returns only the new output since the last read. Also
    /// reports whether the process is still running and its exit code
    /// if it has terminated.
    #[astrid::tool("read_process_logs")]
    pub fn read_process_logs(&self, args: ReadProcessLogsArgs) -> Result<String, SysError> {
        let logs = process::read_logs(args.id)?;

        let status = if logs.running {
            "running".to_string()
        } else if let Some(code) = logs.exit_code {
            format!("exited with code {code}")
        } else {
            "exited (unknown code)".to_string()
        };

        let mut output = format!("Process {} status: {status}\n", args.id);
        if !logs.stdout.is_empty() {
            output.push_str(&format!("--- stdout ---\n{}\n", logs.stdout));
        }
        if !logs.stderr.is_empty() {
            output.push_str(&format!("--- stderr ---\n{}\n", logs.stderr));
        }
        if logs.stdout.is_empty() && logs.stderr.is_empty() {
            output.push_str("(no new output)\n");
        }

        Ok(output)
    }

    /// Terminates a background process and returns any remaining output.
    ///
    /// No additional approval is required since the process was already
    /// approved at spawn time.
    #[astrid::tool("kill_process")]
    pub fn kill_process(&self, args: KillProcessArgs) -> Result<String, SysError> {
        let result = process::kill(args.id)?;

        let exit_info = match result.exit_code {
            Some(code) => format!("exit code {code}"),
            None => "unknown exit code".to_string(),
        };

        let mut output = format!("Process {} killed ({exit_info}).\n", args.id);
        if !result.stdout.is_empty() {
            output.push_str(&format!("--- final stdout ---\n{}\n", result.stdout));
        }
        if !result.stderr.is_empty() {
            output.push_str(&format!("--- final stderr ---\n{}\n", result.stderr));
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Subcommand extraction (depth 1) --

    #[test]
    fn action_git_push() {
        assert_eq!(extract_action("git push --force origin main"), "git push");
    }

    #[test]
    fn action_git_status() {
        assert_eq!(extract_action("git status"), "git status");
    }

    #[test]
    fn action_cargo_build() {
        assert_eq!(extract_action("cargo build --release"), "cargo build");
    }

    // -- Sub-sub-command extraction (depth 2) --

    #[test]
    fn action_docker_compose_up() {
        assert_eq!(extract_action("docker compose up -d"), "docker compose up");
    }

    #[test]
    fn action_kubectl_config_set_context() {
        assert_eq!(
            extract_action("kubectl config set-context --current"),
            "kubectl config set-context"
        );
    }

    #[test]
    fn action_git_remote_add() {
        assert_eq!(
            extract_action("git remote add origin https://example.com"),
            "git remote add"
        );
    }

    // -- Depth cap (stops at MAX_SUBCOMMAND_DEPTH + 1 tokens for known programs) --

    #[test]
    fn action_depth_cap_known_program() {
        // npm is in the whitelist; 3rd non-flag token is NOT included
        assert_eq!(extract_action("npm run build dist"), "npm run build");
    }

    // -- Unknown programs return full command (exact-match safety) --

    #[test]
    fn action_unknown_program_returns_full_command() {
        // Unknown programs return the full command string to prevent
        // dangerously broad "program *" session allowances.
        assert_eq!(extract_action("cat /etc/passwd"), "cat /etc/passwd");
        assert_eq!(extract_action("rm /tmp/foo"), "rm /tmp/foo");
        assert_eq!(extract_action("rm -rf /tmp/foo"), "rm -rf /tmp/foo");
        assert_eq!(extract_action("a b c d e"), "a b c d e");
    }

    // -- Flag stops extraction (known programs only) --

    #[test]
    fn action_known_program_flag_stops() {
        // Flags stop subcommand extraction for whitelisted programs
        assert_eq!(extract_action("cargo build --release"), "cargo build");
        assert_eq!(extract_action("git push --force origin main"), "git push");
    }

    #[test]
    fn action_unknown_program_with_flags_returns_full() {
        // Unknown programs always return full command, flags included
        assert_eq!(extract_action("ls -la /tmp"), "ls -la /tmp");
        assert_eq!(
            extract_action("python -c 'print(1)'"),
            "python -c 'print(1)'"
        );
        assert_eq!(
            extract_action("my-tool --flag value"),
            "my-tool --flag value"
        );
    }

    // -- Edge cases --

    #[test]
    fn action_empty() {
        assert_eq!(extract_action(""), "");
    }

    #[test]
    fn action_single_word() {
        assert_eq!(extract_action("git"), "git");
    }

    #[test]
    fn action_only_flags() {
        assert_eq!(extract_action("--help"), "");
    }

    // -----------------------------------------------------------------------
    // Catastrophic command blocking
    // -----------------------------------------------------------------------

    #[test]
    fn catastrophic_rm_root() {
        assert!(check_catastrophic("rm -rf /").is_some());
        assert!(check_catastrophic("rm -rf /*").is_some());
        assert!(check_catastrophic("rm /").is_some());
    }

    #[test]
    fn catastrophic_rm_system_paths() {
        assert!(check_catastrophic("rm -rf /etc").is_some());
        assert!(check_catastrophic("rm -rf /usr").is_some());
        assert!(check_catastrophic("rm -rf /usr/local").is_some());
        assert!(check_catastrophic("rm -rf /home").is_some());
        assert!(check_catastrophic("rm -rf ~").is_some());
        assert!(check_catastrophic("rm -rf $HOME").is_some());
    }

    #[test]
    fn catastrophic_rm_safe_paths_allowed() {
        // Workspace-relative and /tmp paths are fine
        assert!(check_catastrophic("rm -rf ./build").is_none());
        assert!(check_catastrophic("rm -rf /tmp/build").is_none());
        assert!(check_catastrophic("rm -rf target/debug").is_none());
        assert!(check_catastrophic("rm foo.txt").is_none());
    }

    #[test]
    fn catastrophic_blocked_commands() {
        assert!(check_catastrophic("mkfs /dev/sda1").is_some());
        assert!(check_catastrophic("mkfs.ext4 /dev/sda1").is_some());
        assert!(check_catastrophic("shutdown -h now").is_some());
        assert!(check_catastrophic("reboot").is_some());
        assert!(check_catastrophic("halt").is_some());
        assert!(check_catastrophic("init 0").is_some());
    }

    #[test]
    fn catastrophic_dd_block_device() {
        assert!(check_catastrophic("dd if=/dev/zero of=/dev/sda").is_some());
        assert!(check_catastrophic("dd if=/dev/zero of=/dev/disk0").is_some());
        // dd to a file is fine
        assert!(check_catastrophic("dd if=/dev/zero of=./test.img bs=1M count=10").is_none());
    }

    #[test]
    fn catastrophic_chmod_system() {
        assert!(check_catastrophic("chmod -R 777 /").is_some());
        assert!(check_catastrophic("chown -R root:root /usr").is_some());
        // chmod on workspace files is fine
        assert!(check_catastrophic("chmod -R 755 ./dist").is_none());
        assert!(check_catastrophic("chmod 644 foo.txt").is_none());
    }

    #[test]
    fn catastrophic_fork_bomb() {
        assert!(check_catastrophic(":(){ :|:& };:").is_some());
    }

    #[test]
    fn catastrophic_chained_commands() {
        // Dangerous command hidden after benign one
        assert!(check_catastrophic("ls && rm -rf /").is_some());
        assert!(check_catastrophic("echo hello; shutdown -h now").is_some());
        assert!(check_catastrophic("cat /tmp/f || rm -rf /usr").is_some());
        assert!(check_catastrophic("ls | dd if=/dev/zero of=/dev/sda").is_some());
        // Both safe - should pass
        assert!(check_catastrophic("ls && echo done").is_none());
        assert!(check_catastrophic("cat foo; echo bar").is_none());
    }

    #[test]
    fn catastrophic_normal_commands_pass() {
        assert!(check_catastrophic("git push origin main").is_none());
        assert!(check_catastrophic("cargo build --release").is_none());
        assert!(check_catastrophic("ls -la").is_none());
        assert!(check_catastrophic("cat /etc/hosts").is_none());
    }
}
