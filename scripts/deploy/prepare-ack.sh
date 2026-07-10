#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-beecrawl}"
DATABASE_SECRET="beecrawl-database-secret"
API_SECRET="beecrawl-api-secret"
LLM_SECRET="beecrawl-llm-secret"
IMAGE_PULL_SECRET="${IMAGE_PULL_SECRET:-regcred-ghcr}"

: "${BEECRAWL_DATABASE_URL:?Set BEECRAWL_DATABASE_URL for the dedicated beecrawl database}"

printf '%s' "$BEECRAWL_DATABASE_URL" | python3 -c '
import sys
from urllib.parse import urlparse
database = urlparse(sys.stdin.read()).path.lstrip("/")
if database != "beecrawl":
    raise SystemExit("BEECRAWL_DATABASE_URL must use the dedicated beecrawl database")
'

kubectl cluster-info > /dev/null
kubectl get namespace "$NAMESPACE" > /dev/null 2>&1 || kubectl create namespace "$NAMESPACE"

kubectl create secret generic "$DATABASE_SECRET" \
  --namespace "$NAMESPACE" \
  --from-literal=BEECRAWL_DATABASE_URL="$BEECRAWL_DATABASE_URL" \
  --dry-run=client -o yaml | kubectl apply -f -

if [[ -n "${BEECRAWL_WEB_EXTRACT_API_KEY:-}" ]]; then
  kubectl create secret generic "$API_SECRET" \
    --namespace "$NAMESPACE" \
    --from-literal=BEECRAWL_WEB_EXTRACT_API_KEY="$BEECRAWL_WEB_EXTRACT_API_KEY" \
    --dry-run=client -o yaml | kubectl apply -f -
fi

if [[ -n "${BEECRAWL_LLM_API_KEY:-}" ]]; then
  LLM_ARGS=(
    --from-literal=BEECRAWL_LLM_API_KEY="$BEECRAWL_LLM_API_KEY"
  )
  for name in BEECRAWL_LLM_PROVIDER BEECRAWL_LLM_BASE_URL BEECRAWL_LLM_MODEL; do
    if [[ -n "${!name:-}" ]]; then
      LLM_ARGS+=(--from-literal="$name=${!name}")
    fi
  done
  kubectl create secret generic "$LLM_SECRET" \
    --namespace "$NAMESPACE" \
    "${LLM_ARGS[@]}" \
    --dry-run=client -o yaml | kubectl apply -f -
fi

if [[ -n "${GHCR_TOKEN:-}" ]]; then
  : "${GHCR_USERNAME:?Set GHCR_USERNAME when GHCR_TOKEN is set}"
  kubectl create secret docker-registry "$IMAGE_PULL_SECRET" \
    --namespace "$NAMESPACE" \
    --docker-server=ghcr.io \
    --docker-username="$GHCR_USERNAME" \
    --docker-password="$GHCR_TOKEN" \
    --dry-run=client -o yaml | kubectl apply -f -
fi

echo "ACK namespace and BeeCrawl Secrets are ready in namespace $NAMESPACE"
