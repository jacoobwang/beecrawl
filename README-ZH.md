# 🐝 BeeCrawl

[English](README.md) | 简体中文

BeeCrawl 是一个开源的 Firecrawl 替代方案，面向希望自托管网页抓取、爬取、搜索和结构化提取能力的团队。

**BeeCrawl 提供类似 Firecrawl 的 API，包括干净的 Markdown 提取、浏览器渲染抓取、URL 发现、关键词搜索和确定性的 Schema 提取。** BeeCrawl 的定位是保持核心实现小巧、易于改造，同时为队列驱动的爬取、LLM 提取、特定来源 Provider、代理基础设施和托管部署保留清晰的扩展点。

API 服务使用 Rust 实现。浏览器渲染位于 Python Bee Engine 服务中，因为 Playwright 的 Python 运行时目前更适合作为本项目的浏览器自动化边界。

## API 预览

### `POST /scrape`

从 `workus-realtime-dataservice` 迁移而来的 Firecrawl 风格 Markdown 提取接口：

```json
{
  "url": "https://example.com",
  "formats": ["markdown", "html", "rawHtml", "links", "screenshot"],
  "timeout_seconds": 30,
  "wait_for_ms": 0,
  "use_browser": "auto"
}
```

返回 `request_id`、`final_url`、`markdown` 和 Provider 元数据。请求 `html` 可获取选定内容根节点的 HTML；请求 `rawHtml` 可获取完整的 HTTP 抓取或浏览器渲染 HTML；请求 `links` 可获取去重后的绝对 URL 列表；请求 `screenshot` 可获取 PNG Data URL。截图需要启用浏览器渲染。设置 `BEECRAWL_WEB_EXTRACT_API_KEY` 或 `WEB_EXTRACT_API_KEY` 后，可要求请求携带 `X-Web-Extract-Api-Key`、`X-Api-Key` 或 Bearer Token。

配置 Postgres 后，抓取缓存默认启用。请求路径为 `cache -> browser -> fetch`；缓存读取失败时会放行请求，返回格式会从缓存的 HTML 快照中重新生成。

### `POST /map`

```json
{
  "url": "https://example.com",
  "limit": 100,
  "include_subdomains": false
}
```

先从 sitemap、再从页面链接中发现同站点 URL。

### `POST /batch/scrape`

```json
{
  "urls": [
    "https://example.com",
    "https://example.com/docs"
  ],
  "use_browser": "auto",
  "maxRetries": 2
}
```

为多个相互独立的 URL 创建一个异步任务。入队前会删除重复 URL。通过 `GET /batch/scrape/{id}?offset=0&limit=20` 查询与 crawl 相同分页结构的结果，也可以使用 `DELETE /batch/scrape/{id}` 取消任务。Batch scrape 不会从提交的页面继续发现或跟随链接。

### `POST /crawl`

```json
{
  "url": "https://example.com",
  "limit": 100,
  "maxDepth": 2,
  "useBrowser": "auto"
}
```

启动一个异步的同站点爬取任务。通过 `GET /crawl/{id}?offset=0&limit=20` 查询进度和已收集结果，也可以使用 `DELETE /crawl/{id}` 请求取消。`maxRetries` 控制首次抓取失败后的重试次数，默认值为 `2`。任务和结果存储在 Postgres 中，由独立 Worker 进程消费。

### `POST /search`

```json
{
  "query": "thermal interface material suppliers",
  "limit": 5,
  "scrapeOptions": {
    "formats": ["markdown"],
    "use_browser": "auto"
  }
}
```

按关键词搜索网页，并返回结果 URL、标题和描述。当 `scrapeOptions.formats` 非空时，BeeCrawl 会使用已有的抓取服务抓取每个结果 URL，并将 Markdown 合并到搜索结果中。

设置 `BEECRAWL_SEARXNG_ENDPOINT` 可使用自托管 SearXNG。未配置 SearXNG 时，BeeCrawl 会回退到 DuckDuckGo HTML 搜索。

### `POST /extract`

```json
{
  "url": "https://example.com",
  "schema": {
    "company": "Company name",
    "email": "Contact email"
  },
  "use_browser": "auto"
}
```

返回结构化 JSON 对象。默认使用确定性的页面解析。配置兼容 OpenAI 的 LLM Provider 后，可启用模型驱动的提取：

```bash
BEECRAWL_LLM_PROVIDER=openai-compatible
BEECRAWL_LLM_API_KEY=...
BEECRAWL_LLM_BASE_URL=https://api.openai.com/v1
BEECRAWL_LLM_MODEL=gpt-4o-mini
```

也可以通过 `provider` 或 `llm` 为单次请求覆盖 Provider：

```json
{
  "url": "https://example.com",
  "schema": {
    "company": "Company name"
  },
  "provider": {
    "provider": "openai-compatible",
    "base_url": "https://dashscope.aliyuncs.com/compatible-mode/v1",
    "model": "qwen-plus"
  }
}
```

### Firecrawl v2 兼容性

