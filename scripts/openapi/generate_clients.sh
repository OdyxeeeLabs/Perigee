#!/usr/bin/env bash
# Generates API clients (TypeScript, Python, Go) and a Postman collection from the
# running API's OpenAPI document.
#
#   scripts/openapi/generate_clients.sh [base-url] [out-dir]
#
# Defaults: http://localhost:3000, ./clients. Needs `curl`, `cargo` and Docker.
set -euo pipefail

BASE_URL="${1:-http://localhost:3000}"
OUT_DIR="${2:-clients}"
GENERATOR_IMAGE="openapitools/openapi-generator-cli:v7.5.0"

mkdir -p "$OUT_DIR"
curl --fail --silent --show-error "$BASE_URL/api-docs/openapi.json" -o "$OUT_DIR/openapi.raw.json"

# 3.0 -> 3.1, RFC 7807 errors, /v1 server; fail early if the contract is broken.
cargo run --quiet -p Perigee-core --bin openapi-tools -- prepare "$OUT_DIR/openapi.raw.json" "$OUT_DIR/openapi.json" --version v1
cargo run --quiet -p Perigee-core --bin openapi-tools -- check "$OUT_DIR/openapi.json"
cargo run --quiet -p Perigee-core --bin openapi-tools -- postman "$OUT_DIR/openapi.json" "$OUT_DIR/perigee.postman_collection.json"

for target in "typescript-fetch:typescript" "python:python" "go:go"; do
  generator="${target%%:*}"
  dir="${target##*:}"
  docker run --rm -v "$PWD/$OUT_DIR:/work" "$GENERATOR_IMAGE" generate \
    -i /work/openapi.json -g "$generator" -o "/work/$dir" \
    --additional-properties=packageName=perigee_client
done

echo "Wrote clients and Postman collection to $OUT_DIR/"
