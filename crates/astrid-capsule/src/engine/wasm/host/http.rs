use extism::{CurrentPlugin, Error, UserData, Val};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
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

#[allow(clippy::needless_pass_by_value)]
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

    let (capsule_id, security, runtime_handle) = {
        let ud = user_data.get()?;
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        (
            state.capsule_id.as_str().to_owned(),
            state.security.clone(),
            state.runtime_handle.clone(),
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
        let check = tokio::task::block_in_place(|| {
            runtime_handle
                .block_on(async move { gate.check_http_request(&pid, &m, &full_url).await })
        });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied network access: {reason}"
            )));
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
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

    let response = tokio::task::block_in_place(|| {
        runtime_handle.block_on(async move { request_builder.send().await })
    })
    .map_err(|e| Error::msg(format!("http request failed: {e}")))?;

    let status = response.status().as_u16();

    let mut resp_headers = std::collections::HashMap::new();
    for (k, v) in response.headers() {
        if let Ok(v_str) = v.to_str() {
            resp_headers.insert(k.as_str().to_string(), v_str.to_string());
        }
    }

    let body_result = tokio::task::block_in_place(|| {
        runtime_handle.block_on(async move {
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
        })
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
