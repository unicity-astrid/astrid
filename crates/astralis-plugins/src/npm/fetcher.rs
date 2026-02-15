//! npm registry HTTP fetcher.
//!
//! Downloads packages directly from the npm registry via HTTP — **without using
//! the npm CLI** — to eliminate lifecycle script attack vectors.
//!
//! The full pipeline: resolve version → download tarball → verify SHA-512 SRI →
//! extract safely → validate `openclaw.plugin.json` exists.

use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use tracing::{debug, info};

use super::extract::extract_tarball;
use super::integrity::verify_sri_integrity;
use super::spec::NpmSpec;
use super::types::{PackageMetadata, VersionMetadata};
use crate::error::{PluginError, PluginResult};

/// Default npm registry URL.
const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org";

/// Default maximum tarball size (50 MB).
const DEFAULT_MAX_SIZE: u64 = 50 * 1024 * 1024;

/// Default HTTP request timeout (120 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Default connection timeout (30 seconds).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum number of HTTP redirects to follow when downloading tarballs.
const MAX_REDIRECTS: u32 = 10;

/// `OpenClaw` manifest filename expected in extracted packages.
const OPENCLAW_MANIFEST: &str = "openclaw.plugin.json";

/// HTTP fetcher for npm registry packages.
pub struct NpmFetcher {
    client: reqwest::Client,
    registry_url: String,
    max_tarball_size: u64,
}

/// A successfully fetched and extracted npm package.
pub struct ExtractedPackage {
    /// Full package name (e.g. `@openclaw/hello-tool`).
    pub name: String,
    /// Resolved version string.
    pub version: String,
    /// Temporary directory owning the extracted files (dropped = cleaned up).
    pub extract_dir: tempfile::TempDir,
    /// Path to the extracted package root within `extract_dir`.
    pub package_root: PathBuf,
}

