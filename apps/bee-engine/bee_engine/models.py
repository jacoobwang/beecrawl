from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, Field, model_validator


class Geolocation(BaseModel):
    country: str | None = Field(default=None, min_length=2, max_length=2)
    languages: list[str] = Field(default_factory=list)


class ProxySettings(BaseModel):
    mode: Literal["basic", "stealth", "enhanced"] = "basic"
    server: str = Field(..., min_length=1)
    username: str | None = None
    password: str | None = None


class WaitAction(BaseModel):
    type: Literal["wait"]
    milliseconds: int | None = Field(default=None, ge=1, le=60000)
    selector: str | None = Field(default=None, min_length=1, max_length=4096)

    @model_validator(mode="after")
    def validate_wait_target(self):
        if self.milliseconds is None and self.selector is None:
            self.milliseconds = 1000
        if self.milliseconds is not None and self.selector is not None:
            raise ValueError("wait accepts milliseconds or selector, not both")
        return self


class ScreenshotViewport(BaseModel):
    width: int = Field(ge=1, le=7680)
    height: int = Field(ge=1, le=4320)


class ScreenshotAction(BaseModel):
    type: Literal["screenshot"]
    full_page: bool = Field(default=False, alias="fullPage")
    quality: int | None = Field(default=None, ge=1, le=100)
    viewport: ScreenshotViewport | None = None


class ExecuteJavascriptAction(BaseModel):
    type: Literal["executeJavascript"]
    script: str = Field(min_length=1, max_length=131072)
    metadata: dict[str, Any] = Field(default_factory=dict)


class ScrapeAction(BaseModel):
    type: Literal["scrape"]


class GetCookiesAction(BaseModel):
    type: Literal["getCookies"]


class ClickAction(BaseModel):
    type: Literal["click"]
    selector: str = Field(min_length=1, max_length=4096)
    all: bool = False


class WriteAction(BaseModel):
    type: Literal["write"]
    text: str = Field(max_length=131072)


class PressAction(BaseModel):
    type: Literal["press"]
    key: str = Field(min_length=1, max_length=128)


class ScrollAction(BaseModel):
    type: Literal["scroll"]
    direction: Literal["up", "down"] = "down"
    selector: str | None = Field(default=None, min_length=1, max_length=4096)


class PdfAction(BaseModel):
    type: Literal["pdf"]
    landscape: bool = False
    print_background: bool = Field(default=True, alias="printBackground")
    format: Literal["Letter", "Legal", "Tabloid", "A0", "A1", "A2", "A3", "A4", "A5"] = "A4"


Action = (
    WaitAction
    | ScreenshotAction
    | ExecuteJavascriptAction
    | ScrapeAction
    | GetCookiesAction
    | ClickAction
    | WriteAction
    | PressAction
    | ScrollAction
    | PdfAction
)


class BeeEngineScrapeRequest(BaseModel):
    url: str = Field(..., min_length=1)
    scrape_id: str | None = Field(default=None, alias="scrapeId")
    engine: Literal["playwright", "chrome-cdp"] = "playwright"
    instant_return: bool = Field(default=False, alias="instantReturn")
    headers: dict[str, str] = Field(default_factory=dict)
    actions: list[Action] = Field(default_factory=list, max_length=50)
    timeout: int = Field(default=300000, ge=1000, le=300000)
    wait: int = Field(default=0, ge=0, le=60000)
    mobile: bool = False
    block_media: bool = Field(default=True, alias="blockMedia")
    geolocation: Geolocation | None = None
    skip_tls_verification: bool = Field(default=False, alias="skipTlsVerification")
    proxy: ProxySettings | None = None

    @model_validator(mode="after")
    def validate_action_budget(self):
        payload_size = sum(
            len(getattr(action, "script", "")) + len(getattr(action, "text", ""))
            for action in self.actions
        )
        if payload_size > 262144:
            raise ValueError("action script and text payloads exceed 262144 characters")
        wait_ms = sum(getattr(action, "milliseconds", 0) or 0 for action in self.actions)
        if wait_ms + self.wait > self.timeout:
            raise ValueError("action waits exceed the request timeout")
        return self


class FingerprintFetchRequest(BaseModel):
    url: str = Field(..., min_length=1)
    method: Literal["GET"] = "GET"
    headers: dict[str, str] = Field(default_factory=dict)
    profile: str = "chrome_124"
    timeout_ms: int = Field(default=30000, alias="timeoutMs", ge=1000, le=300000)
    skip_tls_verification: bool = Field(default=False, alias="skipTlsVerification")
    proxy: ProxySettings | None = None


class FingerprintFetchResponse(BaseModel):
    status: int
    url: str
    headers: dict[str, str]
    body: str


class DocumentParseRequest(BaseModel):
    filename: str = Field(min_length=1, max_length=512)
    base64: str = Field(min_length=1)
    formats: list[Literal["markdown", "html", "rawHtml", "summary", "json"]] = Field(
        default_factory=lambda: ["markdown"]
    )
    mode: Literal["fast", "auto", "ocr"] = "auto"
    max_pages: int | None = Field(default=None, alias="maxPages", ge=1, le=10000)


class DocumentParseResponse(BaseModel):
    data: dict[str, Any]
    metadata: dict[str, Any]


class ProcessingResponse(BaseModel):
    job_id: str = Field(alias="jobId")
    processing: bool = True


class ActionResult(BaseModel):
    idx: int
    type: str
    result: dict[str, Any]


class ActionContent(BaseModel):
    url: str
    html: str


class BeeEngineScrapeResponse(BaseModel):
    job_id: str | None = Field(default=None, alias="jobId")
    time_taken: int = Field(alias="timeTaken")
    content: str
    url: str
    page_status_code: int = Field(alias="pageStatusCode")
    page_error: str | None = Field(default=None, alias="pageError")
    response_headers: dict[str, str] = Field(default_factory=dict, alias="responseHeaders")
    screenshots: list[str] = Field(default_factory=list)
    action_content: list[ActionContent] = Field(default_factory=list, alias="actionContent")
    action_results: list[ActionResult] = Field(default_factory=list, alias="actionResults")
    used_mobile_proxy: bool = Field(default=False, alias="usedMobileProxy")
    timezone: str | None = None


class FailedResponse(BaseModel):
    job_id: str = Field(alias="jobId")
    state: Literal["failed"] = "failed"
    processing: bool = False
    error: str


BeeEngineStatusResponse = BeeEngineScrapeResponse | ProcessingResponse | FailedResponse
