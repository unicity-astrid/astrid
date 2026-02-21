//! Safe tarball extraction with path traversal protection.
//!
//! Extracts `.tgz` (gzip-compressed tar) archives while guarding against:
//! - Path traversal (`../` components)
//! - Absolute paths
//! - Excessive file counts (denial-of-service protection)

use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

use crate::error::{PluginError, PluginResult};

/// Maximum number of entries allowed in a tarball.
const MAX_ENTRY_COUNT: usize = 10_000;

/// Maximum total extracted size (500 MB) — gzip bomb protection.
const MAX_EXTRACTED_SIZE: u64 = 500_000_000;

/// Extract a gzip-compressed tarball into `dest`, returning the package root.
///
/// npm tarballs contain a leading `package/` directory. This function strips
/// that prefix and returns the path to the extracted package root.
///
/// # Security
///
/// - Rejects entries with `..` path components
/// - Rejects absolute paths
/// - Limits total entry count to [`MAX_ENTRY_COUNT`]
/// - All paths are validated to stay within `dest`
///
/// # Errors
///
/// Returns `PluginError::ExtractionError` on decompression/archive failures,
/// `PluginError::PathTraversal` on malicious paths.
pub fn extract_tarball(data: &[u8], dest: &Path) -> PluginResult<PathBuf> {
    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);

    let dest = dest
        .canonicalize()
        .map_err(|e| PluginError::ExtractionError {
            message: format!("failed to canonicalize destination: {e}"),
        })?;

    let mut entry_count = 0usize;
    let mut total_size: u64 = 0;

    for entry_result in archive
        .entries()
        .map_err(|e| PluginError::ExtractionError {
            message: format!("failed to read archive entries: {e}"),
        })?
    {
        let mut entry = entry_result.map_err(|e| PluginError::ExtractionError {
            message: format!("failed to read archive entry: {e}"),
        })?;

        entry_count = entry_count.saturating_add(1);
        if entry_count > MAX_ENTRY_COUNT {
            return Err(PluginError::ExtractionError {
                message: format!("archive exceeds maximum entry count ({MAX_ENTRY_COUNT})"),
            });
        }

        // Reject unsafe entry types (symlinks, hardlinks, device nodes, etc.)
        let entry_type = entry.header().entry_type();
        if !is_safe_entry_type(entry_type) {
            let entry_path = entry
                .path()
                .map_or_else(|_| "<unknown>".to_string(), |p| p.display().to_string());
            return Err(PluginError::UnsafeEntryType {
                entry_type: format!("{entry_type:?}"),
                path: entry_path,
            });
        }

        // Track cumulative extracted size for gzip bomb protection.
        let entry_size = entry
            .header()
            .size()
            .map_err(|e| PluginError::ExtractionError {
                message: format!("failed to read entry size: {e}"),
            })?;
        total_size = total_size.saturating_add(entry_size);
        if total_size > MAX_EXTRACTED_SIZE {
            return Err(PluginError::ExtractionError {
                message: format!(
                    "archive exceeds maximum extracted size ({MAX_EXTRACTED_SIZE} bytes)"
                ),
            });
        }

        let entry_path = entry
            .path()
            .map_err(|e| PluginError::ExtractionError {
                message: format!("failed to read entry path: {e}"),
            })?
            .into_owned();

        // Validate the raw path before any manipulation.
        validate_entry_path(&entry_path)?;

        // Strip the leading `package/` directory that npm includes.
        let stripped = strip_package_prefix(&entry_path);
        let target = dest.join(stripped);

        // Defense-in-depth: verify the resolved path stays within dest.
        // The primary defense is `validate_entry_path` above; this catches
        // symlink-based escapes that component-level checks cannot see.
        if let Some(canonical_parent) = target.parent().and_then(|p| p.canonicalize().ok()) {
            let canonical_target = canonical_parent.join(target.file_name().unwrap_or_default());
            if !canonical_target.starts_with(&dest) {
                return Err(PluginError::PathTraversal {
                    path: entry_path.display().to_string(),
                });
            }
        } else {
            // Parent directory does not exist yet (first entry in a
            // subdirectory). Primary validation via validate_entry_path()
            // has already rejected `..`, absolute paths, and prefix
            // components, so this is safe. The directory will be created
            // below, and subsequent entries in it will be canonicalized.
        }

        // Create parent directories if they don't exist.
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PluginError::ExtractionError {
                message: format!("failed to create directory {}: {e}", parent.display()),
            })?;
        }

        entry
            .unpack(&target)
            .map_err(|e| PluginError::ExtractionError {
                message: format!("failed to unpack {}: {e}", entry_path.display()),
            })?;
    }

    if entry_count == 0 {
        return Err(PluginError::ExtractionError {
            message: "archive is empty".into(),
        });
    }

    Ok(dest)
}

