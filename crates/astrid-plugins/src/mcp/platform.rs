//! Platform-specific logic (sandboxing, resource limits) for MCP plugins.

#![allow(unsafe_code)]

#[cfg(target_os = "linux")]
pub(crate) struct PreparedLandlockRules {
    /// Pre-opened `(PathFd, read, write)` tuples.
    rules: Vec<(landlock::PathFd, bool, bool)>,
}

/// Phase 1 (parent process): open file descriptors and compute access flags.
///
/// This runs before `fork()`, so heap allocation and filesystem access are
/// safe. Paths that don't exist are silently skipped.
#[cfg(target_os = "linux")]
pub(crate) fn prepare_landlock_rules(rules: &[crate::sandbox::LandlockPathRule]) -> PreparedLandlockRules {
    use landlock::PathFd;

    let mut prepared = Vec::with_capacity(rules.len());

    for rule in rules {
        if !rule.read && !rule.write {
            continue;
        }

        // Open the path FD now (heap allocation happens here, safely)
        if let Ok(fd) = PathFd::new(&rule.path) {
            prepared.push((fd, rule.read, rule.write));
        }
    }

    PreparedLandlockRules { rules: prepared }
}

/// Phase 2 (child process, inside `pre_exec`): create ruleset and enforce.
///
/// Only Landlock syscalls are invoked here — no heap allocation, no
/// filesystem access. All file descriptors were pre-opened in phase 1.
#[cfg(target_os = "linux")]
pub(crate) fn enforce_landlock_rules(prepared: PreparedLandlockRules) -> Result<(), String> {
    use landlock::{
        ABI, Access, AccessFs, CompatLevel, Compatible, PathBeneath, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetStatus,
    };

    let abi = ABI::V5;

    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(abi))
        .map_err(|e| format!("failed to create Landlock ruleset: {e}"))?
        .create()
        .map_err(|e| format!("failed to create Landlock ruleset: {e}"))?;

    for (fd, read, write) in prepared.rules {
        let access = match (read, write) {
            (true, true) => AccessFs::from_all(abi),
            (true, false) => AccessFs::from_read(abi),
            (false, true) => AccessFs::from_write(abi),
            (false, false) => continue,
        };
        let path_beneath = PathBeneath::new(fd, access);
        ruleset = ruleset
            .add_rule(path_beneath)
            .map_err(|e| format!("failed to add Landlock rule: {e}"))?;
    }

    let status = ruleset
        .restrict_self()
        .map_err(|e| format!("failed to enforce Landlock ruleset: {e}"))?;

    match status.ruleset {
        RulesetStatus::FullyEnforced
        | RulesetStatus::PartiallyEnforced
        | RulesetStatus::NotEnforced => {
            // NotEnforced: kernel doesn't support Landlock — not a fatal error
        },
    }

    Ok(())
}

/// Apply resource limits via `setrlimit` inside a `pre_exec` closure.
///
/// Uses only async-signal-safe operations: `setrlimit` is a direct syscall,
/// and `Error::last_os_error()` reads `errno` without heap allocation.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub(crate) fn apply_resource_limits(limits: &crate::sandbox::ResourceLimits) -> Result<(), std::io::Error> {
    // RLIMIT_NPROC — max processes/threads (per-UID, not per-process)
    let nproc = libc::rlimit {
        rlim_cur: limits.max_processes,
        rlim_max: limits.max_processes,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &raw const nproc) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // RLIMIT_AS — max virtual address space
    let address_space = libc::rlimit {
        rlim_cur: limits.max_memory_bytes,
        rlim_max: limits.max_memory_bytes,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &raw const address_space) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // RLIMIT_NOFILE — max open file descriptors
    let nofile = libc::rlimit {
        rlim_cur: limits.max_open_files,
        rlim_max: limits.max_open_files,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const nofile) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}
