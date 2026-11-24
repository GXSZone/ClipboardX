use std::error::Error as StdError;
use std::time::Duration;

use clipboardx_model::UpdaterError;
use reqwest::blocking::{Client, Response};

/// Builds a blocking HTTPS client, either honoring OS/environment proxies or bypassing them.
fn build_client(
    timeout: Duration,
    user_agent: &str,
    use_proxy: bool,
) -> Result<Client, UpdaterError> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(timeout)
        .user_agent(user_agent);
    if !use_proxy {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|err| UpdaterError::Network(format!("HTTP client init failed: {err}")))
}

/// Executes an HTTPS GET request using OS/environment proxy settings first,
/// falling back to direct `no_proxy()` when a proxy or connection path fails.
pub fn send_get_with_fallback(
    timeout: Duration,
    user_agent: &str,
    url: &str,
    accept_header: Option<&str>,
) -> Result<Response, UpdaterError> {
    if !url.trim().to_ascii_lowercase().starts_with("https://") {
        return Err(UpdaterError::Network(
            "update endpoint must use HTTPS".to_owned(),
        ));
    }

    let first_client = build_client(timeout, user_agent, true)?;
    match send_once(&first_client, url, accept_header) {
        Ok(resp) => Ok(resp),
        Err(first_err) => {
            let should_retry_direct =
                first_err.is_connect() || first_err.is_timeout() || first_err.is_request();
            let first_summary = format_reqwest_error(&first_err);
            tracing::warn!(url = %url, error = %first_summary, "primary update request failed");

            if should_retry_direct {
                let direct_client = build_client(timeout, user_agent, false)?;
                match send_once(&direct_client, url, accept_header) {
                    Ok(resp) => return Ok(resp),
                    Err(second_err) => {
                        let second_summary = format_reqwest_error(&second_err);
                        tracing::warn!(url = %url, error = %second_summary, "direct fallback request failed");
                        return Err(UpdaterError::Network(second_summary));
                    }
                }
            }

            Err(UpdaterError::Network(first_summary))
        }
    }
}

fn send_once(
    client: &Client,
    url: &str,
    accept_header: Option<&str>,
) -> Result<Response, reqwest::Error> {
    let mut req = client.get(url);
    if let Some(accept) = accept_header {
        req = req.header("Accept", accept);
    }
    req.send()?.error_for_status()
}

/// Formats a concise, diagnostic error description extracting root-cause details from `source()`.
pub fn format_reqwest_error(err: &reqwest::Error) -> String {
    if let Some(status) = err.status() {
        return format!("HTTP {}", status.as_u16());
    }

    let mut root_cause = String::new();
    let mut current: Option<&dyn StdError> = err.source();
    while let Some(cause) = current {
        root_cause = cause.to_string();
        current = cause.source();
    }

    if !root_cause.is_empty() {
        return root_cause;
    }

    if err.is_timeout() {
        "request timed out".to_owned()
    } else if err.is_connect() {
        "connection failed".to_owned()
    } else {
        err.to_string()
    }
}
