use extism::{CurrentPlugin, Error, UserData, Val};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::sync::Arc;
use std::time::Duration;

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

#[derive(serde::Deserialize)]
struct HttpRequest {
    url: String,
    #[serde(default = "default_method")]
    method: String,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    body: Option<String>,
}

fn default_method() -> String {
    "GET".to_string()
}

#[derive(serde::Serialize)]
struct HttpResponse {
    status: u16,
    headers: std::collections::HashMap<String, String>,
    body: String,
}

// ── SSRF prevention ──────────────────────────────────────────────────

/// A DNS resolver that prevents SSRF by blocking resolution to local,
/// private, or multicast IP addresses.
#[derive(Clone)]
struct SafeDnsResolver;

impl reqwest::dns::Resolve for SafeDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let name_str = name.as_str().to_string();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((name_str.as_str(), 0))
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            let mut safe_addrs = Vec::new();
            for addr in addrs {
                if is_safe_ip(addr.ip()) {
                    safe_addrs.push(addr);
                }
            }

            if safe_addrs.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "DNS resolved to an unauthorized private or local IP address",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }

            let iter: reqwest::dns::Addrs = Box::new(safe_addrs.into_iter());
            Ok(iter)
        })
    }
}

/// Checks if an IP address is safe to connect to (not local, private, or multicast).
/// Cached SSRF escape-hatch check. Evaluated once per process; logs a
/// warning on first access if either env var is set.
static SSRF_BYPASS: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
    if std::env::var("ASTRID_TEST_ALLOW_LOCAL_IP").is_ok() {
        tracing::warn!(
            "ASTRID_TEST_ALLOW_LOCAL_IP is set - SSRF protection disabled for ALL capsules"
        );
        return true;
    }
    if std::env::var("ASTRID_ALLOW_LOCAL_IPS").is_ok() {
        tracing::warn!(
            "ASTRID_ALLOW_LOCAL_IPS is set - SSRF protection disabled for ALL capsules. \
             Private/loopback IP ranges are reachable by every loaded capsule."
        );
        return true;
    }
    false
});

fn is_safe_ip(mut ip: std::net::IpAddr) -> bool {
    if *SSRF_BYPASS {
        return true;
    }

    if let std::net::IpAddr::V6(ipv6) = ip {
        if let Some(ipv4) = ipv6.to_ipv4_mapped() {
            ip = std::net::IpAddr::V4(ipv4);
        } else if ipv6.segments()[..6].iter().all(|&s| s == 0) {
            // IPv4-compatible addresses (::x.x.x.x) are deprecated by RFC 4291
            // but must still be blocked (e.g. ::127.0.0.1 is loopback).
            let [.., hi, lo] = ipv6.segments();
            let [a, b] = hi.to_be_bytes();
            let [c, d] = lo.to_be_bytes();
            ip = std::net::IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d));
        }
    }

    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }

    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            let is_private = octets[0] == 10
                || octets[0] == 0       // 0.0.0.0/8
                || octets[0] == 255     // Broadcast
                || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 169 && octets[1] == 254)
                || (octets[0] == 100 && octets[1] >= 64 && octets[1] <= 127)
                || octets[0] == 127;
            !is_private
        },
        std::net::IpAddr::V6(ipv6) => {
            let segments = ipv6.segments();
            let is_private = (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80;
            !is_private
        },
    }
}

// ── Shared helpers ───────────────────────────────────────────────────

/// Parse and validate an HTTP method string.
fn parse_method(method: &str) -> Result<reqwest::Method, Error> {
    match method.to_uppercase().as_str() {
        "GET" => Ok(reqwest::Method::GET),
        "POST" => Ok(reqwest::Method::POST),
        "PUT" => Ok(reqwest::Method::PUT),
        "DELETE" => Ok(reqwest::Method::DELETE),
        "PATCH" => Ok(reqwest::Method::PATCH),
        "HEAD" => Ok(reqwest::Method::HEAD),
        "OPTIONS" => Ok(reqwest::Method::OPTIONS),
        other => Err(Error::msg(format!("unsupported http method: {other}"))),
    }
}

