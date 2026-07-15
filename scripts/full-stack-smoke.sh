#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e"
CONTAINER="g7mb-full-stack-minio-$$"
ACCESS_KEY="g7mbtestaccess"
SECRET_KEY="g7mbtestsecret0123456789"
RAW_BUCKET="g7mb-full-stack-raw"
DERIVATIVE_BUCKET="g7mb-full-stack-media"
API_ADDR="${G7MB_FULL_STACK_API_ADDR:-127.0.0.1:18088}"
API_BASE="http://$API_ADDR"
HMAC_SECRET="replace-with-at-least-32-characters"
API_PID=""
TMP="$(mktemp -d "${TMPDIR:-/tmp}/g7mb-full-stack.XXXXXX")"
API_LOG="$TMP/api.log"

cleanup() {
    if [[ -n "$API_PID" ]]; then
        kill "$API_PID" 2>/dev/null || true
        wait "$API_PID" 2>/dev/null || true
    fi
    docker rm --force "$CONTAINER" >/dev/null 2>&1 || true
    rm -rf "$TMP"
}
trap cleanup EXIT

command -v curl >/dev/null
command -v docker >/dev/null
command -v ffmpeg >/dev/null
command -v jq >/dev/null
command -v openssl >/dev/null
command -v split >/dev/null
command -v vipsheader >/dev/null
docker info >/dev/null

file_size() {
    if stat -f%z "$1" >/dev/null 2>&1; then
        stat -f%z "$1"
    else
        stat -c%s "$1"
    fi
}

signed_request() {
    local method="$1"
    local path="$2"
    local body="${3:-}"
    local nonce timestamp body_sha canonical signature
    nonce="$(openssl rand -hex 16)"
    timestamp="$(date +%s)"
    body_sha="$(printf '%s' "$body" | openssl dgst -sha256 | awk '{print $2}')"
    canonical="$(printf 'G7MB-HMAC-SHA256\ng7-primary\n%s\n%s\n%s\n%s\n%s' \
        "$timestamp" "$nonce" "$method" "$path" "$body_sha")"
    signature="$(printf '%s' "$canonical" \
        | openssl dgst -sha256 -mac HMAC -macopt "key:$HMAC_SECRET" -binary \
        | openssl base64 -A \
        | tr '+/' '-_' \
        | tr -d '=')"
    curl --fail-with-body --silent --show-error \
        --request "$method" \
        --header 'accept: application/json' \
        --header 'content-type: application/json' \
        --header 'x-g7mb-key-id: g7-primary' \
        --header "x-g7mb-timestamp: $timestamp" \
        --header "x-g7mb-nonce: $nonce" \
        --header "x-g7mb-content-sha256: $body_sha" \
        --header "x-g7mb-signature: $signature" \
        --data-binary "$body" \
        "$API_BASE$path"
}

presigned_put() {
    local instruction="$1"
    local body="$2"
    local header_output="${3:-/dev/null}"
    local url
    local -a curl_args
    url="$(jq -er '.upload_url' <<<"$instruction")"
    curl_args=(
        --fail-with-body
        --silent
        --show-error
        --request PUT
        --dump-header "$header_output"
        --output /dev/null
    )
    while IFS=$'\t' read -r name value; do
        curl_args+=(--header "$name: $value")
    done < <(jq -r '.required_headers | to_entries[] | [.key, .value] | @tsv' <<<"$instruction")
    curl "${curl_args[@]}" --data-binary "@$body" "$url"
}

wait_for_api() {
    local ready=false
    for _ in $(seq 1 180); do
        if curl --fail --silent "$API_BASE/health/ready" >/dev/null; then
            ready=true
            break
        fi
        if ! kill -0 "$API_PID" 2>/dev/null; then
            break
        fi
        sleep 0.1
    done
    if [[ "$ready" != true ]]; then
        tail -n 160 "$API_LOG" >&2
        exit 1
    fi
}

cd "$ROOT"
export VIPS_CONCURRENCY=1

docker run --detach --rm \
    --name "$CONTAINER" \
    --env "MINIO_ROOT_USER=$ACCESS_KEY" \
    --env "MINIO_ROOT_PASSWORD=$SECRET_KEY" \
    --publish 127.0.0.1::9000 \
    "$IMAGE" server /data >/dev/null

