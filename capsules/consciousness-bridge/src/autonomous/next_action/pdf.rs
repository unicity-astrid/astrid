use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MANIFEST_VERSION: u8 = 1;
const OCR_THRESHOLD_NON_WS: usize = 30;
pub(super) const PDF_READ_PREFIX: &str = "pdf:";
pub(super) const PDF_CHAR_BUDGET: usize = 6000;

#[derive(Debug, Clone)]
pub(super) struct PdfWindow {
    pub text: String,
    pub first_page: usize,
    pub last_page: usize,
    pub total_pages: usize,
    pub next_page: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PdfCacheManifest {
    version: u8,
    source_path: String,
    source_size: u64,
    source_mtime_ms: u64,
    page_count: usize,
}

pub(super) fn marker_for_path(path: &Path) -> String {
    format!("{PDF_READ_PREFIX}{}", path.to_string_lossy())
}

pub(super) fn marker_path(marker: &str) -> Option<PathBuf> {
    marker.strip_prefix(PDF_READ_PREFIX).map(PathBuf::from)
}

pub(super) fn read_pdf_window(
    pdf_path: &Path,
    research_root: &Path,
    start_page: usize,
    char_budget: usize,
) -> Result<PdfWindow, String> {
    let start_page = start_page.max(1);
    let cache = PdfCache::load(pdf_path, research_root)?;
    if start_page > cache.manifest.page_count {
        return Err(format!(
            "[No more PDF pages remain in {}.]",
            pdf_path.display()
        ));
    }

    let mut sections = Vec::new();
    let mut total_chars = 0usize;
    let mut current_page = start_page;
    let mut last_page = start_page;

    while current_page <= cache.manifest.page_count {
        let page_text = cache.page_text(current_page)?;
        let section = format!(
            "--- Page {current_page} of {} ---\n{}",
            cache.manifest.page_count,
            page_text.trim_end()
        );
        let section_chars = section.chars().count();
        let fits_budget = total_chars.saturating_add(section_chars) <= char_budget;
        if !sections.is_empty() && !fits_budget {
            break;
        }
        total_chars = total_chars.saturating_add(section_chars);
        last_page = current_page;
        sections.push(section);
        current_page = current_page.saturating_add(1);
    }

    let next_page = if last_page < cache.manifest.page_count {
        Some(last_page.saturating_add(1))
    } else {
        None
    };

    Ok(PdfWindow {
        text: sections.join("\n\n"),
        first_page: start_page,
        last_page,
        total_pages: cache.manifest.page_count,
        next_page,
    })
}

pub(super) fn format_initial_window(label: &str, window: &PdfWindow) -> String {
    format!(
        "[Research PDF: {label}]\n{}\n\n{}",
        window.text,
        window_footer(window)
    )
}

pub(super) fn format_continuation_window(window: &PdfWindow) -> String {
    format!(
        "[Continuing PDF — pages {}-{} of {}]\n{}\n\n{}",
        window.first_page,
        window.last_page,
        window.total_pages,
        window.text,
        window_footer(window)
    )
}

fn window_footer(window: &PdfWindow) -> String {
    if let Some(next_page) = window.next_page {
        format!(
            "[Showing PDF pages {}-{} of {}. NEXT: READ_MORE for page {}.]",
            window.first_page, window.last_page, window.total_pages, next_page
        )
    } else {
        format!(
            "[End of PDF (pages {}-{} of {}).]",
            window.first_page, window.last_page, window.total_pages
        )
    }
}

struct PdfCache {
    pdf_path: PathBuf,
    cache_dir: PathBuf,
    manifest: PdfCacheManifest,
}

impl PdfCache {
    fn load(pdf_path: &Path, research_root: &Path) -> Result<Self, String> {
        let canonical_path = pdf_path
            .canonicalize()
            .unwrap_or_else(|_| pdf_path.to_path_buf());
        let metadata = fs::metadata(&canonical_path).map_err(|err| {
            format!(
                "[Could not inspect PDF {}: {err}]",
                canonical_path.display()
            )
        })?;
        let source_size = metadata.len();
        let source_mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
            .and_then(|dur| u64::try_from(dur.as_millis()).ok())
            .unwrap_or(0);
        let source_path = canonical_path.to_string_lossy().to_string();
        let cache_root = research_root.join(".pdf_cache");
        fs::create_dir_all(&cache_root).map_err(|err| {
            format!(
                "[Could not create PDF cache root {}: {err}]",
                cache_root.display()
            )
        })?;
        let cache_dir = cache_root.join(doc_id(&source_path));
        fs::create_dir_all(&cache_dir).map_err(|err| {
            format!(
                "[Could not create PDF cache directory {}: {err}]",
                cache_dir.display()
            )
        })?;

        let manifest_path = cache_dir.join("manifest.json");
        let cached = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|content| serde_json::from_str::<PdfCacheManifest>(&content).ok());

