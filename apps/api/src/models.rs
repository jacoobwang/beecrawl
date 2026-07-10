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
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WebExtractLocation {
    pub country: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WebExtractScrapeResponse {
    pub request_id: String,
    pub url: String,
    pub final_url: String,
    pub markdown: String,
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
    #[serde(default)]
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct CrawlStatusQuery {
    #[serde(default)]
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
}

fn default_formats() -> Vec<String> {
    vec!["markdown".to_string()]
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

fn default_crawl_limit() -> usize {
    100
}

fn default_crawl_max_depth() -> usize {
    2
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

fn default_lang() -> String {
    "en".to_string()
}

fn default_country() -> String {
    "us".to_string()
}

fn default_llm_provider() -> String {
    "openai-compatible".to_string()
}
