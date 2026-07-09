from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, HttpUrl


class Link(BaseModel):
    text: str
    url: str


class ScrapeResponse(BaseModel):
    url: str
    title: str | None = None
    text: str
    links: list[Link] = Field(default_factory=list)
    metadata: dict[str, str] = Field(default_factory=dict)


class ExtractRequest(BaseModel):
    url: HttpUrl
    schema_: dict[str, str] = Field(alias="schema")


class ExtractResponse(BaseModel):
    url: str
    data: dict[str, str | None]
    scrape: ScrapeResponse


class WebExtractLocation(BaseModel):
    country: str | None = Field(default=None, min_length=2, max_length=2)
    languages: list[str] = Field(default_factory=list)


class WebExtractScrapeRequest(BaseModel):
    url: str = Field(..., min_length=1)
    formats: list[Literal["markdown"]] = Field(default_factory=lambda: ["markdown"])
    location: WebExtractLocation | None = None
    timeout_seconds: int = Field(default=30, ge=1, le=120)
    wait_for_ms: int = Field(default=0, ge=0, le=60000)
    use_browser: Literal["auto", "always", "never"] = "auto"


class SearchScrapeOptions(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    formats: list[Literal["markdown"]] = Field(default_factory=list)
    timeout_seconds: int = Field(default=30, ge=1, le=120)
    wait_for_ms: int = Field(default=0, ge=0, le=60000)
    use_browser: Literal["auto", "always", "never"] = "auto"


class SearchRequest(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    query: str = Field(..., min_length=1)
    limit: int = Field(default=5, ge=1, le=20)
    lang: str = Field(default="en", min_length=2, max_length=8)
    country: str = Field(default="us", min_length=2, max_length=8)
    scrape_options: SearchScrapeOptions | None = Field(default=None, alias="scrapeOptions")


class SearchResult(BaseModel):
    url: str
    title: str | None = None
    description: str | None = None
    markdown: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)
    scrape_error: str | None = Field(default=None, alias="scrapeError")


class SearchMetadata(BaseModel):
    provider: str
    count: int
    scraped_count: int = Field(default=0, alias="scrapedCount")
    elapsed_ms: int | None = Field(default=None, alias="elapsedMs")


class SearchResponse(BaseModel):
    request_id: str = Field(alias="requestId")
    query: str
    results: list[SearchResult]
    metadata: SearchMetadata


class WebExtractMetadata(BaseModel):
    title: str | None = None
    language: str | None = None
    status_code: int | None = None
    provider: str
    rendered: bool = False
    elapsed_ms: int | None = None


class WebExtractScrapeResponse(BaseModel):
    request_id: str
    url: str
    final_url: str
    markdown: str
    metadata: WebExtractMetadata


class WebExtractMapRequest(BaseModel):
    url: str = Field(..., min_length=1)
    search: str | None = None
    limit: int = Field(default=100, ge=1, le=1000)
    include_subdomains: bool = False
    sitemap: Literal["only", "include", "skip"] = "include"
    ignore_sitemap: bool = False
    ignore_query_parameters: bool = True


class WebExtractMapMetadata(BaseModel):
    provider: str
    count: int
    elapsed_ms: int | None = None


class WebExtractMapResponse(BaseModel):
    request_id: str
    url: str
    links: list[str]
    metadata: WebExtractMapMetadata


class ProviderPage(BaseModel):
    url: str
    final_url: str
    html: str
    status_code: int | None = None
    title: str | None = None
    language: str | None = None
    provider: str
    rendered: bool = False
    extra: dict[str, Any] = Field(default_factory=dict)