        let manifest = if let Some(existing) = cached {
            if existing.version == MANIFEST_VERSION
                && existing.source_path == source_path
                && existing.source_size == source_size
                && existing.source_mtime_ms == source_mtime_ms
            {
                existing
            } else {
                purge_cached_pages(&cache_dir);
                write_manifest(
                    &manifest_path,
                    &PdfCacheManifest {
                        version: MANIFEST_VERSION,
                        source_path,
                        source_size,
                        source_mtime_ms,
                        page_count: pdf_page_count(&canonical_path)?,
                    },
                )?
            }
        } else {
            write_manifest(
                &manifest_path,
                &PdfCacheManifest {
                    version: MANIFEST_VERSION,
                    source_path,
                    source_size,
                    source_mtime_ms,
                    page_count: pdf_page_count(&canonical_path)?,
                },
            )?
        };

        Ok(Self {
            pdf_path: canonical_path,
            cache_dir,
            manifest,
        })
    }

    fn page_text(&self, page: usize) -> Result<String, String> {
        let page_path = self.cache_dir.join(format!("page-{page:04}.txt"));
        let cached = fs::read_to_string(&page_path).unwrap_or_default();
        if !cached.is_empty() {
            return self.resolve_sparse_page(page, cached, &page_path);
        }

        let page_arg = page.to_string();
        let pdf_arg = self.pdf_path.to_string_lossy().to_string();
        let extracted = normalize_page_text(run_utf8_command(
            "pdftotext",
            &[
                "-layout", "-enc", "UTF-8", "-q", "-f", &page_arg, "-l", &page_arg, &pdf_arg, "-",
            ],
            "extract PDF text",
        )?);
        let _ = fs::write(&page_path, &extracted);
        self.resolve_sparse_page(page, extracted, &page_path)
    }

    fn resolve_sparse_page(
        &self,
        page: usize,
        extracted: String,
        page_path: &Path,
    ) -> Result<String, String> {
        if non_whitespace_len(&extracted) >= OCR_THRESHOLD_NON_WS {
            return Ok(non_empty_page_text(extracted));
        }

        match ocr_page_text(&self.pdf_path, page) {
            Ok(ocr_text) if non_whitespace_len(&ocr_text) > non_whitespace_len(&extracted) => {
                let _ = fs::write(page_path, &ocr_text);
                Ok(non_empty_page_text(ocr_text))
            },
            Ok(_) => Ok(annotate_sparse_page(&self.pdf_path, page, extracted, None)),
            Err(reason) => Ok(annotate_sparse_page(
                &self.pdf_path,
                page,
                extracted,
                Some(reason),
            )),
        }
    }
}