对于使用固定版本 `firecrawl-py==4.32.1` 契约的应用，API 也提供 Firecrawl v2 兼容路由：

```text
POST   /v2/scrape
POST   /v2/parse
POST   /v2/parse/base64
POST   /v2/map
POST   /v2/crawl
GET    /v2/crawl/active
GET    /v2/crawl/ongoing
GET    /v2/crawl/{id}
DELETE /v2/crawl/{id}
GET    /v2/crawl/{id}/errors
POST   /v2/batch/scrape
GET    /v2/batch/scrape/{id}
DELETE /v2/batch/scrape/{id}
GET    /v2/batch/scrape/{id}/errors
POST   /v2/extract
POST   /v2/search
```

将 Firecrawl SDK 的 `api_url` 设置为 BeeCrawl 的基础 URL。这些路由接受 Firecrawl 的 camelCase 请求字段，并返回包含 `success` 的响应封装。对于不支持的字段、格式选项和会改变行为的选项值，接口会返回 JSON `400`，而不是静默忽略。`firecrawl-py` 4.32.1 发出的默认抓取选项均可使用，包括可用的 `skipTlsVerification` 支持。可运行 `make firecrawl-contract`，通过官方 Python SDK 在本地 API 上验证适配器。

v2 extract 适配器支持多个 URL 和 JSON Schema 对象。搜索支持 Web 结果及可选的结果抓取；在对应 Provider 加入前，请求的新闻和图片结果组会返回空数组。Batch scrape、错误列表、活跃 crawl 发现和分页任务状态都属于兼容范围。Usage-account 接口尚未实现。

`POST /v2/parse` 通过 `multipart/form-data` 接收本地 PDF：必须提供 `file` 字段，可选提供 JSON 格式的 `options` 字段。接口返回 Markdown，以及 `metadata.numPages`、`metadata.totalPages` 和 `metadata.sourceFile`。当前解析器支持 `fast` 或 `auto` 模式下的文本型 PDF；OCR 和非 PDF 文档格式会被明确拒绝。

对于只使用 JSON 的调用方，`POST /v2/parse/base64` 接收 `base64`（或 `data`）、`filename` 和可选的 `options`。`base64` 可以是裸 Base64，也可以是 `data:application/pdf;base64,...` 格式；解码后的 PDF 大小仍限制为 50 MB。

## 抓取质量评估

确定性测试和实时抓取质量评估是分开的。在本地启动 API 和 Bee Engine 后，执行：

```bash
make scrape-eval
```

评估套件覆盖静态页面、文档密集型页面和 JavaScript 渲染页面，将可观测输出与仓库中的质量门槛进行比较，并生成 JSON 和 Markdown 报告。案例编写方式和 `#scrape-quality-eval` Pull Request 工作流请参阅[抓取质量评估文档](docs/scrape-evals.md)。

如需与其他 Provider 做可复现对比，请运行独立的多轮 benchmark。它会统计成功率、内容质量、p50/p95/p99 延迟、错误数和每分钟成功处理页数：

```bash
make scrape-benchmark
```

该 benchmark 支持同时比较 BeeCrawl、Firecrawl 和 Teracrawl。Provider 配置、fresh-cache/warm-cache 两条测试线以及原始报告输出方式，请参阅[抓取质量评估文档](docs/scrape-evals.md)。

## 快速开始

启动 Rust API：

```bash
make api
```

如需运行分布式爬取，请先启动 Postgres，配置 `BEECRAWL_DATABASE_URL`，执行迁移，然后分别启动 API 和 Worker。BeeCrawl 使用 `sqlx-cli` 创建和执行迁移。

```bash
make db-up
export BEECRAWL_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:55432/beecrawl
cargo install sqlx-cli --no-default-features --features postgres,rustls
make migrate-up
make api
```

在另一个终端执行：

```bash
make worker
```

爬取任务默认保留七天。Worker 每小时执行一次清理；抓取缓存默认复用四小时并保留七天。也可以使用 `make crawl-cleanup` 配置定时清理任务。

`use_browser: "auto"` 的浏览器渲染由 Python Bee Engine 服务提供：

```bash
make install
make playwright-install
make bee-engine
```

浏览器渲染运行在 Bee Engine 中。服务会复用 Chromium 实例，并为每个请求创建隔离的 Context。设置 `BEE_ENGINE_MAX_PAGES` 可控制并发渲染页面数量，默认值为 `4`。

Bee Engine 默认在 `8020` 端口提供 Fire Engine 风格接口：

```text
POST   /scrape
GET    /scrape/{job_id}
DELETE /scrape/{job_id}
```

### Python SDK

HTTP-only Python SDK 位于 `apps/sdk/python`：

```bash
uv pip install -e apps/sdk/python
```

它为 `/scrape`、`/map`、`/search`、`/extract`、`/crawl` 和 `/batch/scrape` 提供同步和异步客户端。SDK 不会在本地运行浏览器；浏览器渲染和 Worker 都运行在 BeeCrawl 服务端。

