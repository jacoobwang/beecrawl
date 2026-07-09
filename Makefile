PYTHON ?= ./.venv/bin/python
UVICORN ?= ./.venv/bin/uvicorn
HOST ?= 127.0.0.1
PORT ?= 8000
UV ?= uv

.PHONY: install api test lint

install:
	$(UV) venv
	$(UV) pip install -e ".[dev]"

api:
	@test -x "$(UVICORN)" || (echo "Missing .venv. Run: make install" && exit 1)
	$(UVICORN) beecrawl.app:app --reload --host $(HOST) --port $(PORT)

test:
	$(PYTHON) -m pytest -q

lint:
	$(PYTHON) -m ruff check .
