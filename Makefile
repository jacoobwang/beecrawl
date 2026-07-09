PYTHON ?= ./.venv/bin/python
UVICORN ?= ./.venv/bin/uvicorn
HOST ?= 127.0.0.1
PORT ?= 8000

.PHONY: install api test lint

install:
	python3 -m venv .venv
	$(PYTHON) -m pip install -e ".[dev]"

api:
	@test -x "$(UVICORN)" || (echo "Missing .venv. Run: make install" && exit 1)
	$(UVICORN) beecrawl.app:app --reload --host $(HOST) --port $(PORT)

test:
	$(PYTHON) -m pytest -q

lint:
	$(PYTHON) -m ruff check .
