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
use std::str::FromStr;

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
struct HostHttpRequest<'a> {
    url: &'a str,
    method: &'a str,
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
    // Compare only the scheme prefix without allocating a lowercased copy.
    let scheme_end = url.find("://").map_or(0, |i| i + 3);
    let scheme = &url[..scheme_end];
    if !scheme.eq_ignore_ascii_case("http://") && !scheme.eq_ignore_ascii_case("https://") {
        return Err("Only http:// and https:// URLs are supported");
    }
    Ok(())
}

/// Allowed HTTP methods. Matches the set supported by the host.
///
/// Parsing via `FromStr` handles case-insensitive matching and rejects
/// unsupported methods (TRACE, CONNECT, etc.) at the type level.
#[derive(Debug)]
enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

impl HttpMethod {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Patch => "PATCH",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }
}

impl FromStr for HttpMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            "PATCH" => Ok(Self::Patch),
            "HEAD" => Ok(Self::Head),
            "OPTIONS" => Ok(Self::Options),
            _ => Err(format!("unsupported HTTP method: {s}")),
        }
    }
}

/// Truncate the body to `max_len` bytes at a valid UTF-8 boundary.
///
/// Takes ownership to avoid cloning when the body fits within the limit.
/// Returns `(body, was_truncated)`.
fn truncate_body(body: String, max_len: usize) -> (String, bool) {
    if body.len() <= max_len {
        return (body, false);
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

        let method: HttpMethod = args
            .method
            .as_deref()
            .unwrap_or("GET")
            .parse()
            .map_err(SysError::ApiError)?;

        let request = HostHttpRequest {
            url,
            method: method.as_str(),
            headers: args.headers.unwrap_or_default(),
            body: args.body,
        };

        let request_bytes =
            serde_json::to_vec(&request).map_err(|e| SysError::ApiError(e.to_string()))?;

        let response_bytes = http::request_bytes(&request_bytes)?;

        let response: HostHttpResponse = serde_json::from_slice(&response_bytes)
            .map_err(|e| SysError::ApiError(format!("failed to parse host response: {e}")))?;

        let (body, truncated) = truncate_body(response.body, MAX_RESPONSE_BODY_LEN);

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

    // -- HttpMethod parsing --

    #[test]
    fn method_defaults_to_get() {
        let method: Option<String> = None;
        let parsed: HttpMethod = method.as_deref().unwrap_or("GET").parse().unwrap();
        assert_eq!(parsed.as_str(), "GET");
    }

    #[test]
    fn method_case_insensitive() {
        assert_eq!("post".parse::<HttpMethod>().unwrap().as_str(), "POST");
        assert_eq!("Get".parse::<HttpMethod>().unwrap().as_str(), "GET");
    }

    #[test]
    fn method_accepts_all_standard() {
        for m in ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"] {
            assert!(m.parse::<HttpMethod>().is_ok(), "should accept {m}");
        }
    }

    #[test]
    fn method_rejects_trace() {
        let err = "TRACE".parse::<HttpMethod>().unwrap_err();
        assert_eq!(err, "unsupported HTTP method: TRACE");
    }

    #[test]
    fn method_rejects_arbitrary() {
        assert!("FROBNICATE".parse::<HttpMethod>().is_err());
    }

    // -- FetchResult serialization --

    #[test]
    fn fetch_result_preserves_error_status() {
        let result = FetchResult {
            status: 404,
            headers: HashMap::new(),
            body: "Not Found".to_string(),
            truncated: false,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"status\":404"));
        // truncated field omitted when false
        assert!(!json.contains("truncated"));
    }

    #[test]
    fn fetch_result_includes_truncated_when_true() {
        let result = FetchResult {
            status: 200,
            headers: HashMap::new(),
            body: "partial...".to_string(),
            truncated: true,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"truncated\":true"));
    }

    // -- Body truncation --

    #[test]
    fn truncate_short_body_unchanged() {
        let (body, truncated) = truncate_body("hello".to_string(), 100);
        assert_eq!(body, "hello");
        assert!(!truncated);
    }

    #[test]
    fn truncate_exact_limit_unchanged() {
        let input = "a".repeat(200);
        let (body, truncated) = truncate_body(input.clone(), 200);
        assert_eq!(body, input);
        assert!(!truncated);
    }

    #[test]
    fn truncate_long_body() {
        let input = "a".repeat(300);
        let (body, truncated) = truncate_body(input, 200);
        assert!(truncated);
        assert!(body.contains("[...truncated, 300 bytes total]"));
        let prefix_end = body.find("\n\n[...truncated").expect("marker missing");
        assert_eq!(prefix_end, 200);
    }

    #[test]
    fn truncate_at_multibyte_char_boundary() {
        // Each emoji is 4 bytes
        let input = "\u{1F600}".repeat(100); // 400 bytes
        let (body, truncated) = truncate_body(input, 10);
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