/// Check whether a tar entry type is safe to extract.
///
/// Allows regular files, directories, and metadata headers.
/// Rejects symlinks, hardlinks, block/char devices, FIFOs, and GNU
/// sparse entries (which should never appear in npm tarballs and
/// expand the attack surface via sparse-map header parsing).
fn is_safe_entry_type(entry_type: tar::EntryType) -> bool {
    matches!(
        entry_type,
        tar::EntryType::Regular
            | tar::EntryType::Directory
            | tar::EntryType::GNULongName
            | tar::EntryType::XHeader
            | tar::EntryType::XGlobalHeader
    )
}

/// Validate that an entry path has no traversal components or absolute paths.
fn validate_entry_path(path: &Path) -> PluginResult<()> {
    // Reject absolute paths.
    if path.is_absolute() {
        return Err(PluginError::PathTraversal {
            path: path.display().to_string(),
        });
    }

    // Reject path components that could escape the destination:
    // - `..` parent directory traversal
    // - Windows drive/UNC prefixes (defense-in-depth)
    // - Root directory components
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::Prefix(_)
                | std::path::Component::RootDir
        ) {
            return Err(PluginError::PathTraversal {
                path: path.display().to_string(),
            });
        }
    }

    Ok(())
}

/// Strip the leading `package/` prefix from an npm tarball entry path.
fn strip_package_prefix(path: &Path) -> PathBuf {
    let mut components = path.components();
    if let Some(first) = components.next() {
        let first_str = first.as_os_str().to_string_lossy();
        if first_str == "package" {
            return components.as_path().to_path_buf();
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Create a gzipped tarball in memory with the given entries (safe paths only).
    fn create_test_tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for &(path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, data).unwrap();
        }
        let tar_data = builder.into_inner().unwrap();

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    /// Create a gzipped tarball with raw path bytes (bypasses tar crate validation).
    /// Used to test path traversal protection in our extraction code.
    fn create_malicious_tarball(path_bytes: &[u8], data: &[u8]) -> Vec<u8> {
        // Build a raw tar entry: 512-byte header + data padded to 512.
        let mut header = [0u8; 512];

        // Copy path into name field (offset 0, 100 bytes).
        let len = path_bytes.len().min(100);
        header[..len].copy_from_slice(&path_bytes[..len]);

        // Mode field (offset 100, 8 bytes): "0000644\0"
        header[100..108].copy_from_slice(b"0000644\0");

        // UID/GID (offset 108..124): zeros are fine.

        // Size field (offset 124, 12 bytes): octal size.
        let size_str = format!("{:011o}\0", data.len());
        header[124..136].copy_from_slice(size_str.as_bytes());

        // Mtime (offset 136, 12 bytes): zeros are fine.

        // Typeflag (offset 156): '0' = regular file.
        header[156] = b'0';

        // Compute checksum (offset 148, 8 bytes).
        // Per tar spec, treat checksum field as spaces for computation.
        header[148..156].copy_from_slice(b"        ");
        let cksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        let cksum_str = format!("{cksum:06o}\0 ");
        header[148..156].copy_from_slice(cksum_str.as_bytes());

        let mut tar_data = Vec::new();
        tar_data.extend_from_slice(&header);
        tar_data.extend_from_slice(data);
        // Pad to 512-byte boundary.
        let padding = (512 - (data.len() % 512)) % 512;
        tar_data.extend(std::iter::repeat_n(0u8, padding));
        // End-of-archive marker: two zero blocks.
        tar_data.extend(std::iter::repeat_n(0u8, 1024));

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn extract_normal_tarball() {
        let tgz = create_test_tarball(&[
            ("package/index.js", b"console.log('hello');"),
            ("package/package.json", b"{}"),
        ]);

        let tmp = tempfile::tempdir().unwrap();
        let root = extract_tarball(&tgz, tmp.path()).unwrap();

        assert!(root.join("index.js").exists());
        assert!(root.join("package.json").exists());
    }

    #[test]
    fn extract_strips_package_prefix() {
        // Include the directory entry so the subdirectory is created.
        let tgz = create_test_tarball(&[("package/src/main.js", b"export default 42;")]);

        let tmp = tempfile::tempdir().unwrap();
        let root = extract_tarball(&tgz, tmp.path()).unwrap();

        assert!(root.join("src/main.js").exists());
    }

    #[test]
    fn reject_path_traversal() {
        let tgz = create_malicious_tarball(b"package/../../../etc/passwd", b"malicious");

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&tgz, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("path traversal"),
            "expected path traversal error, got: {err}"
        );
    }

    #[test]
    fn reject_absolute_path() {
        let tgz = create_malicious_tarball(b"/etc/passwd", b"malicious");

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&tgz, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("path traversal"),
            "expected path traversal error, got: {err}"
        );
    }

    #[test]
    fn reject_empty_archive() {
        let tgz = create_test_tarball(&[]);

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&tgz, tmp.path()).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn reject_oversized_entry_count() {
        // Create a tarball with MAX_ENTRY_COUNT + 1 entries.
        let entries: Vec<(String, Vec<u8>)> = (0..=MAX_ENTRY_COUNT)
            .map(|i| (format!("package/file_{i}.txt"), vec![b'a']))
            .collect();
        let entry_refs: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(p, d)| (p.as_str(), d.as_slice()))
            .collect();

        let tgz = create_test_tarball(&entry_refs);

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&tgz, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("maximum entry count"),
            "expected entry count error, got: {err}"
        );
    }

    /// Create a tarball containing a symlink entry.
    fn create_symlink_tarball() -> Vec<u8> {
        // Build a raw tar entry with type '2' (symlink)
        let path = b"package/evil-link";
        let link_target = b"/etc/passwd";

        let mut header = [0u8; 512];
        // Name field (offset 0, 100 bytes)
        header[..path.len()].copy_from_slice(path);
        // Mode (offset 100, 8 bytes)
        header[100..108].copy_from_slice(b"0000777\0");
        // Size (offset 124, 12 bytes) — symlinks have zero size
        header[124..136].copy_from_slice(b"00000000000\0");
        // Typeflag (offset 156): '2' = symlink
        header[156] = b'2';
        // Link name (offset 157, 100 bytes)
        header[157..157 + link_target.len()].copy_from_slice(link_target);
        // Compute checksum
        header[148..156].copy_from_slice(b"        ");
        let cksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        let cksum_str = format!("{cksum:06o}\0 ");
        header[148..156].copy_from_slice(cksum_str.as_bytes());

        let mut tar_data = Vec::new();
        tar_data.extend_from_slice(&header);
        // End-of-archive marker
        tar_data.extend(std::iter::repeat_n(0u8, 1024));

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn reject_symlink_entry() {
        let tgz = create_symlink_tarball();
        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&tgz, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("unsafe archive entry type"),
            "expected unsafe entry type error, got: {err}"
        );
    }

    #[test]
    fn validate_entry_path_ok() {
        validate_entry_path(Path::new("package/index.js")).unwrap();
        validate_entry_path(Path::new("package/src/deep/file.ts")).unwrap();
    }

    #[test]
    fn validate_entry_path_parent_dir() {
        let err = validate_entry_path(Path::new("package/../escape")).unwrap_err();
        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn reject_gzip_bomb() {
        // Create a tarball with a single entry claiming to be larger than MAX_EXTRACTED_SIZE.
        // The actual data is tiny — this tests header-based size tracking, not real I/O.
        let claimed_size: u64 = MAX_EXTRACTED_SIZE.saturating_add(1);
        let actual_data = b"small";

        let mut header = [0u8; 512];
        let path = b"package/bomb.bin";
        header[..path.len()].copy_from_slice(path);
        header[100..108].copy_from_slice(b"0000644\0");
        let size_str = format!("{claimed_size:011o}\0");
        header[124..136].copy_from_slice(size_str.as_bytes());
        header[156] = b'0';
        header[148..156].copy_from_slice(b"        ");
        let cksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        let cksum_str = format!("{cksum:06o}\0 ");
        header[148..156].copy_from_slice(cksum_str.as_bytes());

        let mut tar_data = Vec::new();
        tar_data.extend_from_slice(&header);
        tar_data.extend_from_slice(actual_data);
        let padding = (512 - (actual_data.len() % 512)) % 512;
        tar_data.extend(std::iter::repeat_n(0u8, padding));
        tar_data.extend(std::iter::repeat_n(0u8, 1024));

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar_data).unwrap();
        let tgz = encoder.finish().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&tgz, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("maximum extracted size"),
            "expected size limit error, got: {err}"
        );
    }
}
