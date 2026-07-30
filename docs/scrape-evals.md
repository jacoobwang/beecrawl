# Scrape quality evaluations

BeeCrawl keeps scrape quality evaluations separate from deterministic tests.
The normal test suite verifies code behavior with local fixtures. The evaluation
suite calls a running BeeCrawl API against stable public pages and checks
observable output quality.

## What is evaluated

The initial suite covers:

- a small static HTML page;
- a long standards document with headings and links;
- a JavaScript-rendered page that requires Bee Engine.

Each case can assert minimum Markdown length, required and forbidden text,
title presence, selected provider, and whether browser rendering was used.
The runner writes both a machine-readable JSON report and a Markdown summary.
The checked-in baseline gates the overall score, every individual case score,
and request failure count.

The external interface of the evaluation module is one command:

```bash
make scrape-eval
```

It expects BeeCrawl API and Bee Engine to be running on their default local
ports. To evaluate another deployment:

```bash
make scrape-eval SCRAPE_EVAL_API_URL=https://api.example.com
```

Reports are written to `artifacts/scrape-evals/` and are intentionally ignored
by Git.

## Editing the suite

- Cases: `evals/scrape-quality/cases.json`
- Quality gates: `evals/scrape-quality/baseline.json`
- Runner and scoring: `evals/scrape_eval.py`

Prefer stable, public, non-personal pages. Assertions should describe durable
content properties rather than exact full-page snapshots. Add a new assertion
type to the runner only when the existing output checks cannot express the
quality regression.

## Pull request workflow

The `Scrape Evals` GitHub Actions workflow is intentionally separate from
normal CI because it installs Chromium and accesses the public internet.

To run it for a pull request, add this marker to the PR title or body before
opening it:

```text
#scrape-quality-eval
```

Alternatively, a repository collaborator with write access can add the same
marker in a PR comment. The workflow can also be started manually with a Git
ref through `workflow_dispatch`.

Only actors with `write`, `maintain`, or `admin` permission can dispatch an
evaluation. The evaluation job receives a read-only repository token. Its JSON
and Markdown reports are uploaded as an Actions artifact, included in the job
summary, and the Markdown result is added to or updated on the pull request.

The workflow runs the code from the selected PR commit, but the dispatch and
commenting jobs remain separate from that code. This keeps write permission
out of the process that executes the evaluated checkout.
