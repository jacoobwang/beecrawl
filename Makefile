CARGO ?= cargo
HOST ?= 127.0.0.1
PORT ?= 8000
BEE_ENGINE_PORT ?= 8020
UV ?= uv

.PHONY: install db-up db-down api worker crawl-cleanup migration-new migrate-up bee-engine playwright-install firecrawl-contract test lint rust-test rust-lint python-test python-lint

install:
	$(UV) sync --extra dev --extra browser

db-up:
	docker compose up -d postgres

db-down:
	docker compose down

api:
	HOST=$(HOST) PORT=$(PORT) $(CARGO) run -p beecrawl-api

worker:
	$(CARGO) run -p beecrawl-api --bin worker

crawl-cleanup:
	$(CARGO) run -p beecrawl-api --bin crawl_cleanup

migration-new:
	./scripts/sqlx.sh migrate add --source apps/api/migrations "$(name)"

migrate-up:
	./scripts/sqlx.sh migrate run --source apps/api/migrations

bee-engine:
	$(UV) run --extra browser uvicorn bee_engine.app:app --reload --app-dir apps/bee-engine --host $(HOST) --port $(BEE_ENGINE_PORT)

playwright-install:
	$(UV) run --extra browser playwright install chromium

firecrawl-contract:
	$(UV) run --with firecrawl-py python scripts/firecrawl_v2_contract.py --api-url http://$(HOST):$(PORT)

python-test:
	$(UV) run --extra dev pytest -q

python-lint:
	$(UV) run --extra dev ruff check .

rust-test:
	$(CARGO) test

rust-lint:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test: rust-test python-test

lint: rust-lint python-lint
