CARGO ?= cargo
HOST ?= 127.0.0.1
PORT ?= 8000
BEE_ENGINE_PORT ?= 8020
UV ?= uv
SCRAPE_EVAL_API_URL ?= http://$(HOST):$(PORT)
SCRAPE_BENCHMARK_PROVIDERS ?= --provider beecrawl=$(SCRAPE_EVAL_API_URL)

.PHONY: install db-up db-down api worker crawl-cleanup migration-new migrate-up bee-engine playwright-install firecrawl-contract scrape-eval scrape-benchmark test lint rust-test rust-lint python-test python-lint

install:
	$(UV) sync --extra dev --extra browser

db-up:
	docker compose up -d postgres

db-down:
	docker compose down

api:
	HOST=$(HOST) PORT=$(PORT) $(CARGO) run -p beecrawl-api --bin beecrawl-api

worker:
	$(CARGO) run -p beecrawl-api --bin worker

crawl-cleanup:
	$(CARGO) run -p beecrawl-api --bin crawl_cleanup

migration-new:
	./scripts/sqlx.sh migrate add --source apps/api/migrations "$(name)"

migrate-up:
	./scripts/sqlx.sh migrate run --source apps/api/migrations

bee-engine:
	$(UV) run --extra browser --extra fingerprint --extra documents uvicorn bee_engine.app:app --reload --app-dir apps/bee-engine --host $(HOST) --port $(BEE_ENGINE_PORT)

playwright-install:
	$(UV) run --extra browser playwright install chromium

firecrawl-contract:
	$(UV) run --with firecrawl-py==4.32.1 python scripts/firecrawl_v2_contract.py --api-url http://$(HOST):$(PORT)

scrape-eval:
	$(UV) run python -m evals.scrape_eval --api-url $(SCRAPE_EVAL_API_URL)

scrape-benchmark:
	$(UV) run python -m evals.scrape_benchmark $(SCRAPE_BENCHMARK_PROVIDERS)

python-test:
	$(UV) run --extra dev --extra documents pytest -q

python-lint:
	$(UV) run --extra dev ruff check .

rust-test:
	$(CARGO) test

rust-lint:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test: rust-test python-test

lint: rust-lint python-lint