impl NpmFetcher {
    /// Create a new fetcher with default configuration.
    ///
    /// # Errors
    ///
    /// Returns `PluginError::RegistryError` if the HTTP client cannot be built
    /// (e.g. TLS backend unavailable on minimal Linux containers).
    pub fn new() -> PluginResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("astralis/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| PluginError::RegistryError {
                message: format!("failed to build HTTP client: {e}"),
            })?;

        Ok(Self {
            client,
            registry_url: DEFAULT_REGISTRY.to_string(),
            max_tarball_size: DEFAULT_MAX_SIZE,
        })
    }

    /// Override the registry URL.
    #[must_use]
    pub fn with_registry_url(mut self, url: String) -> Self {
        self.registry_url = url;
        self
    }

    /// Override the maximum tarball size in bytes.
    #[must_use]
    pub fn with_max_size(mut self, bytes: u64) -> Self {
        self.max_tarball_size = bytes;
        self
    }

    /// Override the HTTP request timeout.
    ///
    /// # Errors
    ///
    /// Returns `PluginError::RegistryError` if the HTTP client cannot be rebuilt.
    pub fn with_timeout(mut self, timeout: Duration) -> PluginResult<Self> {
        self.client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("astralis/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| PluginError::RegistryError {
                message: format!("failed to build HTTP client: {e}"),
            })?;
        Ok(self)
    }

    /// Resolve a package specifier to concrete version metadata.
    ///
    /// Fetches the full package metadata from the registry and resolves:
    /// - Explicit version → look up directly
    /// - Dist-tag (e.g. `"latest"`) → resolve via `dist-tags`
    /// - No version → resolve `"latest"` dist-tag
    ///
    /// # Errors
    ///
    /// Returns `PluginError::RegistryError` on network failures, missing
    /// packages, or unresolvable versions.
    pub async fn resolve_version(&self, spec: &NpmSpec) -> PluginResult<VersionMetadata> {
        let url = format!("{}/{}", self.registry_url, spec.registry_path());
        debug!(url = %url, "fetching package metadata");

        let response = self.follow_registry_redirects(&url, "metadata").await?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(PluginError::RegistryError {
                message: format!("registry returned {status} for {}", spec.full_name()),
            });
        }

        let metadata: PackageMetadata =
            response
                .json::<PackageMetadata>()
                .await
                .map_err(|e| PluginError::RegistryError {
                    message: format!("failed to parse registry response: {e}"),
                })?;

        // Determine which version to use.
        let version_str = match &spec.version {
            Some(v) => {
                // Check if it's a dist-tag first, then try as exact version.
                if let Some(resolved) = metadata.dist_tags.get(v.as_str()) {
                    resolved.clone()
                } else {
                    v.clone()
                }
            },
            None => metadata
                .dist_tags
                .get("latest")
                .ok_or_else(|| PluginError::RegistryError {
                    message: format!("no 'latest' dist-tag for {}", spec.full_name()),
                })?
                .clone(),
        };

        metadata
            .versions
            .get(&version_str)
            .cloned()
            .ok_or_else(|| PluginError::RegistryError {
                message: format!("version {version_str} not found for {}", spec.full_name()),
            })
    }

    /// Fetch a package: resolve → download → verify → extract → validate.
    ///
    /// Returns an [`ExtractedPackage`] with the extracted files in a temporary
    /// directory. The directory is cleaned up when the `ExtractedPackage` is dropped.
    ///
    /// # Errors
    ///
    /// Returns errors for network failures, integrity mismatches, extraction
    /// problems, or missing `openclaw.plugin.json`.
    pub async fn fetch(&self, spec: &NpmSpec) -> PluginResult<ExtractedPackage> {
        let version_meta = self.resolve_version(spec).await?;
        let package_name = spec.full_name();

        info!(
            package = %package_name,
            version = %version_meta.version,
            "downloading package"
        );

        // Download the tarball with size checking.
        let tarball_data = self
            .download_tarball(&version_meta.dist.tarball, &package_name)
            .await?;

        // Verify integrity.
        if let Some(integrity) = &version_meta.dist.integrity {
            debug!(package = %package_name, "verifying SRI integrity");
            verify_sri_integrity(&tarball_data, integrity, &package_name)?;
        } else {
            // No integrity hash — this is suspicious for modern packages.
            return Err(PluginError::RegistryError {
                message: format!(
                    "no integrity hash provided for {package_name}@{} — refusing to install",
                    version_meta.version
                ),
            });
        }

        // Extract to a temp directory.
        let extract_dir = tempfile::tempdir().map_err(|e| PluginError::ExtractionError {
            message: format!("failed to create temp directory: {e}"),
        })?;

        debug!(
            dest = %extract_dir.path().display(),
            "extracting tarball"
        );
        let package_root = extract_tarball(&tarball_data, extract_dir.path())?;

        // Validate that this is an OpenClaw plugin.
        if !package_root.join(OPENCLAW_MANIFEST).exists() {
            return Err(PluginError::NotOpenClawPlugin);
        }

        info!(
            package = %package_name,
            version = %version_meta.version,
            root = %package_root.display(),
            "package fetched and verified"
        );

        Ok(ExtractedPackage {
            name: package_name,
            version: version_meta.version,
            extract_dir,
            package_root,
        })
    }

    /// Validate that a tarball URL is safe to fetch (SSRF protection).
    ///
    /// Requires HTTPS scheme and that the hostname matches the configured
    /// registry hostname. This is simpler and more robust than IP blocklisting,
    /// and immune to DNS rebinding attacks.
    fn validate_tarball_url(&self, tarball_url: &str) -> PluginResult<()> {
        let tarball = url::Url::parse(tarball_url).map_err(|e| PluginError::SsrfBlocked {
            url: format!("{tarball_url} (parse error: {e})"),
        })?;

        // Require HTTPS
        if tarball.scheme() != "https" {
            return Err(PluginError::SsrfBlocked {
                url: tarball_url.to_string(),
            });
        }

        // Compare full origin (scheme + host + port) to prevent port-based SSRF
        let registry =
            url::Url::parse(&self.registry_url).map_err(|e| PluginError::RegistryError {
                message: format!("invalid registry URL: {e}"),
            })?;

        if tarball.host_str() != registry.host_str()
            || tarball.port_or_known_default() != registry.port_or_known_default()
        {
            return Err(PluginError::SsrfBlocked {
                url: tarball_url.to_string(),
            });
        }

        Ok(())
    }

    /// Follow redirects manually, validating each hop against the registry origin.
    ///
    /// Automatic redirects are disabled on the client so that intermediate hops
    /// cannot reach internal hosts (SSRF via redirect chain). Each hop's URL is
    /// validated before following.
    async fn follow_registry_redirects(
        &self,
        url: &str,
        context: &str,
    ) -> PluginResult<reqwest::Response> {
        self.validate_tarball_url(url)?;

        let mut current_url = url.to_string();

        for _ in 0..MAX_REDIRECTS {
            let resp = self.client.get(&current_url).send().await.map_err(|e| {
                PluginError::RegistryError {
                    message: format!("failed to fetch {context}: {e}"),
                }
            })?;

            if resp.status().is_redirection() {
                let location = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| PluginError::RegistryError {
                        message: format!("redirect without Location header for {context}"),
                    })?;

                let next_url = url::Url::parse(&current_url)
                    .and_then(|base| base.join(location))
                    .map_err(|e| PluginError::SsrfBlocked {
                        url: format!("{location} (parse error: {e})"),
                    })?;

                self.validate_tarball_url(next_url.as_str())?;
                debug!(redirect = %next_url, context, "following validated redirect");
                current_url = next_url.into();
                continue;
            }

            return Ok(resp);
        }

        Err(PluginError::RegistryError {
            message: format!("too many redirects for {context}"),
        })
    }

    /// Download a tarball with per-hop SSRF validation and streaming size enforcement.
    async fn download_tarball(&self, url: &str, package_name: &str) -> PluginResult<Vec<u8>> {
        debug!(url = %url, "downloading tarball");

        let response = self.follow_registry_redirects(url, package_name).await?;

        if !response.status().is_success() {
            return Err(PluginError::RegistryError {
                message: format!(
                    "tarball download failed with status {} for {package_name}",
                    response.status()
                ),
            });
        }

        // Check Content-Length hint before streaming.
        if let Some(content_length) = response.content_length()
            && content_length > self.max_tarball_size
        {
            return Err(PluginError::PackageTooLarge {
                size: content_length,
                limit: self.max_tarball_size,
            });
        }

        // Stream the body with a running size counter to enforce the limit,
        // even when Content-Length is missing or lies.
        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut downloaded: u64 = 0;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| PluginError::RegistryError {
                message: format!("failed to read tarball body for {package_name}: {e}"),
            })?;

            downloaded = downloaded.saturating_add(chunk.len() as u64);
            if downloaded > self.max_tarball_size {
                return Err(PluginError::PackageTooLarge {
                    size: downloaded,
                    limit: self.max_tarball_size,
                });
            }

            buffer.extend_from_slice(&chunk);
        }

        Ok(buffer)
    }
}

