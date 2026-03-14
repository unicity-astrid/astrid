#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! HTTP fetch tool capsule for Astrid agents.
//!
//! Provides the `fetch_url` tool, giving agents native HTTP access without
//! shelling out to `curl`. Uses the host's HTTP implementation which includes
//! SSRF prevention, timeouts, and payload limits.
//!
//! # Security notes
//!
//! **Headers**: The tool passes agent-provided headers to the host unfiltered.
//! This means an agent (or a prompt-injected agent) can set `Host`,
//! `Authorization`, `Cookie`, or `X-Forwarded-For`. The host's SSRF layer
//! blocks private/local IPs at DNS resolution time, but header injection to
//! public endpoints is within the threat model accepted by `net = ["*"]`.
//!
//! **Response headers**: The full response header map is returned to the LLM,
//! including `Set-Cookie` and any auth tokens. This is by design - the agent
//! needs headers to interpret responses - but operators should be aware that
//! response secrets enter the LLM context window.

use std::collections::HashMap;

use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use serde::{Deserialize, Serialize};

/// Maximum response body size returned to the LLM (200 KB).
///
/// The host enforces a hard 10 MB cap; this soft limit prevents a single
/// fetch from exhausting the agent's context window.
const MAX_RESPONSE_BODY_LEN: usize = 200 * 1024;

/// The main entry point for the HTTP Tools capsule.
#[derive(Default)]
pub struct HttpTools;

/// Input arguments for the `fetch_url` tool.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct FetchUrlArgs {
    /// The URL to fetch (http:// or https:// only).
    pub url: String,
    /// HTTP method. Defaults to "GET".
    pub method: Option<String>,
    /// Optional HTTP headers as key-value pairs.
    pub headers: Option<HashMap<String, String>>,
    /// Optional request body (for POST/PUT/PATCH).
    pub body: Option<String>,
}

/// The request format expected by the host's `astrid_http_request`.
#[derive(Serialize)]
struct HostHttpRequest {
    url: String,
    method: String,
    headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
}

/// The response format returned by the host.
#[derive(Deserialize)]
struct HostHttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