/// Build a `HeaderMap` from a string→string map.
fn build_headers(raw: std::collections::HashMap<String, String>) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    for (k, v) in raw {
        let h_name = HeaderName::from_bytes(k.as_bytes())
            .map_err(|e| Error::msg(format!("invalid header name {k}: {e}")))?;
        let h_value = HeaderValue::from_str(&v)
            .map_err(|e| Error::msg(format!("invalid header value {v}: {e}")))?;
        headers.insert(h_name, h_value);
    }
    Ok(headers)
}

/// Run the security gate check for an HTTP request.
fn check_http_security(
    security: &Option<Arc<dyn crate::security::CapsuleSecurityGate>>,
    capsule_id: String,
    req: &HttpRequest,
    runtime_handle: &tokio::runtime::Handle,
    host_semaphore: &Arc<tokio::sync::Semaphore>,
) -> Result<(), Error> {
    if let Some(gate) = security {
        let url_obj = reqwest::Url::parse(&req.url)
            .map_err(|e| Error::msg(format!("invalid url {}: {e}", req.url)))?;
        let _ = url_obj
            .host_str()
            .ok_or_else(|| Error::msg("URL missing host"))?;

        let full_url = req.url.clone();
        let m = req.method.clone();
        let gate = gate.clone();
        let check = util::bounded_block_on(runtime_handle, host_semaphore, async move {
            gate.check_http_request(&capsule_id, &m, &full_url).await
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied network access: {reason}"
            )));
        }
    }
    Ok(())
}

