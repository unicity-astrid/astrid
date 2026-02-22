use extism::{CurrentPlugin, Error, UserData, Val};

#[cfg(feature = "http")]
use astrid_core::plugin_abi::HttpResponse;
use astrid_core::plugin_abi::KeyValuePair;

use crate::wasm::host::util;
use crate::wasm::host_state::HostState;

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn astrid_http_request_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct HttpRequest {
        method: String,
        url: String,
        #[serde(default)]
        headers: Vec<KeyValuePair>,
        #[serde(default)]
        body: Option<String>,
    }

    let request_json: String =
        util::get_safe_string(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;

    let req: HttpRequest = serde_json::from_str(&request_json)
        .map_err(|e| Error::msg(format!("invalid HTTP request JSON: {e}")))?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let method = req.method.to_uppercase();
        let url = req.url.clone();
        let check =
            handle.block_on(async move { gate.check_http_request(&pid, &method, &url).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied HTTP request: {reason}"
            )));
        }
    }

    #[cfg(feature = "http")]
    {
        let HttpRequest {
            method: req_method,
            url: req_url,
            headers: req_headers,
            body: req_body,
        } = req;
        let response = handle.block_on(async {
            perform_http_request(&req_method, &req_url, &req_headers, req_body).await
        })?;
        let response_json = serde_json::to_string(&response)
            .map_err(|e| Error::msg(format!("failed to serialize HTTP response: {e}")))?;
        let mem = plugin.memory_new(&response_json)?;
        outputs[0] = plugin.memory_to_val(mem);
        Ok(())
    }

    #[cfg(not(feature = "http"))]
    {
        let _ = outputs;
        Err(Error::msg(
            "HTTP support not enabled — enable the 'http' feature on astrid-plugins",
        ))
    }
}

#[cfg(feature = "http")]
static HTTP_CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    // Note: The 30-second timeout is a hard ceiling for total request duration across all plugins.
    // If a plugin needs to download a large multi-megabyte payload or query a slow API, it will abort here.
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .dns_resolver(std::sync::Arc::new(astrid_core::http::SafeDnsResolver))
        .build()
        .expect("failed to build global HTTP client")
});

#[cfg(feature = "http")]
async fn perform_http_request(
    method: &str,
    url: &str,
    headers: &[KeyValuePair],
    body: Option<String>,
) -> Result<HttpResponse, Error> {
    let client = &*HTTP_CLIENT;

    let req_method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
        .map_err(|_| Error::msg(format!("invalid HTTP method: {method}")))?;

    let parsed_url = url::Url::parse(url).map_err(|e| Error::msg(format!("invalid URL: {e}")))?;

    if let Some(host) = parsed_url.host() {
        let ip_to_check = match host {
            url::Host::Ipv4(ipv4) => Some(std::net::IpAddr::V4(ipv4)),
            url::Host::Ipv6(ipv6) => Some(std::net::IpAddr::V6(ipv6)),
            url::Host::Domain(_) => None,
        };
        if let Some(ip) = ip_to_check
            && !astrid_core::http::is_safe_ip(ip)
        {
            return Err(Error::msg(
                "URL host is an unauthorized private or local IP address",
            ));
        }
    }

    let mut builder = client.request(req_method, parsed_url.as_str());

    for kv in headers {
        if kv.key.eq_ignore_ascii_case("host")
            || kv.key.eq_ignore_ascii_case("connection")
            || kv.key.eq_ignore_ascii_case("upgrade")
            || kv.key.eq_ignore_ascii_case("content-length")
            || kv.key.eq_ignore_ascii_case("transfer-encoding")
            || kv.key.eq_ignore_ascii_case("x-forwarded-host")
            || kv.key.eq_ignore_ascii_case("x-forwarded-for")
            || kv.key.eq_ignore_ascii_case("x-forwarded-proto")
            || kv.key.eq_ignore_ascii_case("forwarded")
            || kv.key.eq_ignore_ascii_case("via")
        {
            tracing::warn!("WASM plugin attempted to set restricted header: {}", kv.key);
            continue;
        }
        let h_name = reqwest::header::HeaderName::try_from(kv.key.as_str())
            .map_err(|e| Error::msg(format!("invalid header name '{}': {e}", kv.key)))?;
        let h_value = reqwest::header::HeaderValue::try_from(kv.value.as_str())
            .map_err(|e| Error::msg(format!("invalid header value for '{}': {e}", kv.key)))?;
        builder = builder.header(h_name, h_value);
    }

    if let Some(b) = body {
        builder = builder.body(b);
    }

    let mut resp = builder
        .send()
        .await
        .map_err(|e| Error::msg(format!("HTTP request failed: {e}")))?;

    let status = resp.status().as_u16();
    let resp_headers: Vec<KeyValuePair> = resp
        .headers()
        .iter()
        .map(|(k, v)| KeyValuePair {
            key: k.to_string(),
            value: String::from_utf8_lossy(v.as_bytes()).into_owned(),
        })
        .collect();

    let content_length = resp.content_length().unwrap_or(0);
    if content_length > util::MAX_GUEST_PAYLOAD_LEN {
        return Err(Error::msg(
            "HTTP response body exceeds maximum allowed guest payload limit",
        ));
    }

    let mut resp_bytes = Vec::new();

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::msg(format!("failed to read chunk: {e}")))?
    {
        if resp_bytes.len().saturating_add(chunk.len()) as u64 > util::MAX_GUEST_PAYLOAD_LEN {
            return Err(Error::msg(
                "HTTP response body exceeds maximum allowed guest payload limit",
            ));
        }
        resp_bytes.extend_from_slice(&chunk);
    }

    let resp_body = String::from_utf8_lossy(&resp_bytes).into_owned();

    Ok(HttpResponse {
        status,
        headers: resp_headers,
        body: resp_body,
    })
}
