use crate::{VfsError, VfsResult};
use std::path::{Component, Path, PathBuf};

/// Lexically resolves a relative path against a secure base root.
/// Returns an error if the relative path attempts to traverse `..` above the base.
/// Does NOT touch the filesystem; purely computational (protects against symlink/hardlink bypass).
///
/// # Errors
///
/// Returns `VfsError::SandboxViolation` if the `request_path` attempts to traverse outside
/// the `base_root` using `..` or if it is an absolute path.
pub fn resolve_path(base_root: &Path, request_path: &str) -> VfsResult<PathBuf> {
    let req = Path::new(request_path);

    if req.is_absolute() {
        return Err(VfsError::SandboxViolation(
            "Absolute paths are not allowed in the VFS sandbox".into(),
        ));
    }

    let mut resolved = base_root.to_path_buf();

    for component in req.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                return Err(VfsError::SandboxViolation(
                    "Prefix or root components are not allowed".into(),
                ));
            },
            Component::CurDir => {}, // ignore `.`
            Component::ParentDir => {
                // If popping the parent drops us below the base root, it's a traversal attack.
                if resolved == base_root {
                    return Err(VfsError::SandboxViolation(
                        "Attempted to traverse above sandbox root".into(),
                    ));
                }
                resolved.pop();
            },
            Component::Normal(p) => {
                resolved.push(p);
            },
        }
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_path() {
        let base = Path::new("/var/sandbox");
        let res = resolve_path(base, "src/main.rs").unwrap();
        assert_eq!(res, Path::new("/var/sandbox/src/main.rs"));
    }

    #[test]
    fn test_traversal_blocked() {
        let base = Path::new("/var/sandbox");
        let res = resolve_path(base, "src/../../etc/passwd");
        assert!(matches!(res, Err(VfsError::SandboxViolation(_))));
    }

    #[test]
    fn test_absolute_blocked() {
        let base = Path::new("/var/sandbox");
        let res = resolve_path(base, "/etc/passwd");
        assert!(matches!(res, Err(VfsError::SandboxViolation(_))));
    }
}
