from __future__ import annotations

import asyncio
import base64
import os
from typing import Any

from bee_engine.models import (
    ActionContent,
    ActionResult,
    BeeEngineScrapeRequest,
    BeeEngineScrapeResponse,
)


DEFAULT_USER_AGENT = "BeeEngine/0.1"
BLOCKED_RESOURCE_TYPES = {"image", "media", "font"}
BLOCKED_HOST_PARTS = (
    "googletagmanager.com",
    "google-analytics.com",
    "doubleclick.net",
    "facebook.net",
    "hotjar.com",
)
STEALTH_INIT_SCRIPT = """
Object.defineProperty(navigator, 'webdriver', {get: () => undefined});
Object.defineProperty(navigator, 'languages', {get: () => ['en-US', 'en']});
Object.defineProperty(navigator, 'plugins', {get: () => [1, 2, 3, 4, 5]});
window.chrome = window.chrome || {runtime: {}};
"""


class BrowserPool:
    def __init__(self, *, max_pages: int | None = None, playwright_factory=None) -> None:
        self._max_pages = max_pages or int(os.getenv("BEE_ENGINE_MAX_PAGES", "4"))
        self._page_semaphore = asyncio.Semaphore(self._max_pages)
        self._playwright_factory = playwright_factory
        self._playwright = None
        self._browser = None
        self._launch_lock = asyncio.Lock()

    def health(self) -> dict[str, Any]:
        browser_connected = bool(self._browser and self._browser.is_connected())
        available = self._page_semaphore._value
        return {
            "maxPages": self._max_pages,
            "availablePages": available,
            "activePages": self._max_pages - available,
            "browserConnected": browser_connected,
        }

    async def render(self, request: BeeEngineScrapeRequest) -> BeeEngineScrapeResponse:
        async with self._page_semaphore:
            browser = await self._ensure_browser()
            context = await browser.new_context(**_context_options(request))
            if _uses_stealth(request):
                await context.add_init_script(STEALTH_INIT_SCRIPT)
            if request.block_media:
                await context.route("**/*", _route_handler)

            page = await context.new_page()
            response = None
            try:
                response = await page.goto(
                    request.url,
                    wait_until="domcontentloaded",
                    timeout=request.timeout,
                )
                try:
                    await page.wait_for_load_state("networkidle", timeout=min(5000, request.timeout))
                except Exception:
                    pass
                if request.wait:
                    await page.wait_for_timeout(request.wait)

                screenshots: list[str] = []
                action_results: list[ActionResult] = []
                action_content: list[ActionContent] = []

                for idx, action in enumerate(request.actions):
                    if action.type == "wait":
                        if action.selector:
                            await page.wait_for_selector(action.selector, timeout=request.timeout)
                        else:
                            await page.wait_for_timeout(action.milliseconds)
                        action_results.append(ActionResult(idx=idx, type="wait", result={"ok": True}))
                    elif action.type == "screenshot":
                        if action.viewport:
                            await page.set_viewport_size(action.viewport.model_dump())
                        screenshot_options: dict[str, Any] = {"full_page": action.full_page}
                        media_type = "image/png"
                        if action.quality is not None:
                            screenshot_options.update(type="jpeg", quality=action.quality)
                            media_type = "image/jpeg"
                        screenshot = await page.screenshot(**screenshot_options)
                        encoded = base64.b64encode(screenshot).decode("ascii")
                        data_url = f"data:{media_type};base64,{encoded}"
                        screenshots.append(data_url)
                        action_results.append(
                            ActionResult(idx=idx, type="screenshot", result={"data": data_url})
                        )
                    elif action.type == "executeJavascript":
                        value = await page.evaluate(action.script)
                        action_results.append(
                            ActionResult(
                                idx=idx,
                                type="executeJavascript",
                                result={"return": _serialize_javascript_result(value)},
                            )
                        )
                    elif action.type == "scrape":
                        action_content.append(ActionContent(url=page.url, html=await page.content()))
                        action_results.append(
                            ActionResult(
                                idx=idx,
                                type="scrape",
                                result={"url": page.url, "html": await page.content()},
                            )
                        )
                    elif action.type == "getCookies":
                        cookies = await context.cookies()
                        action_results.append(
                            ActionResult(idx=idx, type="getCookies", result={"cookies": cookies})
                        )
                    elif action.type == "click":
                        locator = page.locator(action.selector)
                        if action.all:
                            count = await locator.count()
                            for match in await locator.all():
                                await match.click()
                        else:
                            await locator.first.click()
                            count = 1
                        action_results.append(
                            ActionResult(idx=idx, type="click", result={"clicked": count})
                        )
                    elif action.type == "write":
                        await page.keyboard.type(action.text)
                        action_results.append(
                            ActionResult(idx=idx, type="write", result={"written": len(action.text)})
                        )
                    elif action.type == "press":
                        await page.keyboard.press(action.key)
                        action_results.append(
                            ActionResult(idx=idx, type="press", result={"key": action.key})
                        )
                    elif action.type == "scroll":
                        distance = -720 if action.direction == "up" else 720
                        if action.selector:
                            await page.locator(action.selector).evaluate(
                                "(element, distance) => element.scrollBy(0, distance)", distance
                            )
                        else:
                            await page.mouse.wheel(0, distance)
                        action_results.append(
                            ActionResult(
                                idx=idx,
                                type="scroll",
                                result={"direction": action.direction, "selector": action.selector},
                            )
                        )
                    elif action.type == "pdf":
                        document = await page.pdf(
                            landscape=action.landscape,
                            print_background=action.print_background,
                            format=action.format,
                        )
                        data_url = "data:application/pdf;base64," + base64.b64encode(
                            document
                        ).decode("ascii")
                        action_results.append(
                            ActionResult(idx=idx, type="pdf", result={"data": data_url})
                        )

                headers = dict(response.headers) if response else {}
                return BeeEngineScrapeResponse(
                    timeTaken=0,
                    content=await page.content(),
                    url=page.url,
                    pageStatusCode=response.status if response else 0,
                    pageError=None,
                    responseHeaders=headers,
                    screenshots=screenshots,
                    actionContent=action_content,
                    actionResults=action_results,
                    usedMobileProxy=_uses_stealth(request),
                    timezone=None,
                )
            except Exception as exc:
                return BeeEngineScrapeResponse(
                    timeTaken=0,
                    content="",
                    url=getattr(page, "url", request.url),
                    pageStatusCode=response.status if response else 0,
                    pageError=str(exc),
                    responseHeaders=dict(response.headers) if response else {},
                )
            finally:
                await context.close()

    async def close(self) -> None:
        if self._browser:
            await self._browser.close()
            self._browser = None
        if self._playwright:
            await self._playwright.stop()
            self._playwright = None

    async def _ensure_browser(self):
        async with self._launch_lock:
            if self._browser and self._browser.is_connected():
                return self._browser

            if self._playwright_factory:
                self._playwright = self._playwright_factory()
            else:
                try:
                    from playwright.async_api import async_playwright
                except ImportError as exc:
                    raise RuntimeError(
                        "Playwright is not installed. Run: uv pip install -e '.[browser]'"
                    ) from exc
                self._playwright = await async_playwright().start()

            self._browser = await self._playwright.chromium.launch(
                headless=True,
                args=["--disable-dev-shm-usage", "--no-sandbox"],
            )
            return self._browser


