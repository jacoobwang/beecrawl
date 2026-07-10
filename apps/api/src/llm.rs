use std::collections::HashMap;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::models::LlmProviderConfig;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o-mini";
const MAX_EXTRACT_CONTEXT_CHARS: usize = 24_000;

#[derive(Debug, Clone)]
pub struct ResolvedLlmProvider {
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("unsupported_llm_provider: {0}")]
    UnsupportedProvider(String),
    #[error("missing_llm_api_key: Set BEECRAWL_LLM_API_KEY or pass provider.api_key")]
    MissingApiKey,
    #[error("llm_request_failed: {0}")]
    RequestFailed(String),
    #[error("llm_response_invalid: {0}")]
    InvalidResponse(String),
}

impl LlmError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::UnsupportedProvider(_) | Self::MissingApiKey => StatusCode::BAD_REQUEST,
            Self::RequestFailed(_) | Self::InvalidResponse(_) => StatusCode::BAD_GATEWAY,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedProvider(_) => "unsupported_llm_provider",
            Self::MissingApiKey => "missing_llm_api_key",
            Self::RequestFailed(_) => "llm_request_failed",
            Self::InvalidResponse(_) => "llm_response_invalid",
        }
    }
}

pub fn resolve_provider(
    request_provider: Option<&LlmProviderConfig>,
) -> Result<Option<ResolvedLlmProvider>, LlmError> {
    let provider = request_provider
        .and_then(|config| non_empty(config.provider.as_str()))
        .or_else(|| env_value("BEECRAWL_LLM_PROVIDER"))
        .unwrap_or_else(|| "openai-compatible".to_string());

    let normalized_provider = provider.to_lowercase();
    if !matches!(
        normalized_provider.as_str(),
        "openai-compatible" | "openai" | "dashscope" | "deepseek"
    ) {
        return Err(LlmError::UnsupportedProvider(provider));
    }

    let api_key = request_provider
        .and_then(|config| config.api_key.as_deref().and_then(non_empty))
        .or_else(|| env_value("BEECRAWL_LLM_API_KEY"))
        .or_else(|| env_value("OPENAI_API_KEY"));
    let Some(api_key) = api_key else {
        if request_provider.is_some() {
            return Err(LlmError::MissingApiKey);
        }
        return Ok(None);
    };

    let base_url = request_provider
        .and_then(|config| config.base_url.as_deref().and_then(non_empty))
        .or_else(|| env_value("BEECRAWL_LLM_BASE_URL"))
        .or_else(|| env_value("OPENAI_BASE_URL"))
        .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string());
    let model = request_provider
        .and_then(|config| config.model.as_deref().and_then(non_empty))
        .or_else(|| env_value("BEECRAWL_LLM_MODEL"))
        .or_else(|| env_value("OPENAI_MODEL"))
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    Ok(Some(ResolvedLlmProvider {
        provider: "openai-compatible".to_string(),
        api_key,
        base_url,
        model,
    }))
}

pub async fn extract_structured_data(
    client: &reqwest::Client,
    provider: &ResolvedLlmProvider,
    url: &str,
    schema: &HashMap<String, String>,
    markdown: &str,
) -> Result<HashMap<String, Option<String>>, LlmError> {
    let system = "You extract structured data from scraped web page Markdown. Return only valid JSON. Do not include markdown fences, comments, or extra text.";
    let user = build_extract_prompt(url, schema, markdown);
    let request = ChatCompletionRequest {
        model: provider.model.clone(),
        messages: vec![
            ChatMessage {
                role: "system",
                content: system.to_string(),
            },
            ChatMessage {
                role: "user",
                content: user,
            },
        ],
        temperature: 0.0,
        response_format: Some(json!({ "type": "json_object" })),
    };
    let endpoint = chat_completions_url(&provider.base_url);
    let response = client
        .post(endpoint)
        .bearer_auth(&provider.api_key)
        .json(&request)
        .send()
        .await
        .map_err(|err| LlmError::RequestFailed(err.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(LlmError::RequestFailed(format!(
            "OpenAI-compatible API error ({status}): {body}"
        )));
    }
    let body: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|err| LlmError::InvalidResponse(err.to_string()))?;
    let content = body
        .choices
        .first()
        .map(|choice| choice.message.content.trim())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| LlmError::InvalidResponse("missing assistant content".to_string()))?;
    parse_extraction_json(content, schema)
}