// NpmFetcher does not implement Default because construction is fallible
// (TLS backend may be unavailable). Use NpmFetcher::new() instead.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let fetcher = NpmFetcher::new().unwrap();
        assert_eq!(fetcher.registry_url, DEFAULT_REGISTRY);
        assert_eq!(fetcher.max_tarball_size, DEFAULT_MAX_SIZE);
    }

    #[test]
    fn builder_methods() {
        let fetcher = NpmFetcher::new()
            .unwrap()
            .with_registry_url("https://custom.registry.com".into())
            .with_max_size(10 * 1024 * 1024);
        assert_eq!(fetcher.registry_url, "https://custom.registry.com");
        assert_eq!(fetcher.max_tarball_size, 10 * 1024 * 1024);
    }

    #[test]
    fn ssrf_rejects_http() {
        let fetcher = NpmFetcher::new().unwrap();
        let err = fetcher
            .validate_tarball_url("http://registry.npmjs.org/pkg/-/pkg-1.0.0.tgz")
            .unwrap_err();
        assert!(err.to_string().contains("SSRF blocked"));
    }

    #[test]
    fn ssrf_rejects_different_host() {
        let fetcher = NpmFetcher::new().unwrap();
        let err = fetcher
            .validate_tarball_url("https://evil.com/pkg/-/pkg-1.0.0.tgz")
            .unwrap_err();
        assert!(err.to_string().contains("SSRF blocked"));
    }

    #[test]
    fn ssrf_allows_registry_host() {
        let fetcher = NpmFetcher::new().unwrap();
        fetcher
            .validate_tarball_url("https://registry.npmjs.org/pkg/-/pkg-1.0.0.tgz")
            .unwrap();
    }

    #[test]
    fn ssrf_rejects_different_port() {
        let fetcher = NpmFetcher::new()
            .unwrap()
            .with_registry_url("https://npm.mycorp.com:8443".into());
        let err = fetcher
            .validate_tarball_url("https://npm.mycorp.com:9999/pkg/-/pkg-1.0.0.tgz")
            .unwrap_err();
        assert!(err.to_string().contains("SSRF blocked"));
    }

    #[test]
    fn ssrf_allows_custom_registry() {
        let fetcher = NpmFetcher::new()
            .unwrap()
            .with_registry_url("https://npm.mycorp.com".into());
        fetcher
            .validate_tarball_url("https://npm.mycorp.com/pkg/-/pkg-1.0.0.tgz")
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "hits real npm registry — run manually or in integration CI"]
    async fn resolve_nonexistent_package() {
        let fetcher = NpmFetcher::new().unwrap();
        let spec =
            NpmSpec::parse("@openclaw/this-package-definitely-does-not-exist-xyz-12345").unwrap();
        let result = fetcher.resolve_version(&spec).await;
        assert!(result.is_err());
    }
}