port="$(docker port "$CONTAINER" 9000/tcp | sed -n '1s/.*://p')"
if [[ -z "$port" ]]; then
    echo "MinIO published port was not found" >&2
    exit 1
fi
endpoint="http://127.0.0.1:$port"

minio_ready=false
for _ in $(seq 1 120); do
    if curl --fail --silent "$endpoint/minio/health/live" >/dev/null; then
        minio_ready=true
        break
    fi
    sleep 0.25
done
if [[ "$minio_ready" != true ]]; then
    docker logs "$CONTAINER" >&2
    exit 1
fi

export G7MB_TEST_S3_ENDPOINT="$endpoint"
export G7MB_TEST_S3_ACCESS_KEY="$ACCESS_KEY"
export G7MB_TEST_S3_SECRET_KEY="$SECRET_KEY"
export G7MB_TEST_S3_RAW_BUCKET="$RAW_BUCKET"
export G7MB_TEST_S3_DERIVATIVE_BUCKET="$DERIVATIVE_BUCKET"
cargo test --quiet --locked --package g7mb-object-store-s3 \
    --test minio_conformance -- --ignored --nocapture

if base64 --decode <tests/fixtures/private-exif.jpg.b64 >"$TMP/private-exif.jpg" 2>/dev/null; then
    :
else
    base64 -D <tests/fixtures/private-exif.jpg.b64 >"$TMP/private-exif.jpg"
fi
ffmpeg -hide_banner -loglevel error -nostdin \
    -f lavfi -i "nullsrc=s=4000x3000,noise=alls=100:allf=t" \
    -frames:v 1 -c:v mjpeg -q:v 1 -threads 1 -y "$TMP/multipart.jpg"

single_size="$(file_size "$TMP/private-exif.jpg")"
multipart_size="$(file_size "$TMP/multipart.jpg")"
if (( single_size < 1 || single_size >= 5 * 1024 * 1024 )); then
    echo "single fixture is outside the single-PUT policy" >&2
    exit 1
fi
if (( multipart_size <= 5 * 1024 * 1024 )); then
    echo "multipart fixture did not cross the multipart threshold" >&2
    exit 1
fi

cargo build --quiet --locked --package g7mb-api --package g7mb-worker
cargo build --quiet --locked --package g7mb-sandbox --features native-vips

export G7MB__SERVER__BIND_ADDR="$API_ADDR"
export G7MB__DATABASE__URL="sqlite://$TMP/g7mb.db"
export G7MB__DATABASE__BACKUP_DIRECTORY="$TMP/backups"
export G7MB__STORAGE__ENDPOINT_URL="$endpoint"
export G7MB__STORAGE__REGION="us-east-1"
export G7MB__STORAGE__RAW_BUCKET="$RAW_BUCKET"
export G7MB__STORAGE__DERIVATIVE_BUCKET="$DERIVATIVE_BUCKET"
export G7MB__STORAGE__ACCESS_KEY_ID="$ACCESS_KEY"
export G7MB__STORAGE__SECRET_ACCESS_KEY="$SECRET_KEY"
export G7MB__STORAGE__FORCE_PATH_STYLE="true"
export G7MB__UPLOAD__MULTIPART_THRESHOLD_BYTES="$((5 * 1024 * 1024))"
export G7MB__UPLOAD__MULTIPART_PART_SIZE_BYTES="$((5 * 1024 * 1024))"
export G7MB__WORKER__SANDBOX_BINARY="$ROOT/target/debug/g7mb-sandbox"
export G7MB__WORKER__TEMP_DIRECTORY="$TMP/worker"
export G7MB__WORKER__NATIVE_THREADS_PER_JOB="1"
export G7MB__WORKER__MAX_CONCURRENT_JOBS="2"
export G7MB__WORKER__MAX_CONCURRENT_HEAVY_IMAGES="1"
export G7MB__WORKER__MAX_CONCURRENT_VIDEOS="1"

mkdir -p "$TMP/worker" "$TMP/backups"
target/debug/g7mb-api --config config/g7mb.example.toml >"$API_LOG" 2>&1 &
API_PID="$!"
wait_for_api

batch_body="$(jq -nc \
    --argjson single_size "$single_size" \
    --argjson multipart_size "$multipart_size" \
    '{files: [
        {client_ref: "single-exif", declared_kind: "image", content_length: $single_size, content_type_hint: "image/jpeg"},
        {client_ref: "multipart-large", declared_kind: "image", content_length: $multipart_size, content_type_hint: "image/jpeg"}
    ]}')"
