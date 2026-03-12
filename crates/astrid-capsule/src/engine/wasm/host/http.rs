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
fn is_safe_ip(mut ip: std::net::IpAddr) -> bool {
    // Escape hatch for integration tests that need to spin up local servers
    if std::env::var("ASTRID_TEST_ALLOW_LOCAL_IP").is_ok() {
        return true;
    }

    // Global escape hatch for deployments that require plugins to access internal network services
    if std::env::var("ASTRID_ALLOW_LOCAL_IPS").is_ok() {
        return true;
    }

    if let std::net::IpAddr::V6(ipv6) = ip {
        if let Some(ipv4) = ipv6.to_ipv4_mapped() {
            ip = std::net::IpAddr::V4(ipv4);
        } else if let Some(ipv4) = ipv6.to_ipv4() {
            ip = std::net::IpAddr::V4(ipv4);
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

// ── Host function implementation ─────────────────────────────────────

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

    // Check capability via security gate (which will check if the host is allowed)
    if let Some(gate) = security {
        let url_obj = reqwest::Url::parse(&req.url)
            .map_err(|e| Error::msg(format!("invalid url {}: {e}", req.url)))?;
        let _ = url_obj
            .host_str()
            .ok_or_else(|| Error::msg("URL missing host"))?;

        let pid = capsule_id.clone();
        let full_url = req.url.clone();
        let m = req.method.clone();
        let check = util::bounded_block_on(&runtime_handle, &host_semaphore, async move {
            gate.check_http_request(&pid, &m, &full_url).await
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied network access: {reason}"
            )));
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .dns_resolver(Arc::new(SafeDnsResolver))
        .build()
        .map_err(|e| Error::msg(format!("failed to build http client: {e}")))?;

    let method = match req.method.to_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        other => return Err(Error::msg(format!("unsupported http method: {other}"))),
    };

    let mut headers = HeaderMap::new();
    for (k, v) in req.headers {
        let h_name = HeaderName::from_bytes(k.as_bytes())
            .map_err(|e| Error::msg(format!("invalid header name {k}: {e}")))?;
        let h_value = HeaderValue::from_str(&v)
            .map_err(|e| Error::msg(format!("invalid header value {v}: {e}")))?;
        headers.insert(h_name, h_value);
    }

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
}
