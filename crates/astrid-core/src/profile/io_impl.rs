//! On-disk IO for [`PrincipalProfile`]: path resolution, load, atomic save.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{PrincipalProfile, ProfileError, ProfileResult};
use crate::dirs::PrincipalHome;

impl PrincipalProfile {
    /// Canonical on-disk path for a principal's profile:
    /// `{home}/.config/profile.toml`.
    #[must_use]
    pub fn path_for(home: &PrincipalHome) -> PathBuf {
        home.config_dir().join("profile.toml")
    }

    /// Load the profile for `home`, falling back to [`Self::default`] if
    /// the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError::Io`] on IO failure other than `NotFound`,
    /// [`ProfileError::Parse`] on malformed or unknown-field TOML, and
    /// [`ProfileError::Invalid`] on semantic validation failure.
    pub fn load(home: &PrincipalHome) -> ProfileResult<Self> {
        Self::load_from_path(&Self::path_for(home))
    }

    /// Load a profile from an explicit path. Exposed for tests and tools
    /// that don't own a [`PrincipalHome`].
    ///
    /// # Errors
    ///
    /// See [`Self::load`].
    pub fn load_from_path(path: &Path) -> ProfileResult<Self> {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::default());
            },
            Err(e) => return Err(ProfileError::Io(e)),
        };
        let profile: Self = toml::from_str(&content)?;
        profile.validate()?;
        Ok(profile)
    }

    /// Save the profile to `home`, creating `.config/` if needed.
    ///
    /// The write is atomic on Unix: a temp file is written with mode
    /// `0o600` and then `rename`d over the target. A failed rename leaves
    /// the original file untouched.
    ///
    /// # Errors
    ///
    /// Validates before writing. Returns [`ProfileError::Invalid`] if the
    /// in-memory profile is malformed, [`ProfileError::Serialize`] on
    /// serialization failure, and [`ProfileError::Io`] on filesystem
    /// failure.
    pub fn save(&self, home: &PrincipalHome) -> ProfileResult<()> {
        self.save_to_path(&Self::path_for(home))
    }

    /// Save the profile to an explicit path. See [`Self::save`].
    ///
    /// # Errors
    ///
    /// See [`Self::save`].
    pub fn save_to_path(&self, path: &Path) -> ProfileResult<()> {
        self.validate()?;
        let content = toml::to_string_pretty(self)?;
        write_atomic(path, content.as_bytes())
    }
}