/// The structured response returned to the LLM.
#[derive(Serialize)]
struct FetchResult {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
    #[serde(skip_serializing_if = "is_false")]
    truncated: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// Validate a URL before sending it to the host.
///
/// Rejects empty URLs and non-http(s) schemes.
fn validate_url(url: &str) -> Result<(), &'static str> {
    if url.is_empty() {
        return Err("URL cannot be empty");
    }
    // RFC 3986: scheme is case-insensitive, so accept HTTP:// and HTTPS://.
    // Only lowercase the scheme portion for comparison - path/query are case-sensitive.
    let scheme_lower = url
        .as_bytes()
        .iter()
        .map(u8::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if !scheme_lower.starts_with(b"http://") && !scheme_lower.starts_with(b"https://") {
        return Err("Only http:// and https:// URLs are supported");
    }
    Ok(())
}

/// Truncate the body to `max_len` bytes at a valid UTF-8 boundary.
///
/// Returns `(body, was_truncated)`.
fn truncate_body(body: &str, max_len: usize) -> (String, bool) {
    if body.len() <= max_len {
        return (body.to_string(), false);
    }
    let end = body.floor_char_boundary(max_len);
    let truncated = format!(
        "{}\n\n[...truncated, {} bytes total]",
        &body[..end],
        body.len()
    );
    (truncated, true)
}

#[capsule]
impl HttpTools {
    /// Fetch a URL over HTTP/HTTPS.
    ///
    /// Returns a JSON object with `status`, `headers`, `body`, and an optional
    /// `truncated` flag. HTTP error statuses (4xx/5xx) are returned as data so
    /// the LLM can reason about them. Only infrastructure failures (DNS,
    /// timeout, SSRF block) produce errors.
    #[astrid::tool("fetch_url")]
    pub fn fetch_url(&self, args: FetchUrlArgs) -> Result<String, SysError> {
        let url = args.url.trim();
        validate_url(url).map_err(|e| SysError::ApiError(e.into()))?;

        let method = args.method.as_deref().unwrap_or("GET").to_uppercase();

        let request = HostHttpRequest {
            url: url.to_string(),
            method,
            headers: args.headers.unwrap_or_default(),
            body: args.body,
        };

        let request_bytes =
            serde_json::to_vec(&request).map_err(|e| SysError::ApiError(e.to_string()))?;

        let response_bytes = http::request_bytes(&request_bytes)?;

        let response: HostHttpResponse = serde_json::from_slice(&response_bytes)
            .map_err(|e| SysError::ApiError(format!("failed to parse host response: {e}")))?;

        let (body, truncated) = truncate_body(&response.body, MAX_RESPONSE_BODY_LEN);

        let result = FetchResult {
            status: response.status,
            headers: response.headers,
            body,
            truncated,
        };

        serde_json::to_string(&result).map_err(|e| SysError::ApiError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- URL validation --

    #[test]
    fn validate_url_rejects_empty() {
        assert_eq!(validate_url(""), Err("URL cannot be empty"));
    }

    #[test]
    fn validate_url_rejects_whitespace_only() {
        // validate_url itself doesn't trim; the caller (fetch_url) does.
        // Whitespace-only input hits the scheme check, not the empty check.
        assert_eq!(
            validate_url("   "),
            Err("Only http:// and https:// URLs are supported")
        );
    }

    #[test]
    fn validate_url_rejects_file_scheme() {
        assert_eq!(
            validate_url("file:///etc/passwd"),
            Err("Only http:// and https:// URLs are supported")
        );
    }

    #[test]
    fn validate_url_rejects_ftp_scheme() {
        assert_eq!(
            validate_url("ftp://example.com/file"),
            Err("Only http:// and https:// URLs are supported")
        );
    }

    #[test]
    fn validate_url_rejects_no_scheme() {
        assert_eq!(
            validate_url("example.com"),
            Err("Only http:// and https:// URLs are supported")
        );
    }

    #[test]
    fn validate_url_accepts_https() {
        assert_eq!(validate_url("https://example.com"), Ok(()));
    }

    #[test]
    fn validate_url_accepts_http() {
        assert_eq!(validate_url("http://example.com"), Ok(()));
    }

    #[test]
    fn validate_url_accepts_uppercase_scheme() {
        assert_eq!(validate_url("HTTP://example.com"), Ok(()));
        assert_eq!(validate_url("HTTPS://example.com"), Ok(()));
        assert_eq!(validate_url("Http://example.com"), Ok(()));
    }

    // -- Method normalization --

    #[test]
    fn method_defaults_to_get() {
        let method: Option<String> = None;
        assert_eq!(method.as_deref().unwrap_or("GET").to_uppercase(), "GET");
    }

    #[test]
    fn method_uppercased() {
        let method = Some("post".to_string());
        assert_eq!(method.as_deref().unwrap_or("GET").to_uppercase(), "POST");
    }

    // -- Body truncation --

    #[test]
    fn truncate_short_body_unchanged() {
        let (body, truncated) = truncate_body("hello", 100);
        assert_eq!(body, "hello");
        assert!(!truncated);
    }

    #[test]
    fn truncate_exact_limit_unchanged() {
        let input = "a".repeat(200);
        let (body, truncated) = truncate_body(&input, 200);
        assert_eq!(body, input);
        assert!(!truncated);
    }

    #[test]
    fn truncate_long_body() {
        let input = "a".repeat(300);
        let (body, truncated) = truncate_body(&input, 200);
        assert!(truncated);
        assert!(body.contains("[...truncated, 300 bytes total]"));
        let prefix_end = body.find("\n\n[...truncated").expect("marker missing");
        assert_eq!(prefix_end, 200);
    }

    #[test]
    fn truncate_at_multibyte_char_boundary() {
        // Each emoji is 4 bytes
        let input = "\u{1F600}".repeat(100); // 400 bytes
        let (body, truncated) = truncate_body(&input, 10);
        assert!(truncated);
        // floor_char_boundary(10) for 4-byte chars = 8, so 2 emoji chars
        let prefix_end = body.find("\n\n[...truncated").expect("marker missing");
        assert_eq!(prefix_end, 8);
    }

    // -- serde skip helper --

    #[test]
    fn is_false_returns_true_for_false() {
        assert!(is_false(&false));
    }

    #[test]
    fn is_false_returns_false_for_true() {
        assert!(!is_false(&true));
    }
}
