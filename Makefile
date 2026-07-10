CARGO ?= cargo
HOST ?= 127.0.0.1
PORT ?= 8000
BEE_ENGINE_PORT ?= 8020
UV ?= uv

.PHONY: install db-up db-down api worker migrate-up bee-engine playwright-install test lint rust-test rust-lint python-test python-lint

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

migrate-up:
	$(CARGO) run -p beecrawl-api --bin migrate

bee-engine:
	$(UV) run --extra browser uvicorn bee_engine.app:app --reload --app-dir apps/bee-engine --host $(HOST) --port $(BEE_ENGINE_PORT)

playwright-install:
	$(UV) run --extra browser playwright install chromium

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