/// Per-process monotonic counter disambiguating concurrent tmp filenames.
/// PID alone is not enough — two threads in the same daemon calling `save`
/// would race on the same tmp path and corrupt each other's writes.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn write_atomic(path: &Path, data: &[u8]) -> ProfileResult<()> {
    let parent = path.parent().ok_or_else(|| {
        ProfileError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "profile path has no parent directory",
        ))
    })?;
    fs::create_dir_all(parent)?;

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        // Same-filesystem temp sibling so `rename` is atomic. PID + monotonic
        // counter → unique per call across threads within a process and
        // across processes sharing the directory.
        let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = path.with_extension(format!("toml.tmp.{}.{seq}", std::process::id()));
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);

        if let Err(e) = fs::rename(&tmp_path, path) {
            // Rename failed; don't leave a secret-adjacent temp file.
            let _ = fs::remove_file(&tmp_path);
            return Err(ProfileError::Io(e));
        }
    }

    // Non-Unix fallback: no atomic rename, no explicit permissions.
    // Astrid's supported platforms are Unix; this exists only to keep
    // the crate buildable on Windows.
    #[cfg(not(unix))]
    {
        fs::write(path, data)?;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // tests mutate a known-good baseline
mod tests {
    use super::*;

    use crate::profile::CURRENT_PROFILE_VERSION;

    use tempfile::tempdir;

    fn scratch_home() -> (tempfile::TempDir, PrincipalHome) {
        let dir = tempdir().unwrap();
        let home = PrincipalHome::from_path(dir.path().join("alice"));
        home.ensure().unwrap();
        (dir, home)
    }

    // ── Load ──────────────────────────────────────────────────────────

    #[test]
    fn load_missing_file_returns_default() {
        let (_d, home) = scratch_home();
        assert!(!PrincipalProfile::path_for(&home).exists());
        let loaded = PrincipalProfile::load(&home).unwrap();
        assert_eq!(loaded, PrincipalProfile::default());
    }

    #[test]
    fn load_malformed_toml_is_hard_error() {
        let (_d, home) = scratch_home();
        let path = PrincipalProfile::path_for(&home);
        fs::write(&path, "this is not valid = = toml [").unwrap();
        let err = PrincipalProfile::load(&home).unwrap_err();
        assert!(matches!(err, ProfileError::Parse(_)), "got: {err:?}");
    }

    #[test]
    fn load_empty_file_is_valid_default_like() {
        // Empty TOML is a valid document → all #[serde(default)] fire.
        // File existence is not a "reset"; the defaults are the same
        // whether the file is absent or empty.
        let (_d, home) = scratch_home();
        let path = PrincipalProfile::path_for(&home);
        fs::write(&path, "").unwrap();
        let loaded = PrincipalProfile::load(&home).unwrap();
        assert_eq!(loaded, PrincipalProfile::default());
    }

    #[test]
    fn load_rejects_unknown_top_level_field() {
        let (_d, home) = scratch_home();
        let path = PrincipalProfile::path_for(&home);
        fs::write(&path, "enabled = true\nenableed = true\n").unwrap();
        let err = PrincipalProfile::load(&home).unwrap_err();
        assert!(matches!(err, ProfileError::Parse(_)), "got: {err:?}");
    }

    #[test]
    fn load_rejects_unknown_auth_method_variant() {
        // Enum variant typo (`passky`) must fail loudly at parse time.
        let (_d, home) = scratch_home();
        let path = PrincipalProfile::path_for(&home);
        fs::write(&path, "[auth]\nmethods = [\"passky\"]\n").unwrap();
        let err = PrincipalProfile::load(&home).unwrap_err();
        assert!(matches!(err, ProfileError::Parse(_)), "got: {err:?}");
    }

    #[test]
    fn load_accepts_known_auth_method_variants() {
        let (_d, home) = scratch_home();
        let path = PrincipalProfile::path_for(&home);
        fs::write(
            &path,
            "[auth]\nmethods = [\"keypair\", \"passkey\", \"system\"]\n",
        )
        .unwrap();
        let loaded = PrincipalProfile::load(&home).unwrap();
        assert_eq!(loaded.auth.methods.len(), 3);
    }

    #[test]
    fn load_rejects_unknown_nested_field() {
        let (_d, home) = scratch_home();
        let path = PrincipalProfile::path_for(&home);
        fs::write(
            &path,
            "[quotas]\nmax_memory_bytes = 1048576\ntypo_field = 42\n",
        )
        .unwrap();
        let err = PrincipalProfile::load(&home).unwrap_err();
        assert!(matches!(err, ProfileError::Parse(_)), "got: {err:?}");
    }

    #[test]
    fn load_rejects_future_version() {
        let (_d, home) = scratch_home();
        let path = PrincipalProfile::path_for(&home);
        let toml_doc = format!("profile_version = {}\n", CURRENT_PROFILE_VERSION + 1);
        fs::write(&path, toml_doc).unwrap();
        let err = PrincipalProfile::load(&home).unwrap_err();
        match err {
            ProfileError::Invalid(msg) => assert!(msg.contains("profile_version"), "msg: {msg}"),
            other => panic!("expected Invalid, got: {other:?}"),
        }
    }

    #[test]
    fn load_accepts_current_version() {
        let (_d, home) = scratch_home();
        let path = PrincipalProfile::path_for(&home);
        let toml_doc = format!("profile_version = {CURRENT_PROFILE_VERSION}\n");
        fs::write(&path, toml_doc).unwrap();
        let loaded = PrincipalProfile::load(&home).unwrap();
        assert_eq!(loaded.profile_version, CURRENT_PROFILE_VERSION);
    }

    // ── Save ──────────────────────────────────────────────────────────

    #[test]
    fn save_then_load_roundtrips() {
        let (_d, home) = scratch_home();
        let mut p = PrincipalProfile::default();
        p.enabled = false;
        p.groups.push("admins".into());
        p.save(&home).unwrap();

        let loaded = PrincipalProfile::load(&home).unwrap();
        assert_eq!(loaded, p);
    }

    #[test]
    fn save_creates_config_dir_if_missing() {
        // Don't call PrincipalHome::ensure — save() must create .config/.
        let dir = tempdir().unwrap();
        let home = PrincipalHome::from_path(dir.path().join("bob"));
        assert!(!home.config_dir().exists());
        PrincipalProfile::default().save(&home).unwrap();
        assert!(home.config_dir().exists());
        assert!(PrincipalProfile::path_for(&home).exists());
    }

    #[test]
    fn save_rejects_invalid_profile() {
        let (_d, home) = scratch_home();
        let mut p = PrincipalProfile::default();
        p.quotas.max_memory_bytes = 0;
        let err = p.save(&home).unwrap_err();
        assert!(matches!(err, ProfileError::Invalid(_)));
        assert!(
            !PrincipalProfile::path_for(&home).exists(),
            "invalid profile must not be written"
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let (_d, home) = scratch_home();
        PrincipalProfile::default().save(&home).unwrap();
        let path = PrincipalProfile::path_for(&home);
        let perms = fs::metadata(&path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "profile.toml must be owner-only",
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_overwrites_preserves_mode() {
        use std::os::unix::fs::PermissionsExt;

        let (_d, home) = scratch_home();
        PrincipalProfile::default().save(&home).unwrap();
        let mut p = PrincipalProfile::default();
        p.enabled = false;
        p.save(&home).unwrap();
        let path = PrincipalProfile::path_for(&home);
        let perms = fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn save_does_not_leave_temp_file() {
        let (_d, home) = scratch_home();
        PrincipalProfile::default().save(&home).unwrap();
        let entries: Vec<_> = fs::read_dir(home.config_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(entries.contains(&"profile.toml".to_string()));
        assert!(
            !entries.iter().any(|n| n.contains(".tmp.")),
            "temp files should be renamed away: {entries:?}",
        );
    }
}
