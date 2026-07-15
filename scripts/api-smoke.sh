#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PID=""
TMP="$(mktemp -d "${TMPDIR:-/tmp}/g7mb-api.XXXXXX")"

cleanup() {
    if [[ -n "$PID" ]]; then
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

cd "$ROOT"
cargo build --quiet --locked --package g7mb-api
export G7MB__DATABASE__URL="sqlite://$TMP/g7mb.db"
export G7MB__WORKER__SANDBOX_BINARY="$ROOT/tests/fixtures/fake-capability-sandbox.sh"
target/debug/g7mb-api --config config/g7mb.example.toml >"${TMPDIR:-/tmp}/g7mb-api-smoke.log" 2>&1 &
PID="$!"

for _attempt in $(seq 1 50); do
    if curl --silent --fail http://127.0.0.1:8088/health/live >/dev/null; then
        break
    fi
    sleep 0.1
done

curl --silent --fail http://127.0.0.1:8088/health/live | grep -q '"status":"live"'
curl --silent --fail http://127.0.0.1:8088/health/ready | grep -q '"status":"ready"'
[[ "$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:8088/v1/capabilities)" == "400" ]]
curl --silent --fail --dump-header - http://127.0.0.1:8088/health/live \
    | tr -d '\r' \
    | grep -qi '^x-content-type-options: nosniff$'

signed_request() {
    local method="$1"
    local path="$2"
    local body="$3"
    local nonce="$4"
    local timestamp body_sha canonical signature
    timestamp="$(date +%s)"
    body_sha="$(printf '%s' "$body" | openssl dgst -sha256 | awk '{print $2}')"
    canonical="$(printf 'G7MB-HMAC-SHA256\ng7-primary\n%s\n%s\n%s\n%s\n%s' \
        "$timestamp" "$nonce" "$method" "$path" "$body_sha")"
    signature="$(printf '%s' "$canonical" \
        | openssl dgst -sha256 -mac HMAC -macopt 'key:replace-with-at-least-32-characters' -binary \
        | openssl base64 -A \
        | tr '+/' '-_' \
        | tr -d '=')"
    curl --silent --fail \
        --request "$method" \
        --header 'content-type: application/json' \
        --header 'x-g7mb-key-id: g7-primary' \
        --header "x-g7mb-timestamp: $timestamp" \
        --header "x-g7mb-nonce: $nonce" \
        --header "x-g7mb-content-sha256: $body_sha" \
        --header "x-g7mb-signature: $signature" \
        --data-binary "$body" \
        "http://127.0.0.1:8088$path"
}

capabilities="$(signed_request GET /v1/capabilities '' 7123456789abcdef0123456789abcdef)"
[[ "$capabilities" == *'"image_inputs":["avif","gif","heif","jpeg","png","webp"]'* ]]
[[ "$capabilities" == *'"mp4_thumbnail":true'* ]]
[[ "$capabilities" == *'"mp4_h264_fallback":true'* ]]

issued_at="$(date +%s)"
policy_body="{\"schema_version\":1,\"revision\":1,\"issued_at\":$issued_at,\"watermark\":null}"
published_policy="$(signed_request PUT /v1/site-policy "$policy_body" 8123456789abcdef0123456789abcdef)"
[[ "$published_policy" == *'"revision":1'* ]]
[[ "$published_policy" == *'"settings_sha256":"'* ]]
active_policy="$(signed_request GET /v1/site-policy '' 9123456789abcdef0123456789abcdef)"
[[ "$active_policy" == *'"revision":1'* ]]
