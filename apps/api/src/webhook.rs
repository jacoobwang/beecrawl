use std::collections::HashMap;

use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

use crate::models::FirecrawlWebhook;

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("invalid_webhook: {0}")]
    Invalid(String),
    #[error("webhook_delivery_failed: {0}")]
    Delivery(String),
}

pub fn validate(webhook: &FirecrawlWebhook) -> Result<(), WebhookError> {
    let config = webhook.config();
    let url =
        url::Url::parse(&config.url).map_err(|error| WebhookError::Invalid(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(WebhookError::Invalid(
            "webhook URL must use http or https".to_string(),
        ));
    }
    if config
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("x-firecrawl-signature"))
    {
        return Err(WebhookError::Invalid(
            "X-Firecrawl-Signature cannot be supplied by the caller".to_string(),
        ));
    }
    for event in &config.events {
        if !matches!(event.as_str(), "started" | "page" | "completed" | "failed") {
            return Err(WebhookError::Invalid(format!(
                "unsupported webhook event: {event}"
            )));
        }
    }
    webhook_secret()?;
    request_headers(&config.headers)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn deliver(
    client: &reqwest::Client,
    webhook: &FirecrawlWebhook,
    job_type: &str,
    job_id: Uuid,
    event: &str,
    success: bool,
    data: Value,
    error: Option<&str>,
) -> Result<(), WebhookError> {
    let config = webhook.config();
    if !config.events.iter().any(|configured| configured == event) {
        return Ok(());
    }
    let namespace = match job_type {
        "batch_scrape" => "batch_scrape",
        "monitor" => "monitor",
        _ => "crawl",
    };
    let mut payload = json!({
        "success": success,
        "type": format!("{namespace}.{event}"),
        "id": job_id.to_string(),
        "webhookId": Uuid::new_v4().to_string(),
        "data": data,
    });
    if let Some(error) = error {
        payload["error"] = json!(error);
    }
    if !config.metadata.is_empty() {
        payload["metadata"] = json!(config.metadata);
    }
    let body =
        serde_json::to_vec(&payload).map_err(|error| WebhookError::Invalid(error.to_string()))?;
    let signature = sign(&body, &webhook_secret()?)?;
    let mut headers = request_headers(&config.headers)?;
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("x-firecrawl-signature"),
        HeaderValue::from_str(&signature)
            .map_err(|error| WebhookError::Invalid(error.to_string()))?,
    );
    let response = client
        .post(config.url)
        .headers(headers)
        .body(body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| WebhookError::Delivery(error.to_string()))?;
    if !response.status().is_success() {
        return Err(WebhookError::Delivery(format!(
            "webhook returned HTTP {}",
            response.status()
        )));
    }
    Ok(())
}

fn sign(body: &[u8], secret: &str) -> Result<String, WebhookError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|error| WebhookError::Invalid(error.to_string()))?;
    mac.update(body);
    Ok(format!("sha256={:x}", mac.finalize().into_bytes()))
}

fn webhook_secret() -> Result<String, WebhookError> {
    std::env::var("BEECRAWL_WEBHOOK_HMAC_SECRET")
        .or_else(|_| std::env::var("SELF_HOSTED_WEBHOOK_HMAC_SECRET"))
        .ok()
        .filter(|secret| !secret.is_empty())
        .ok_or_else(|| {
            WebhookError::Invalid(
                "Set BEECRAWL_WEBHOOK_HMAC_SECRET to enable signed webhooks".to_string(),
            )
        })
}

fn request_headers(headers: &HashMap<String, String>) -> Result<HeaderMap, WebhookError> {
    let mut result = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| WebhookError::Invalid(error.to_string()))?;
        let value = HeaderValue::from_str(value)
            .map_err(|error| WebhookError::Invalid(error.to_string()))?;
        result.insert(name, value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_signature_is_hmac_sha256() {
        assert_eq!(
            sign(b"The quick brown fox jumps over the lazy dog", "key").unwrap(),
            "sha256=f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }
}