// ── Host function implementation (buffered) ──────────────────────────

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_http_request_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    // Use get_safe_bytes instead of getting a UTF-8 string directly
    let request_bytes: Vec<u8> =
        util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;

    let request_json = String::from_utf8(request_bytes)
        .map_err(|e| Error::msg(format!("failed to parse request as utf8: {e}")))?;

    let req: HttpRequest = serde_json::from_str(&request_json)
        .map_err(|e| Error::msg(format!("invalid http request json: {e}")))?;

    let (capsule_id, security, runtime_handle, host_semaphore) = {
        let ud = user_data.get()?;
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        (
            state.capsule_id.as_str().to_owned(),
            state.security.clone(),
            state.runtime_handle.clone(),
            state.host_semaphore.clone(),
        )
    };

    check_http_security(
        &security,
        capsule_id,
        &req,
        &runtime_handle,
        &host_semaphore,
    )?;

    // Security gate already validated the URL and capsule capabilities.
    // Skip SafeDnsResolver — it blocks legitimate local endpoints that
    // capsules with net=["*"] should reach (local LLM servers, etc.).
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| Error::msg(format!("failed to build http client: {e}")))?;

    let method = parse_method(&req.method)?;
    let headers = build_headers(req.headers)?;

    let mut request_builder = client.request(method, &req.url).headers(headers);

    if let Some(body) = req.body {
        request_builder = request_builder.body(body);
    }

    let response = util::bounded_block_on(&runtime_handle, &host_semaphore, async move {
        request_builder.send().await
    })
    .map_err(|e| Error::msg(format!("http request failed: {e}")))?;

    let status = response.status().as_u16();

    let mut resp_headers = std::collections::HashMap::new();
    for (k, v) in response.headers() {
        if let Ok(v_str) = v.to_str() {
            resp_headers.insert(k.as_str().to_string(), v_str.to_string());
        }
    }

    let body_result = util::bounded_block_on(&runtime_handle, &host_semaphore, async move {
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if bytes.len() + chunk.len() > util::MAX_GUEST_PAYLOAD_LEN as usize {
                return Err(format!(
                    "HTTP response exceeded maximum payload limit ({} bytes)",
                    util::MAX_GUEST_PAYLOAD_LEN
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| "response body is not valid UTF-8".to_string())
    });

    let body =
        body_result.map_err(|e| Error::msg(format!("failed to read http response body: {e}")))?;

    let resp_obj = HttpResponse {
        status,
        headers: resp_headers,
        body,
    };

    let resp_json = serde_json::to_string(&resp_obj)
        .map_err(|e| Error::msg(format!("failed to serialize http response: {e}")))?;

    let mem = plugin.memory_new(&resp_json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ── Host function implementation (streaming) ─────────────────────────

/// Maximum concurrent HTTP streaming responses per capsule.
const MAX_ACTIVE_HTTP_STREAMS: usize = 4;
/// Connect timeout for streaming HTTP requests (time to first byte).
const HTTP_STREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-chunk read timeout for streaming HTTP responses.
const HTTP_STREAM_READ_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(serde::Serialize)]
struct HttpStreamStartResponse {
    handle: String,
    status: u16,
    headers: std::collections::HashMap<String, String>,
}

/// Start a streaming HTTP request: send the request, wait for headers,
/// store the response body stream in `HostState`, and return the handle
/// along with status code and headers.
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_http_stream_start_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let request_bytes: Vec<u8> =
        util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;

    let request_json = String::from_utf8(request_bytes)
        .map_err(|e| Error::msg(format!("failed to parse request as utf8: {e}")))?;

    let req: HttpRequest = serde_json::from_str(&request_json)
        .map_err(|e| Error::msg(format!("invalid http request json: {e}")))?;

    let (capsule_id, security, runtime_handle, host_semaphore) = {
        let ud = user_data.get()?;
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

        // Check stream cap before doing any network I/O.
        if state.active_http_streams.len() >= MAX_ACTIVE_HTTP_STREAMS {
            return Err(Error::msg(format!(
                "HTTP stream cap reached ({}/{})",
                state.active_http_streams.len(),
                MAX_ACTIVE_HTTP_STREAMS
            )));
        }

        (
            state.capsule_id.as_str().to_owned(),
            state.security.clone(),
            state.runtime_handle.clone(),
            state.host_semaphore.clone(),
        )
    };

    check_http_security(
        &security,
        capsule_id,
        &req,
        &runtime_handle,
        &host_semaphore,
    )?;

    // Security gate already validated the URL and capsule capabilities.
    // Skip SafeDnsResolver for streaming — it blocks legitimate local
    // LLM endpoints (127.0.0.1, 192.168.*) that capsules with net=["*"]
    // should be able to reach.
    let client = reqwest::Client::builder()
        .connect_timeout(HTTP_STREAM_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| Error::msg(format!("failed to build http client: {e}")))?;

    let method = parse_method(&req.method)?;
    let headers = build_headers(req.headers)?;

    let mut request_builder = client.request(method, &req.url).headers(headers);
    if let Some(body) = req.body {
        request_builder = request_builder.body(body);
    }

    // Send request and wait for headers (not body).
    let response = util::bounded_block_on(&runtime_handle, &host_semaphore, async move {
        request_builder.send().await
    })
    .map_err(|e| Error::msg(format!("http stream request failed: {e}")))?;

    let status = response.status().as_u16();

    let mut resp_headers = std::collections::HashMap::new();
    for (k, v) in response.headers() {
        if let Ok(v_str) = v.to_str() {
            resp_headers.insert(k.as_str().to_string(), v_str.to_string());
        }
    }

    // Store the response body stream and allocate a handle.
    let handle_id = {
        let ud = user_data.get()?;
        let mut state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

        let handle_id = state.next_http_stream_id;
        state.next_http_stream_id = state
            .next_http_stream_id
            .checked_add(1)
            .ok_or_else(|| Error::msg("HTTP stream handle ID space exhausted"))?;

        debug_assert!(
            !state.active_http_streams.contains_key(&handle_id),
            "HTTP stream handle ID collision"
        );
        state
            .active_http_streams
            .insert(handle_id, Arc::new(tokio::sync::Mutex::new(response)));
        handle_id
    };

    let resp = HttpStreamStartResponse {
        handle: handle_id.to_string(),
        status,
        headers: resp_headers,
    };
    let resp_json = serde_json::to_string(&resp)
        .map_err(|e| Error::msg(format!("failed to serialize stream start response: {e}")))?;

    let mem = plugin.memory_new(&resp_json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

/// Read the next chunk from a streaming HTTP response. Returns the raw
/// bytes, or empty bytes when the stream is exhausted (EOF).
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_http_stream_read_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let handle_str = util::get_safe_string(plugin, &inputs[0], 1024)?;
    let handle_id: u64 = handle_str
        .parse()
        .map_err(|e| Error::msg(format!("invalid HTTP stream handle: {e}")))?;

    let (response_arc, rt_handle, cancel_token, host_semaphore) = {
        let ud = user_data.get()?;
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

        let response = state
            .active_http_streams
            .get(&handle_id)
            .ok_or_else(|| Error::msg("HTTP stream handle not found"))?
            .clone();

        (
            response,
            state.runtime_handle.clone(),
            state.cancel_token.clone(),
            state.host_semaphore.clone(),
        )
    };

    let result =
        util::bounded_block_on_cancellable(&rt_handle, &host_semaphore, &cancel_token, async {
            let mut resp = response_arc.lock().await;
            tokio::time::timeout(HTTP_STREAM_READ_TIMEOUT, resp.chunk()).await
        });

    let chunk_data = match result {
        // Cancelled (capsule unloading).
        None => Vec::new(),
        // Timeout waiting for next chunk.
        Some(Err(_elapsed)) => {
            return Err(Error::msg(format!(
                "HTTP stream read timed out after {}s",
                HTTP_STREAM_READ_TIMEOUT.as_secs()
            )));
        },
        // Network/body error.
        Some(Ok(Err(e))) => {
            return Err(Error::msg(format!("HTTP stream read error: {e}")));
        },
        // Got a chunk.
        Some(Ok(Ok(Some(bytes)))) => bytes.to_vec(),
        // EOF — stream exhausted.
        Some(Ok(Ok(None))) => Vec::new(),
    };

    let mem = plugin.memory_new(&chunk_data)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

/// Close a streaming HTTP response handle, releasing host-side resources.
/// Idempotent — closing an already-closed handle is a no-op.
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_http_stream_close_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let handle_str = util::get_safe_string(plugin, &inputs[0], 1024)?;
    let handle_id: u64 = handle_str
        .parse()
        .map_err(|e| Error::msg(format!("invalid HTTP stream handle: {e}")))?;

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    // Idempotent: silently ignore if the handle was already removed.
    let _ = state.active_http_streams.remove(&handle_id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use std::str::FromStr;

    #[test]
    fn safe_public_ips() {
        assert!(is_safe_ip(IpAddr::from_str("8.8.8.8").unwrap()));
        assert!(is_safe_ip(IpAddr::from_str("1.1.1.1").unwrap()));
        assert!(is_safe_ip(IpAddr::from_str("198.51.100.1").unwrap()));
        assert!(is_safe_ip(
            IpAddr::from_str("2001:4860:4860::8888").unwrap()
        ));
    }

    #[test]
    fn blocks_loopback_and_unspecified() {
        assert!(!is_safe_ip(IpAddr::from_str("127.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("::1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("0.0.0.0").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("::").unwrap()));
    }

    #[test]
    fn blocks_zero_block() {
        assert!(!is_safe_ip(IpAddr::from_str("0.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("0.255.255.255").unwrap()));
    }

    #[test]
    fn blocks_rfc1918_private() {
        assert!(!is_safe_ip(IpAddr::from_str("10.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("10.255.255.255").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("172.16.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("172.31.255.255").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("192.168.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("192.168.255.255").unwrap()));
    }

    #[test]
    fn blocks_link_local_and_cgnat() {
        assert!(!is_safe_ip(IpAddr::from_str("169.254.169.254").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("100.64.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("100.127.255.255").unwrap()));
    }

    #[test]
    fn blocks_private_ipv6() {
        assert!(!is_safe_ip(IpAddr::from_str("fc00::1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("fd00::1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("fe80::1").unwrap()));
    }

    #[test]
    fn blocks_ipv4_mapped_ipv6_bypass() {
        assert!(!is_safe_ip(IpAddr::from_str("::ffff:127.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("::ffff:10.0.0.1").unwrap()));
        assert!(!is_safe_ip(
            IpAddr::from_str("::ffff:169.254.169.254").unwrap()
        ));
    }

    #[test]
    fn blocks_ipv4_compatible_ipv6_bypass() {
        // IPv4-compatible (deprecated RFC 4291, no ::ffff prefix).
        // These exercise the explicit segment extraction that replaced
        // the deprecated Ipv6Addr::to_ipv4().
        assert!(!is_safe_ip(IpAddr::from_str("::127.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("::10.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("::169.254.169.254").unwrap()));
        // ::1 is IPv6 loopback; after compatible-branch extraction it
        // becomes 0.0.0.1, blocked by the 0.0.0.0/8 check (not loopback).
        assert!(!is_safe_ip(IpAddr::from_str("::0.0.0.1").unwrap()));
    }
}
