PYTHON ?= ./.venv/bin/python
UVICORN ?= ./.venv/bin/uvicorn
HOST ?= 127.0.0.1
PORT ?= 8000
BEE_ENGINE_PORT ?= 8020
UV ?= uv

.PHONY: install api bee-engine test lint

install:
	$(UV) venv
	$(UV) pip install -e ".[dev]"

api:
	@test -x "$(UVICORN)" || (echo "Missing .venv. Run: make install" && exit 1)
	$(UVICORN) beecrawl.app:app --reload --host $(HOST) --port $(PORT)

bee-engine:
	@test -x "$(UVICORN)" || (echo "Missing .venv. Run: make install" && exit 1)
	$(UVICORN) bee_engine.app:app --reload --app-dir apps/bee-engine --host $(HOST) --port $(BEE_ENGINE_PORT)

test:
	$(PYTHON) -m pytest -q

lint:
	$(PYTHON) -m ruff check .