### Node.js SDK

Node.js SDK 位于 `apps/sdk/node`：

```bash
npm install beecrawl-sdk
pnpm --filter beecrawl-sdk build
```

它使用 Node 18+ 原生 `fetch`，为 `/scrape`、`/map`、`/search`、`/extract`、`/crawl` 和 `/batch/scrape` 提供 TypeScript 客户端。

```js
import { BeeCrawlClient } from "beecrawl-sdk";

const client = new BeeCrawlClient({
  apiKey: "your-key",
  baseUrl: "https://api.beecrawl.dev",
});

const page = await client.scrape("https://example.com", {
  formats: ["markdown", "links"],
});
```

### BeeCrawl CLI

TypeScript CLI 位于 `apps/cli`，要求 Node.js 18 或更高版本。CLI 通过 Node.js SDK 使用 v2 API：

```bash
pnpm install
pnpm cli:build
node apps/cli/dist/main.js --help
```

首次调用 API 前，请通过 Dashboard 完成认证。CLI 会保存一个命名的本地 Profile，其中包含授权后的 API URL 和 Key：

```bash
node apps/cli/dist/main.js login
node apps/cli/dist/main.js profile current
```

在自动化或本地开发场景中，也可以通过环境变量提供 API Key，而不保存 Profile：

```bash
BEECRAWL_API_KEY=your-key \
BEECRAWL_BASE_URL=http://127.0.0.1:8000 \
node apps/cli/dist/main.js scrape https://example.com
```

可用的数据命令包括 `search`、`scrape`、`map`、`extract`、`crawl` 和 `agent`：

```bash
node apps/cli/dist/main.js search "web scraping" --limit 5
node apps/cli/dist/main.js scrape https://example.com
node apps/cli/dist/main.js map https://example.com --json
node apps/cli/dist/main.js extract https://example.com \
  --schema '{"title":"Page title"}' --json
node apps/cli/dist/main.js crawl https://example.com --no-wait --json
node apps/cli/dist/main.js agent "Summarize the main topics on this site" --json
```

`scrape` 默认输出 Markdown；其他数据命令默认输出 JSON。使用 `--json` 或 `--format json` 获取机器可读结果，使用 `--options-file` 传递嵌套 API 选项。Crawl 和 Agent 命令默认等待任务进入终态；如需分离工作流，可使用 `start`、`status`、`cancel` 或 `--no-wait`。`node apps/cli/dist/main.js init --agent codex` 可以安装内置 Agent Skill，`profile list|use|remove` 用于管理本地凭据 Profile。

### Rust SDK

异步 Rust SDK 位于 `apps/sdk/rust`，也可以作为已发布的 `beecrawl-sdk` Cargo 依赖添加：

```toml
[dependencies]
beecrawl-sdk = "0.1"
```

三个 SDK 会通过 `sdk-v<version>` Tag 一起发布。仓库会将 Python 包发布到 PyPI，将 npm 包发布到 npmjs.com，并将 Rust Crate 发布到 crates.io。发布工作流会验证三个包的版本与 Tag 一致，并要求仓库配置 `PYPI_API_TOKEN`、`NPM_TOKEN` 和 `CARGO_REGISTRY_TOKEN` Secrets。

然后调用：

```bash
curl -X POST http://127.0.0.1:8000/scrape \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com"}'
```

## 仓库结构

```text
apps/api         Rust API 包
apps/bee-engine  浏览器渲染服务
apps/sdk/node    Node.js SDK 包
apps/sdk/python  Python SDK 包
apps/sdk/rust    Rust SDK Crate
apps/cli         Node.js CLI 包
```

## Roadmap

### CLI 和 SDK 生态

- 将 `beecrawl-cli` 作为有版本管理的 npm 包发布，并增加发布工作流、Dashboard 授权，以及 Claude Code、Codex 和 OpenCode 的入门示例。
- 扩展 CLI，覆盖剩余的公开 v2 工作流：批量抓取、文档解析、浏览器 Session 与交互，以及 Monitor。
- 让 Python、Node.js 和 Rust SDK 与完整公开 API 保持一致，并继续以匹配的版本一起发布。

### 抓取质量和运维

- 增加代码密集型页面、HTML 异常页面、PDF 和 JavaScript 密集型页面评估案例，并为 Markdown 和元数据增加回归门槛。
- 增加转换和引擎指标，例如输入/输出大小、延迟、空输出比例、回退原因和 Provider 选择。
- 改进 Bee Engine 生命周期管理，为多实例部署提供明确的健康检查、关闭流程和容量控制。

### 自托管平台

- 增加可插拔的 Markdown 转换 Provider，以及可按域名配置的内容清理策略。
- 改善 Docker Compose 和 Helm 的部署体验，包括升级、Secret 配置，以及 Worker/浏览器的独立扩缩容。
- 在能为用户提供价值的范围内继续完善 Firecrawl 兼容性，同时明确不支持的托管 Usage-account 行为。

## 许可证

MIT
