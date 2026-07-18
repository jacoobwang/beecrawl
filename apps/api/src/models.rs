use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct WebExtractScrapeRequest {
    pub url: String,
    #[serde(default = "default_formats")]
    pub formats: Vec<String>,
    pub location: Option<WebExtractLocation>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub wait_for_ms: u64,
    #[serde(default = "default_use_browser")]
    pub use_browser: String,
    #[serde(rename = "skipTlsVerification", default)]
    pub skip_tls_verification: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WebExtractLocation {
    pub country: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2ScrapeRequest {
    pub url: String,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(
        default = "default_formats",
        deserialize_with = "deserialize_firecrawl_formats"
    )]
    pub formats: Vec<String>,
    pub location: Option<WebExtractLocation>,
    #[serde(default = "default_timeout_milliseconds")]
    pub timeout: u64,
    #[serde(rename = "waitFor", default)]
    pub wait_for_ms: u64,
    #[serde(rename = "onlyMainContent")]
    pub only_main_content: Option<bool>,
    #[serde(rename = "skipTlsVerification")]
    pub skip_tls_verification: Option<bool>,
    #[serde(rename = "removeBase64Images")]
    pub remove_base64_images: Option<bool>,
    #[serde(rename = "fastMode")]
    pub fast_mode: Option<bool>,
    #[serde(rename = "blockAds")]
    pub block_ads: Option<bool>,
    #[serde(rename = "storeInCache")]
    pub store_in_cache: Option<bool>,
    #[serde(rename = "maxAge")]
    pub max_age: Option<u64>,
    pub mobile: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2ParseOptions {
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(
        default = "default_formats",
        deserialize_with = "deserialize_firecrawl_formats"
    )]
    pub formats: Vec<String>,
    #[serde(default = "default_timeout_milliseconds")]
    pub timeout: u64,
    #[serde(default)]
    pub parsers: Vec<FirecrawlV2FileParser>,
}

impl Default for FirecrawlV2ParseOptions {
    fn default() -> Self {
        Self {
            origin: None,
            formats: default_formats(),
            timeout: default_timeout_milliseconds(),
            parsers: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2Base64ParseRequest {
    #[serde(alias = "data")]
    pub base64: String,
    pub filename: String,
    #[serde(default)]
    pub options: FirecrawlV2ParseOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2FileParser {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(rename = "maxPages")]
    pub max_pages: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2CrawlRequest {
    pub url: String,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default = "default_firecrawl_crawl_limit")]
    pub limit: usize,
    #[serde(
        rename = "maxDiscoveryDepth",
        alias = "max_depth",
        alias = "maxDepth",
        default = "default_firecrawl_crawl_max_depth"
    )]
    pub max_discovery_depth: usize,
    #[serde(rename = "allowSubdomains", default)]
    pub allow_subdomains: bool,
    #[serde(rename = "deduplicateSimilarURLs")]
    pub deduplicate_similar_urls: Option<bool>,
    #[serde(rename = "crawlEntireDomain")]
    pub crawl_entire_domain: Option<bool>,
    #[serde(rename = "allowExternalLinks")]
    pub allow_external_links: Option<bool>,
    #[serde(rename = "ignoreRobotsTxt")]
    pub ignore_robots_txt: Option<bool>,
    #[serde(rename = "regexOnFullURL")]
    pub regex_on_full_url: Option<bool>,
    #[serde(rename = "zeroDataRetention")]
    pub zero_data_retention: Option<bool>,
    #[serde(rename = "ignoreQueryParameters", default)]
    pub ignore_query_parameters: bool,
    #[serde(rename = "scrapeOptions", default)]
    pub scrape_options: Option<FirecrawlV2ScrapeOptions>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2BatchScrapeRequest {
    pub urls: Vec<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(flatten)]
    pub scrape_options: FirecrawlV2ScrapeOptions,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2ScrapeOptions {
    #[serde(default, deserialize_with = "deserialize_firecrawl_formats")]
    pub formats: Vec<String>,
    #[serde(default = "default_timeout_milliseconds")]
    pub timeout: u64,
    #[serde(rename = "waitFor", default)]
    pub wait_for_ms: u64,
    #[serde(rename = "onlyMainContent")]
    pub only_main_content: Option<bool>,
    #[serde(rename = "skipTlsVerification")]
    pub skip_tls_verification: Option<bool>,
    #[serde(rename = "removeBase64Images")]
    pub remove_base64_images: Option<bool>,
    #[serde(rename = "fastMode")]
    pub fast_mode: Option<bool>,
    #[serde(rename = "blockAds")]
    pub block_ads: Option<bool>,
    #[serde(rename = "storeInCache")]
    pub store_in_cache: Option<bool>,
    #[serde(rename = "maxAge")]
    pub max_age: Option<u64>,
    pub mobile: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2ExtractRequest {
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub schema: Value,
    #[serde(rename = "enableWebSearch", default)]
    pub enable_web_search: bool,
    #[serde(rename = "showSources", default)]
    pub show_sources: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2SearchRequest {
    pub query: String,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default = "default_firecrawl_search_limit")]
    pub limit: usize,
    pub timeout: Option<u64>,
    #[serde(default)]
    pub sources: Vec<FirecrawlV2Source>,
    #[serde(rename = "scrapeOptions", default)]
    pub scrape_options: Option<FirecrawlV2ScrapeOptions>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum FirecrawlV2Source {
    Name(String),
    Config(FirecrawlV2SourceConfig),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2SourceConfig {
    pub r#type: String,
}

impl FirecrawlV2Source {
    pub fn name(&self) -> &str {
        match self {
            Self::Name(name) => name,
            Self::Config(config) => &config.r#type,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirecrawlV2MapRequest {
    pub url: String,
    #[serde(default)]
    pub origin: Option<String>,
    pub search: Option<String>,
    #[serde(default = "default_firecrawl_map_limit")]
    pub limit: usize,
    #[serde(
        rename = "includeSubdomains",
        default = "default_firecrawl_include_subdomains"
    )]
    pub include_subdomains: bool,
    #[serde(default = "default_sitemap")]
    pub sitemap: String,
    #[serde(
        rename = "ignoreQueryParameters",
        default = "default_ignore_query_parameters"
    )]
    pub ignore_query_parameters: bool,
}

impl From<FirecrawlV2MapRequest> for WebExtractMapRequest {
    fn from(request: FirecrawlV2MapRequest) -> Self {
        Self {
            url: request.url,
            search: request.search,
            limit: request.limit,
            include_subdomains: request.include_subdomains,
            sitemap: request.sitemap,
            ignore_sitemap: false,
            ignore_query_parameters: request.ignore_query_parameters,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WebExtractScrapeResponse {
    pub request_id: String,
    pub url: String,
    pub final_url: String,
    pub markdown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(rename = "rawHtml", default, skip_serializing_if = "Option::is_none")]
    pub raw_html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    pub metadata: WebExtractMetadata,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WebExtractMetadata {
    pub title: Option<String>,
    pub language: Option<String>,
    pub status_code: Option<u16>,
    pub provider: String,
    pub rendered: bool,
    pub elapsed_ms: Option<u128>,
}

#[derive(Debug, Deserialize)]
pub struct WebExtractMapRequest {
    pub url: String,
    pub search: Option<String>,
    #[serde(default = "default_map_limit")]
    pub limit: usize,
    #[serde(rename = "includeSubdomains", alias = "include_subdomains", default)]
    pub include_subdomains: bool,
    #[serde(default = "default_sitemap")]
    pub sitemap: String,
    #[serde(default)]
    pub ignore_sitemap: bool,
    #[serde(default = "default_ignore_query_parameters")]
    pub ignore_query_parameters: bool,
}

#[derive(Debug, Serialize)]
pub struct WebExtractMapResponse {
    pub request_id: String,
    pub url: String,
    pub links: Vec<String>,
    pub metadata: WebExtractMapMetadata,
}

#[derive(Debug, Serialize)]
pub struct WebExtractMapMetadata {
    pub provider: String,
    pub count: usize,
    pub elapsed_ms: Option<u128>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CrawlRequest {
    pub url: String,
    #[serde(default = "default_crawl_limit")]
    pub limit: usize,
    #[serde(rename = "maxDepth", default = "default_crawl_max_depth")]
    pub max_depth: usize,
    #[serde(rename = "includeSubdomains", default)]
    pub include_subdomains: bool,
    #[serde(
        rename = "ignoreQueryParameters",
        default = "default_ignore_query_parameters"
    )]
    pub ignore_query_parameters: bool,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(rename = "waitFor", default)]
    pub wait_for_ms: u64,
    #[serde(rename = "useBrowser", default = "default_use_browser")]
    pub use_browser: String,
    #[serde(rename = "skipTlsVerification", default)]
    pub skip_tls_verification: bool,
    #[serde(rename = "maxRetries", default = "default_crawl_max_retries")]
    pub max_retries: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BatchScrapeRequest {
    pub urls: Vec<String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub wait_for_ms: u64,
    #[serde(default = "default_use_browser")]
    pub use_browser: String,
    #[serde(rename = "skipTlsVerification", default)]
    pub skip_tls_verification: bool,
    #[serde(rename = "maxRetries", default = "default_crawl_max_retries")]
    pub max_retries: usize,
}

#[derive(Debug, Serialize)]
pub struct BatchScrapeEnqueueResponse {
    pub id: String,
    pub status: String,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct CrawlEnqueueResponse {
    pub id: String,
    pub url: String,
    pub status: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct CrawlStatusResponse {
    pub id: String,
    pub url: String,
    pub status: String,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub data: Vec<WebExtractScrapeResponse>,
    pub errors: Vec<CrawlError>,
    pub pagination: CrawlPagination,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CrawlStatusQuery {
    #[serde(default, alias = "skip")]
    pub offset: usize,
    #[serde(default = "default_crawl_status_limit")]
    pub limit: usize,
}

impl Default for CrawlStatusQuery {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: default_crawl_status_limit(),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct CrawlPagination {
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub next: Option<usize>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CrawlError {
    pub url: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct FirecrawlJobErrorsResponse {
    pub errors: Vec<FirecrawlJobError>,
    #[serde(rename = "robotsBlocked")]
    pub robots_blocked: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FirecrawlJobError {
    pub id: String,
    pub timestamp: Option<String>,
    pub url: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct ActiveCrawlsResponse {
    pub success: bool,
    pub crawls: Vec<ActiveCrawl>,
}

#[derive(Debug, Serialize)]
pub struct ActiveCrawl {
    pub id: String,
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub url: String,
    pub status: String,
    pub options: Value,
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    #[serde(default = "default_lang")]
    pub lang: String,
    #[serde(default = "default_country")]
    pub country: String,
    #[serde(rename = "scrapeOptions")]
    pub scrape_options: Option<SearchScrapeOptions>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SearchScrapeOptions {
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub wait_for_ms: u64,
    #[serde(default = "default_use_browser")]
    pub use_browser: String,
    #[serde(rename = "skipTlsVerification", default)]
    pub skip_tls_verification: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub query: String,
    pub results: Vec<SearchResult>,
    pub metadata: SearchMetadata,
}

#[derive(Debug, Serialize, Clone)]
pub struct SearchResult {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub markdown: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
    #[serde(rename = "scrapeError")]
    pub scrape_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchMetadata {
    pub provider: String,
    pub count: usize,
    #[serde(rename = "scrapedCount")]
    pub scraped_count: usize,
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: Option<u128>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractRequest {
    pub url: String,
    pub schema: HashMap<String, String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub wait_for_ms: u64,
    #[serde(default = "default_use_browser")]
    pub use_browser: String,
    pub provider: Option<LlmProviderConfig>,
    pub llm: Option<LlmProviderConfig>,
}

#[derive(Debug, Serialize)]
pub struct ExtractResponse {
    pub url: String,
    pub data: HashMap<String, Option<String>>,
    pub scrape: ScrapeResponse,
    pub metadata: ExtractMetadata,
}

#[derive(Debug, Serialize)]
pub struct ExtractMetadata {
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScrapeResponse {
    pub url: String,
    pub title: Option<String>,
    pub text: String,
    pub links: Vec<Link>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct Link {
    pub text: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LlmProviderConfig {
    #[serde(default = "default_llm_provider")]
    pub provider: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderPage {
    pub url: String,
    pub final_url: String,
    pub html: String,
    pub status_code: Option<u16>,
    pub title: Option<String>,
    pub language: Option<String>,
    pub provider: String,
    pub rendered: bool,
    pub screenshot: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BeeEngineScrapeResponse {
    #[serde(rename = "timeTaken")]
    pub time_taken: Option<u128>,
    pub content: String,
    pub url: String,
    #[serde(rename = "pageStatusCode")]
    pub page_status_code: Option<u16>,
    #[serde(rename = "pageError")]
    pub page_error: Option<String>,
    #[serde(rename = "responseHeaders", default)]
    pub response_headers: HashMap<String, String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
}

fn default_formats() -> Vec<String> {
    vec!["markdown".to_string()]
}

fn deserialize_firecrawl_formats<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let formats = Vec::<Value>::deserialize(deserializer)?;
    formats
        .into_iter()
        .map(|format| {
            let name = match format {
                Value::String(name) => name,
                Value::Object(mut options) => {
                    let name = options
                        .remove("type")
                        .and_then(|value| value.as_str().map(str::to_string))
                        .ok_or_else(|| {
                            D::Error::custom("format object must contain a string type")
                        })?;
                    if !options.is_empty() {
                        let fields = options.keys().cloned().collect::<Vec<_>>().join(", ");
                        return Err(D::Error::custom(format!(
                            "Firecrawl format '{name}' options are not supported: {fields}"
                        )));
                    }
                    name
                }
                _ => {
                    return Err(D::Error::custom(
                        "format must be a string or an object containing type",
                    ))
                }
            };
            if !matches!(
                name.as_str(),
                "markdown" | "html" | "rawHtml" | "links" | "screenshot"
            ) {
                return Err(D::Error::custom(format!(
                    "Firecrawl format '{name}' is not supported by BeeCrawl"
                )));
            }
            Ok(name)
        })
        .collect()
}

fn default_timeout_milliseconds() -> u64 {
    default_timeout_seconds() * 1_000
}

fn default_timeout_seconds() -> u64 {
    30
}

fn default_use_browser() -> String {
    "auto".to_string()
}

fn default_map_limit() -> usize {
    100
}

fn default_firecrawl_map_limit() -> usize {
    5_000
}

fn default_crawl_limit() -> usize {
    100
}

fn default_firecrawl_crawl_limit() -> usize {
    10_000
}

fn default_crawl_max_depth() -> usize {
    2
}

fn default_firecrawl_crawl_max_depth() -> usize {
    10_000
}

fn default_crawl_max_retries() -> usize {
    2
}

fn default_crawl_status_limit() -> usize {
    20
}

fn default_sitemap() -> String {
    "include".to_string()
}

fn default_ignore_query_parameters() -> bool {
    true
}

fn default_search_limit() -> usize {
    5
}

fn default_firecrawl_search_limit() -> usize {
    10
}

fn default_firecrawl_include_subdomains() -> bool {
    true
}

fn default_lang() -> String {
    "en".to_string()
}

fn default_country() -> String {
    "us".to_string()
}

fn default_llm_provider() -> String {
    "openai-compatible".to_string()
}