def _context_options(request: BeeEngineScrapeRequest) -> dict[str, Any]:
    locale = None
    if request.geolocation and request.geolocation.languages:
        locale = request.geolocation.languages[0]

    options: dict[str, Any] = {
        "ignore_https_errors": request.skip_tls_verification,
        "extra_http_headers": request.headers,
        "user_agent": request.headers.get("User-Agent", DEFAULT_USER_AGENT),
        "viewport": {"width": 390, "height": 844} if request.mobile else {"width": 1366, "height": 768},
        "is_mobile": request.mobile,
        "has_touch": request.mobile,
    }
    if locale:
        options["locale"] = locale
    if request.proxy:
        options["proxy"] = request.proxy.model_dump(exclude={"mode"}, exclude_none=True)
    return options


def _uses_stealth(request: BeeEngineScrapeRequest) -> bool:
    return bool(request.proxy and request.proxy.mode in {"stealth", "enhanced"})


async def _route_handler(route) -> None:
    request = route.request
    host = ""
    try:
        host = request.url.split("/")[2]
    except IndexError:
        pass
    if request.resource_type in BLOCKED_RESOURCE_TYPES or any(x in host for x in BLOCKED_HOST_PARTS):
        await route.abort()
    else:
        await route.continue_()


def _serialize_javascript_result(value: Any) -> dict[str, Any]:
    value_type = "null" if value is None else type(value).__name__
    try:
        import json

        json.dumps(value)
        return {"type": value_type, "value": value}
    except TypeError:
        return {"type": value_type, "value": str(value)}