pub async fn extract_structured_value(
    client: &reqwest::Client,
    provider: &ResolvedLlmProvider,
    urls: &[String],
    schema: &Value,
    markdown: &str,
    instructions: Option<&str>,
) -> Result<Value, LlmError> {
    let schema_json = serde_json::to_string_pretty(schema).unwrap_or_else(|_| "{}".to_string());
    let request = ChatCompletionRequest {
        model: provider.model.clone(),
        messages: vec![
            ChatMessage {
                role: "system",
                content: "You extract structured data from scraped web pages. Return only JSON matching the supplied JSON Schema. Use null when evidence is unavailable."
                    .to_string(),
            },
            ChatMessage {
                role: "user",
                content: format!(
                    "URLs:\n{}\n\nInstructions:\n{}\n\nJSON Schema:\n{}\n\nPage Markdown:\n{}",
                    urls.join("\n"),
                    instructions.unwrap_or("Extract factual information from the supplied pages."),
                    schema_json,
                    truncate_context(markdown),
                ),
            },
        ],
        temperature: 0.0,
        response_format: Some(json!({ "type": "json_object" })),
    };
    let endpoint = chat_completions_url(&provider.base_url);
    let response = client
        .post(endpoint)
        .bearer_auth(&provider.api_key)
        .json(&request)
        .send()
        .await
        .map_err(|err| LlmError::RequestFailed(err.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(LlmError::RequestFailed(format!(
            "OpenAI-compatible API error ({status}): {body}"
        )));
    }
    let body: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|err| LlmError::InvalidResponse(err.to_string()))?;
    let content = body
        .choices
        .first()
        .map(|choice| choice.message.content.trim())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| LlmError::InvalidResponse("missing assistant content".to_string()))?;
    parse_json_value(content)
}

fn parse_json_value(content: &str) -> Result<Value, LlmError> {
    serde_json::from_str(content).or_else(|_| {
        let start = content.find('{').ok_or_else(|| {
            LlmError::InvalidResponse("response did not contain a JSON object".to_string())
        })?;
        let end = content.rfind('}').ok_or_else(|| {
            LlmError::InvalidResponse("response did not contain a JSON object".to_string())
        })?;
        serde_json::from_str(&content[start..=end])
            .map_err(|err| LlmError::InvalidResponse(err.to_string()))
    })
}

fn build_extract_prompt(url: &str, schema: &HashMap<String, String>, markdown: &str) -> String {
    let schema_json = serde_json::to_string_pretty(schema).unwrap_or_else(|_| "{}".to_string());
    let context = truncate_context(markdown);
    format!(
        "URL:\n{url}\n\nSchema fields and descriptions:\n{schema_json}\n\nReturn a JSON object with exactly these field names. Use string values when the value is present. Use null when the page does not contain enough evidence.\n\nPage Markdown:\n{context}"
    )
}

fn truncate_context(markdown: &str) -> String {
    if markdown.chars().count() <= MAX_EXTRACT_CONTEXT_CHARS {
        return markdown.to_string();
    }
    markdown.chars().take(MAX_EXTRACT_CONTEXT_CHARS).collect()
}

pub fn parse_extraction_json(
    content: &str,
    schema: &HashMap<String, String>,
) -> Result<HashMap<String, Option<String>>, LlmError> {
    let value: Value = match serde_json::from_str(content) {
        Ok(value) => value,
        Err(_) => {
            let start = content.find('{').ok_or_else(|| {
                LlmError::InvalidResponse("response did not contain a JSON object".to_string())
            })?;
            let end = content.rfind('}').ok_or_else(|| {
                LlmError::InvalidResponse("response did not contain a JSON object".to_string())
            })?;
            serde_json::from_str(&content[start..=end])
                .map_err(|err| LlmError::InvalidResponse(err.to_string()))?
        }
    };
    let object = value
        .as_object()
        .ok_or_else(|| LlmError::InvalidResponse("response JSON was not an object".to_string()))?;
    let mut data = HashMap::new();
    for field in schema.keys() {
        let value = object.get(field).and_then(json_value_to_string);
        data.insert(field.clone(), value);
    }
    Ok(data)
}

fn json_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => non_empty(value),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    }
}

fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(|value| non_empty(&value))
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    #[serde(rename = "response_format", skip_serializing_if = "Option::is_none")]
    response_format: Option<Value>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_completions_url_accepts_root_or_v1_base_url() {
        assert_eq!(
            chat_completions_url("https://dashscope.aliyuncs.com/compatible-mode"),
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn parse_extraction_json_keeps_only_schema_fields() {
        let schema = HashMap::from([
            ("company".to_string(), "Company name".to_string()),
            ("email".to_string(), "Contact email".to_string()),
            ("missing".to_string(), "Unknown field".to_string()),
        ]);
        let data = parse_extraction_json(
            r#"{"company":"BeeCrawl","email":null,"extra":"ignored"}"#,
            &schema,
        )
        .unwrap();
        assert_eq!(data.get("company"), Some(&Some("BeeCrawl".to_string())));
        assert_eq!(data.get("email"), Some(&None));
        assert_eq!(data.get("missing"), Some(&None));
        assert!(!data.contains_key("extra"));
    }
}