batch="$(signed_request POST /v1/upload-batches "$batch_body")"
[[ "$(jq -r '.uploads | length' <<<"$batch")" == "2" ]]
[[ "$(jq -r '[.uploads[].expires_at | type] | unique | join(",")' <<<"$batch")" == "string" ]]

single="$(jq -c '.uploads[] | select(.client_ref == "single-exif")' <<<"$batch")"
multipart="$(jq -c '.uploads[] | select(.client_ref == "multipart-large")' <<<"$batch")"
[[ "$(jq -r '.method' <<<"$single")" == "single_put" ]]
[[ "$(jq -r '.method' <<<"$multipart")" == "multipart" ]]
single_id="$(jq -er '.upload_id' <<<"$single")"
multipart_id="$(jq -er '.upload_id' <<<"$multipart")"

presigned_put "$single" "$TMP/private-exif.jpg"
signed_request POST "/v1/uploads/$single_id/complete" '' >/dev/null

part_size="$(jq -er '.part_size_bytes' <<<"$multipart")"
split -b "$part_size" "$TMP/multipart.jpg" "$TMP/part-"
parts='[]'
part_number=0
for part in "$TMP"/part-*; do
    part_number=$((part_number + 1))
    length="$(file_size "$part")"
    presign_body="$(jq -nc --argjson length "$length" '{content_length: $length}')"
    instruction="$(signed_request POST "/v1/uploads/$multipart_id/parts/$part_number/presign" "$presign_body")"
    [[ "$(jq -r '.expires_at | type' <<<"$instruction")" == "string" ]]
    headers="$TMP/part-$part_number.headers"
    presigned_put "$instruction" "$part" "$headers"
    etag="$(awk 'tolower($0) ~ /^etag:/ { sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit }' "$headers")"
    if [[ -z "$etag" ]]; then
        echo "multipart PUT returned no ETag" >&2
        exit 1
    fi
    parts="$(jq -c --argjson number "$part_number" --arg etag "$etag" \
        '. + [{part_number: $number, etag: $etag}]' <<<"$parts")"
done
if (( part_number != 2 )); then
    echo "expected exactly two multipart parts, got $part_number" >&2
    exit 1
fi
complete_body="$(jq -nc --argjson parts "$parts" '{parts: $parts}')"
signed_request POST "/v1/uploads/$multipart_id/multipart/complete" "$complete_body" >/dev/null

target/debug/g7mb-worker --config config/g7mb.example.toml once --worker-id full-stack-1 >/dev/null
target/debug/g7mb-worker --config config/g7mb.example.toml once --worker-id full-stack-2 >/dev/null

for upload_id in "$single_id" "$multipart_id"; do
    status="$(signed_request GET "/v1/uploads/$upload_id" '')"
    [[ "$(jq -r '.state' <<<"$status")" == "ready" ]]
    [[ "$(jq -r '[.derivatives[].variant] | sort | join(",")' <<<"$status")" == "master,thumbnail" ]]
    [[ "$(jq -r '.detected_content_type' <<<"$status")" == "image/jpeg" ]]

    for variant in master thumbnail; do
        delivery="$(signed_request GET "/v1/uploads/$upload_id/derivatives/$variant/delivery" '')"
        [[ "$(jq -r '.variant' <<<"$delivery")" == "$variant" ]]
        [[ "$(jq -r '.expires_at | type' <<<"$delivery")" == "string" ]]
        delivery_url="$(jq -er '.delivery_url' <<<"$delivery")"
        curl --fail-with-body --silent --show-error --output "$TMP/$upload_id-$variant.jpg" "$delivery_url"
        [[ "$(vipsheader -f format "$TMP/$upload_id-$variant.jpg")" == "uchar" ]]
    done
done

sanitized_metadata="$(vipsheader -a "$TMP/$single_id-master.jpg" 2>/dev/null)"
if [[ "$sanitized_metadata" == *"PrivateCamera"* \
    || "$sanitized_metadata" == *"GPSLatitude"* \
    || "$sanitized_metadata" == *"exif-data"* ]]; then
    echo "full-stack derivative retained private EXIF metadata" >&2
    exit 1
fi

printf 'full-stack-smoke PASS single_put=1 multipart_parts=%s ready=2 derivatives=4 exif_removed=1\n' "$part_number"
