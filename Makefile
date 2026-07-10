PYTHON ?= ./.venv/bin/python
UVICORN ?= ./.venv/bin/uvicorn
CARGO ?= cargo
HOST ?= 127.0.0.1
PORT ?= 8000
BEE_ENGINE_PORT ?= 8020
UV ?= uv

.PHONY: install api bee-engine test lint rust-test rust-lint python-test python-lint

install:
	$(UV) venv
	$(UV) pip install -e ".[dev]"

api:
	HOST=$(HOST) PORT=$(PORT) $(CARGO) run -p beecrawl-api

bee-engine:
	@test -x "$(UVICORN)" || (echo "Missing .venv. Run: make install" && exit 1)
	$(UVICORN) bee_engine.app:app --reload --app-dir apps/bee-engine --host $(HOST) --port $(BEE_ENGINE_PORT)

python-test:
	$(PYTHON) -m pytest -q

python-lint:
	$(PYTHON) -m ruff check .

rust-test:
	$(CARGO) test

rust-lint:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test: rust-test python-test

lint: rust-lint python-lint
