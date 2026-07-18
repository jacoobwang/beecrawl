from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, Field


class Geolocation(BaseModel):
    country: str | None = Field(default=None, min_length=2, max_length=2)
    languages: list[str] = Field(default_factory=list)


class WaitAction(BaseModel):
    type: Literal["wait"]
    milliseconds: int = Field(default=1000, ge=0, le=60000)


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
    script: str
    metadata: dict[str, Any] = Field(default_factory=dict)


class ScrapeAction(BaseModel):
    type: Literal["scrape"]


class GetCookiesAction(BaseModel):
    type: Literal["getCookies"]


Action = WaitAction | ScreenshotAction | ExecuteJavascriptAction | ScrapeAction | GetCookiesAction


class BeeEngineScrapeRequest(BaseModel):
    url: str = Field(..., min_length=1)
    scrape_id: str | None = Field(default=None, alias="scrapeId")
    engine: Literal["playwright", "chrome-cdp"] = "playwright"
    instant_return: bool = Field(default=False, alias="instantReturn")
    headers: dict[str, str] = Field(default_factory=dict)
    actions: list[Action] = Field(default_factory=list)
    timeout: int = Field(default=300000, ge=1000, le=300000)
    wait: int = Field(default=0, ge=0, le=60000)
    mobile: bool = False
    block_media: bool = Field(default=True, alias="blockMedia")
    geolocation: Geolocation | None = None
    skip_tls_verification: bool = Field(default=False, alias="skipTlsVerification")


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