fn purge_cached_pages(cache_dir: &Path) {
    if let Ok(entries) = fs::read_dir(cache_dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("page-") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn write_manifest(path: &Path, manifest: &PdfCacheManifest) -> Result<PdfCacheManifest, String> {
    let content = serde_json::to_string_pretty(manifest).map_err(|err| {
        format!(
            "[Could not serialize PDF cache manifest {}: {err}]",
            path.display()
        )
    })?;
    fs::write(path, content).map_err(|err| {
        format!(
            "[Could not write PDF cache manifest {}: {err}]",
            path.display()
        )
    })?;
    Ok(PdfCacheManifest {
        version: manifest.version,
        source_path: manifest.source_path.clone(),
        source_size: manifest.source_size,
        source_mtime_ms: manifest.source_mtime_ms,
        page_count: manifest.page_count,
    })
}

fn pdf_page_count(pdf_path: &Path) -> Result<usize, String> {
    let pdf_arg = pdf_path.to_string_lossy().to_string();
    let output = run_utf8_command("pdfinfo", &[&pdf_arg], "inspect PDF metadata")?;
    output
        .lines()
        .find_map(|line| {
            line.strip_prefix("Pages:")
                .and_then(|rest| rest.trim().parse::<usize>().ok())
        })
        .ok_or_else(|| {
            format!(
                "[Could not determine page count for PDF {}.]",
                pdf_path.display()
            )
        })
}

fn ocr_page_text(pdf_path: &Path, page: usize) -> Result<String, String> {
    let temp_root =
        std::env::temp_dir().join(format!("astrid-pdf-ocr-{}-{}", std::process::id(), page));
    let _ = fs::remove_dir_all(&temp_root);
    fs::create_dir_all(&temp_root).map_err(|err| {
        format!(
            "OCR temp directory creation failed for {}: {err}",
            pdf_path.display()
        )
    })?;
    let image_base = temp_root.join("page");
    let image_path = temp_root.join("page.png");
    let page_arg = page.to_string();
    let pdf_arg = pdf_path.to_string_lossy().to_string();
    let image_base_arg = image_base.to_string_lossy().to_string();
    let image_arg = image_path.to_string_lossy().to_string();

    let render_result = run_utf8_command(
        "pdftoppm",
        &[
            "-f",
            &page_arg,
            "-l",
            &page_arg,
            "-r",
            "200",
            "-png",
            "-singlefile",
            &pdf_arg,
            &image_base_arg,
        ],
        "render PDF page for OCR",
    );
    if let Err(err) = render_result {
        let _ = fs::remove_dir_all(&temp_root);
        return Err(err);
    }

    let ocr = run_utf8_command("tesseract", &[&image_arg, "stdout"], "OCR PDF page")
        .map(normalize_page_text);
    let _ = fs::remove_dir_all(&temp_root);
    ocr
}

fn run_utf8_command(command: &str, args: &[&str], action: &str) -> Result<String, String> {
    let output = Command::new(command).args(args).output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            format!("[{action} requires `{command}`, but it is not installed.]")
        } else {
            format!("[Could not {action} with `{command}`: {err}]")
        }
    })?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        if detail.is_empty() {
            Err(format!("[`{command}` failed while trying to {action}.]"))
        } else {
            Err(format!(
                "[`{command}` failed while trying to {action}: {detail}]"
            ))
        }
    }
}

fn normalize_page_text(text: String) -> String {
    text.replace('\u{000C}', "\n").replace("\r\n", "\n")
}

fn annotate_sparse_page(
    pdf_path: &Path,
    page: usize,
    extracted: String,
    reason: Option<String>,
) -> String {
    let body = if extracted.trim().is_empty() {
        "[No extractable text on this page.]".to_string()
    } else {
        extracted.trim_end().to_string()
    };
    let note = match reason {
        Some(detail) => format!(
            "[Page {page} of {} may need OCR for a fuller read. {detail}]",
            pdf_path.display()
        ),
        None => format!(
            "[Page {page} of {} appears sparse. OCR did not improve extraction.]",
            pdf_path.display()
        ),
    };
    format!("{body}\n\n{note}")
}

fn non_empty_page_text(text: String) -> String {
    if text.trim().is_empty() {
        "[No extractable text on this page.]".to_string()
    } else {
        text
    }
}

fn non_whitespace_len(text: &str) -> usize {
    text.chars().filter(|ch| !ch.is_whitespace()).count()
}

fn doc_id(source_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_path.as_bytes());
    let bytes = hasher.finalize();
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}
