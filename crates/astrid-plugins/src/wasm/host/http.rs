use extism::{CurrentPlugin, Error, UserData, Val};

#[cfg(feature = "http")]
use astrid_core::plugin_abi::HttpResponse;
use astrid_core::plugin_abi::KeyValuePair;

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

    let request_json: String = plugin.memory_get_val(&inputs[0])?;

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
        let method = req.method.clone();
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
        let response = handle.block_on(async {
            perform_http_request(&req.method, &req.url, &req.headers, req.body.as_deref()).await
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
async fn perform_http_request(
    method: &str,
    url: &str,
    headers: &[KeyValuePair],
    body: Option<&str>,
) -> Result<HttpResponse, Error> {
    let client = reqwest::Client::new();
    let mut builder = match method.to_uppercase().as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "PATCH" => client.patch(url),
        "HEAD" => client.head(url),
        other => {
            return Err(Error::msg(format!("unsupported HTTP method: {other}")));
        },
    };

    for kv in headers {
        builder = builder.header(&kv.key, &kv.value);
    }

    if let Some(b) = body {
        builder = builder.body(b.to_string());
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| Error::msg(format!("HTTP request failed: {e}")))?;

    let status = resp.status().as_u16();
    let resp_headers: Vec<KeyValuePair> = resp
        .headers()
        .iter()
        .map(|(k, v)| KeyValuePair {
            key: k.to_string(),
            value: v.to_str().unwrap_or("").to_string(),
        })
        .collect();
    let resp_body = resp
        .text()
        .await
        .map_err(|e| Error::msg(format!("failed to read HTTP response body: {e}")))?;

    Ok(HttpResponse {
        status,
        headers: resp_headers,
        body: resp_body,
    })
}
