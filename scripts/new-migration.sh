#!/usr/bin/env bash
set -euo pipefail

name="${1:-}"
if [[ ! "$name" =~ ^[a-z0-9]+(_[a-z0-9]+)*$ ]]; then
  echo "usage: make migration-new name=lowercase_snake_case" >&2
  exit 1
fi

version="$(date -u +%Y%m%d%H%M%S)"
path="apps/api/migrations/${version}_${name}.sql"
if [[ -e "$path" ]]; then
  echo "migration already exists: $path" >&2
  exit 1
fi

touch "$path"
echo "created $path"
