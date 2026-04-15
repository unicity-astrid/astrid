use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local};

pub const DEFAULT_LIVE_CAP: usize = 6_000;
pub const DEFAULT_BUCKET_SIZE: usize = 3_000;

#[derive(Debug, Clone, Copy)]
pub struct ManagedDirOptions<'a> {
    extension: &'a str,
    live_cap: usize,
    bucket_size: usize,
}

impl<'a> ManagedDirOptions<'a> {
    #[must_use]
    pub const fn new(extension: &'a str) -> Self {
        Self {
            extension,
            live_cap: DEFAULT_LIVE_CAP,
            bucket_size: DEFAULT_BUCKET_SIZE,
        }
    }
}

#[derive(Debug, Clone)]
struct ManagedFile {
    path: PathBuf,
    modified: SystemTime,
}

#[allow(dead_code)]
pub fn compact_text_directory(dir: &Path) -> io::Result<Vec<PathBuf>> {
    compact_directory(dir, ManagedDirOptions::new("txt"))
}

pub fn compact_json_directory(dir: &Path) -> io::Result<Vec<PathBuf>> {
    compact_directory(dir, ManagedDirOptions::new("json"))
}

pub fn compact_directory(dir: &Path, options: ManagedDirOptions<'_>) -> io::Result<Vec<PathBuf>> {
    if options.live_cap == 0 || options.bucket_size == 0 || !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut created = Vec::new();
    let archive_root = dir.join("archive");

    loop {
        let live_files = live_files(dir, options.extension)?;
        if live_files.len() <= options.live_cap {
            return Ok(created);
        }

        let bucket_files = &live_files[..options.bucket_size.min(live_files.len())];
        let newest_moved = bucket_files
            .last()
            .map(|file| file.modified)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let bucket_dir = archive_root.join(format!("until_{}", bucket_timestamp(newest_moved)));
        fs::create_dir_all(&bucket_dir)?;

        for file in bucket_files {
            if let Some(name) = file.path.file_name() {
                fs::rename(&file.path, bucket_dir.join(name))?;
            }
        }

        if created.last() != Some(&bucket_dir) {
            created.push(bucket_dir);
        }
    }
}

fn live_files(dir: &Path, extension: &str) -> io::Result<Vec<ManagedFile>> {
    let mut files: Vec<_> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || !path.extension().is_some_and(|ext| ext == extension) {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some(ManagedFile { path, modified })
        })
        .collect();

    files.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(files)
}

fn bucket_timestamp(modified: SystemTime) -> String {
    let datetime: DateTime<Local> = modified.into();
    datetime.format("%Y-%m-%dT%H-%M-%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("{label}_{unique}"))
    }

    fn write_files(dir: &Path, count: usize, extension: &str) {
        fs::create_dir_all(dir).unwrap();
        for idx in 0..count {
            let path = dir.join(format!("{idx:04}.{extension}"));
            fs::write(path, idx.to_string()).unwrap();
        }
    }

    fn live_names(dir: &Path, extension: &str) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|ext| ext == extension) {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned)
                } else {
                    None
                }
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn compacts_oldest_direct_files() {
        let dir = temp_dir("managed_dir_compact");
        let _ = fs::remove_dir_all(&dir);
        write_files(&dir, 7, "txt");

        let created = compact_directory(
            &dir,
            ManagedDirOptions {
                extension: "txt",
                live_cap: 6,
                bucket_size: 3,
            },
        )
        .unwrap();

        assert_eq!(created.len(), 1);
        assert!(
            created[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("until_")
        );
        assert_eq!(
            live_names(&dir, "txt"),
            vec![
                "0003.txt".to_string(),
                "0004.txt".to_string(),
                "0005.txt".to_string(),
                "0006.txt".to_string(),
            ]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_existing_archive_subtree() {
        let dir = temp_dir("managed_dir_archive_ignore");
        let archive_dir = dir.join("archive/until_old");
        let _ = fs::remove_dir_all(&dir);
        write_files(&dir, 7, "txt");
        write_files(&archive_dir, 4, "txt");

        let created = compact_directory(
            &dir,
            ManagedDirOptions {
                extension: "txt",
                live_cap: 6,
                bucket_size: 3,
            },
        )
        .unwrap();

        assert_eq!(created.len(), 1);
        assert_eq!(live_names(&dir, "txt").len(), 4);
        assert_eq!(live_names(&archive_dir, "txt").len(), 4);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeats_in_chunks_until_under_cap() {
        let dir = temp_dir("managed_dir_multiple_chunks");
        let _ = fs::remove_dir_all(&dir);
        write_files(&dir, 13, "json");

        let created = compact_directory(
            &dir,
            ManagedDirOptions {
                extension: "json",
                live_cap: 6,
                bucket_size: 3,
            },
        )
        .unwrap();

        assert!(!created.is_empty());
        assert_eq!(
            live_names(&dir, "json"),
            vec![
                "0009.json".to_string(),
                "0010.json".to_string(),
                "0011.json".to_string(),
                "0012.json".to_string(),
            ]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rerun_is_a_noop_under_cap() {
        let dir = temp_dir("managed_dir_noop");
        let _ = fs::remove_dir_all(&dir);
        write_files(&dir, 6, "txt");

        let first = compact_directory(
            &dir,
            ManagedDirOptions {
                extension: "txt",
                live_cap: 6,
                bucket_size: 3,
            },
        )
        .unwrap();
        let second = compact_directory(
            &dir,
            ManagedDirOptions {
                extension: "txt",
                live_cap: 6,
                bucket_size: 3,
            },
        )
        .unwrap();

        assert!(first.is_empty());
        assert!(second.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
