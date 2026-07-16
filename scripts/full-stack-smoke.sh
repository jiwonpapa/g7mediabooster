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
POLICY_SMOKE="${G7MB_FULL_STACK_POLICY_SMOKE:-false}"
LARGE_MULTIPART_BYTES="${G7MB_FULL_STACK_LARGE_MULTIPART_BYTES:-0}"
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
if [[ "$POLICY_SMOKE" != false && "$POLICY_SMOKE" != true ]]; then
    echo "G7MB_FULL_STACK_POLICY_SMOKE must be true or false" >&2
    exit 2
fi
if [[ ! "$LARGE_MULTIPART_BYTES" =~ ^[0-9]+$ ]] \
    || (( LARGE_MULTIPART_BYTES != 0 \
        && (LARGE_MULTIPART_BYTES < 5 * 1024 * 1024 \
            || LARGE_MULTIPART_BYTES > 5 * 1024 * 1024 * 1024) )); then
    echo "G7MB_FULL_STACK_LARGE_MULTIPART_BYTES must be 0 or between 5MiB and 5GiB" >&2
    exit 2
fi
if (( LARGE_MULTIPART_BYTES > 0 )); then
    command -v truncate >/dev/null
fi
if [[ "$POLICY_SMOKE" == true ]]; then
    command -v php >/dev/null
fi

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

start_api() {
    target/debug/g7mb-api --config config/g7mb.example.toml >>"$API_LOG" 2>&1 &
    API_PID="$!"
    wait_for_api
}

upload_single_image() {
    local source="$1"
    local client_ref="$2"
    local content_type="${3:-image/jpeg}"
    local size batch instruction upload_id
    size="$(file_size "$source")"
    batch="$(jq -nc --arg client_ref "$client_ref" --arg content_type "$content_type" --argjson size "$size" \
        '{files: [{client_ref: $client_ref, declared_kind: "image", content_length: $size, content_type_hint: $content_type}]}')"
    instruction="$(signed_request POST /v1/upload-batches "$batch" | jq -ec '.uploads[0]')"
    [[ "$(jq -r '.method' <<<"$instruction")" == "single_put" ]]
    upload_id="$(jq -er '.upload_id' <<<"$instruction")"
    presigned_put "$instruction" "$source"
    signed_request POST "/v1/uploads/$upload_id/complete" '' >/dev/null
    printf '%s\n' "$upload_id"
}

download_derivative() {
    local upload_id="$1"
    local variant="$2"
    local output="$3"
    local delivery delivery_url
    delivery="$(signed_request GET "/v1/uploads/$upload_id/derivatives/$variant/delivery" '')"
    [[ "$(jq -r '.variant' <<<"$delivery")" == "$variant" ]]
    delivery_url="$(jq -er '.delivery_url' <<<"$delivery")"
    curl --fail-with-body --silent --show-error --output "$output" "$delivery_url"
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
ffmpeg -hide_banner -loglevel error -nostdin \
    -f lavfi -i "color=c=blue:s=320x180:r=10" \
    -t 1 -c:v libx264 -pix_fmt yuv420p -threads 1 -map_metadata -1 -f mov -y "$TMP/video.mov"

single_size="$(file_size "$TMP/private-exif.jpg")"
multipart_size="$(file_size "$TMP/multipart.jpg")"
mov_size="$(file_size "$TMP/video.mov")"
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
sandbox_capabilities="$(target/debug/g7mb-sandbox capabilities)"
printf 'sandbox-capabilities %s\n' "$sandbox_capabilities"
jq -e '
    (["avif", "gif", "heif", "jpeg", "png", "webp"] - .image_inputs | length == 0)
    and (["avif", "jpeg", "png", "webp"] - .image_outputs | length == 0)
    and (["mov", "mp4"] - .video_inputs | length == 0)
    and .mp4_thumbnail
    and .mp4_h264_fallback
' <<<"$sandbox_capabilities" >/dev/null

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
if (( LARGE_MULTIPART_BYTES > 0 )); then
    export G7MB__UPLOAD__MULTIPART_PART_SIZE_BYTES="$((32 * 1024 * 1024))"
else
    export G7MB__UPLOAD__MULTIPART_PART_SIZE_BYTES="$((5 * 1024 * 1024))"
fi
export G7MB__WORKER__SANDBOX_BINARY="$ROOT/target/debug/g7mb-sandbox"
export G7MB__WORKER__TEMP_DIRECTORY="$TMP/worker"
export G7MB__WORKER__NATIVE_THREADS_PER_JOB="1"
export G7MB__WORKER__MAX_CONCURRENT_JOBS="2"
export G7MB__WORKER__MAX_CONCURRENT_HEAVY_IMAGES="1"
export G7MB__WORKER__MAX_CONCURRENT_VIDEOS="1"

mkdir -p "$TMP/worker" "$TMP/backups"
start_api

if (( LARGE_MULTIPART_BYTES > 0 )); then
    large_batch_body="$(jq -nc --argjson size "$LARGE_MULTIPART_BYTES" \
        '{files: [{client_ref: "large-video", declared_kind: "video", content_length: $size, content_type_hint: "video/mp4"}]}')"
    large_instruction="$(signed_request POST /v1/upload-batches "$large_batch_body" | jq -ec '.uploads[0]')"
    [[ "$(jq -r '.method' <<<"$large_instruction")" == "multipart" ]]
    large_upload_id="$(jq -er '.upload_id' <<<"$large_instruction")"
    large_part_size="$(jq -er '.part_size_bytes' <<<"$large_instruction")"
    large_part_count="$(((LARGE_MULTIPART_BYTES + large_part_size - 1) / large_part_size))"
    [[ "$large_part_count" -le 10000 ]]
    large_parts='[]'
    large_part_file="$TMP/large-part.bin"
    api_rss_start="$(ps -o rss= -p "$API_PID" | awk '{print $1}')"
    api_rss_peak="$api_rss_start"
    api_restarts=0

    for ((part_number = 1; part_number <= large_part_count; part_number += 1)); do
        offset="$(((part_number - 1) * large_part_size))"
        length="$((LARGE_MULTIPART_BYTES - offset))"
        if (( length > large_part_size )); then
            length="$large_part_size"
        fi
        truncate -s "$length" "$large_part_file"
        presign_body="$(jq -nc --argjson length "$length" '{content_length: $length}')"
        instruction="$(signed_request POST "/v1/uploads/$large_upload_id/parts/$part_number/presign" "$presign_body")"
        headers="$TMP/large-part-$part_number.headers"
        presigned_put "$instruction" "$large_part_file" "$headers"
        etag="$(awk 'tolower($0) ~ /^etag:/ { sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit }' "$headers")"
        if [[ -z "$etag" ]]; then
            echo "large multipart PUT returned no ETag for part $part_number" >&2
            exit 1
        fi
        large_parts="$(jq -c --argjson number "$part_number" --arg etag "$etag" \
            '. + [{part_number: $number, etag: $etag}]' <<<"$large_parts")"
        current_rss="$(ps -o rss= -p "$API_PID" | awk '{print $1}')"
        if (( current_rss > api_rss_peak )); then
            api_rss_peak="$current_rss"
        fi
        if (( part_number == large_part_count / 2 )); then
            kill "$API_PID"
            wait "$API_PID" || true
            API_PID=""
            start_api
            api_restarts=$((api_restarts + 1))
            current_rss="$(ps -o rss= -p "$API_PID" | awk '{print $1}')"
            if (( current_rss > api_rss_peak )); then
                api_rss_peak="$current_rss"
            fi
        fi
        if (( part_number % 20 == 0 || part_number == large_part_count )); then
            printf 'large-multipart progress parts=%s/%s\n' "$part_number" "$large_part_count" >&2
        fi
    done

    complete_body="$(jq -nc --argjson parts "$large_parts" '{parts: $parts}')"
    signed_request POST "/v1/uploads/$large_upload_id/multipart/complete" "$complete_body" >/dev/null
    signed_request POST "/v1/uploads/$large_upload_id/multipart/complete" "$complete_body" >/dev/null
    large_status="$(signed_request GET "/v1/uploads/$large_upload_id" '')"
    [[ "$(jq -r '.state' <<<"$large_status")" == "quarantined" ]]
    current_rss="$(ps -o rss= -p "$API_PID" | awk '{print $1}')"
    if (( current_rss > api_rss_peak )); then
        api_rss_peak="$current_rss"
    fi
    api_rss_delta="$((api_rss_peak - api_rss_start))"
    if (( api_rss_delta > 32768 )); then
        echo "API RSS grew more than 32MiB during body-free multipart control: ${api_rss_delta}KiB" >&2
        exit 1
    fi
    [[ "$api_restarts" == "1" ]]
    printf 'large-multipart-smoke PASS bytes=%s parts=%s api_rss_start_kib=%s api_rss_peak_kib=%s api_rss_delta_kib=%s direct_body=1 api_restarts=%s duplicate_complete=1 quarantined=1\n' \
        "$LARGE_MULTIPART_BYTES" "$large_part_count" "$api_rss_start" "$api_rss_peak" "$api_rss_delta" "$api_restarts"
    exit 0
fi

batch_body="$(jq -nc \
    --argjson single_size "$single_size" \
    --argjson multipart_size "$multipart_size" \
    --argjson mov_size "$mov_size" \
    '{files: [
        {client_ref: "single-exif", declared_kind: "image", content_length: $single_size, content_type_hint: "image/jpeg"},
        {client_ref: "multipart-large", declared_kind: "image", content_length: $multipart_size, content_type_hint: "image/jpeg"},
        {client_ref: "mov-video", declared_kind: "video", content_length: $mov_size, content_type_hint: "video/quicktime"}
    ]}')"
batch="$(signed_request POST /v1/upload-batches "$batch_body")"
[[ "$(jq -r '.uploads | length' <<<"$batch")" == "3" ]]
[[ "$(jq -r '[.uploads[].expires_at | type] | unique | join(",")' <<<"$batch")" == "string" ]]

single="$(jq -c '.uploads[] | select(.client_ref == "single-exif")' <<<"$batch")"
multipart="$(jq -c '.uploads[] | select(.client_ref == "multipart-large")' <<<"$batch")"
mov="$(jq -c '.uploads[] | select(.client_ref == "mov-video")' <<<"$batch")"
[[ "$(jq -r '.method' <<<"$single")" == "single_put" ]]
[[ "$(jq -r '.method' <<<"$multipart")" == "multipart" ]]
[[ "$(jq -r '.method' <<<"$mov")" == "multipart" ]]
single_id="$(jq -er '.upload_id' <<<"$single")"
multipart_id="$(jq -er '.upload_id' <<<"$multipart")"
mov_id="$(jq -er '.upload_id' <<<"$mov")"

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

mov_presign_body="$(jq -nc --argjson length "$mov_size" '{content_length: $length}')"
mov_instruction="$(signed_request POST "/v1/uploads/$mov_id/parts/1/presign" "$mov_presign_body")"
mov_headers="$TMP/mov-part.headers"
presigned_put "$mov_instruction" "$TMP/video.mov" "$mov_headers"
mov_etag="$(awk 'tolower($0) ~ /^etag:/ { sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit }' "$mov_headers")"
if [[ -z "$mov_etag" ]]; then
    echo "MOV multipart PUT returned no ETag" >&2
    exit 1
fi
mov_complete_body="$(jq -nc --arg etag "$mov_etag" '{parts: [{part_number: 1, etag: $etag}]}')"
signed_request POST "/v1/uploads/$mov_id/multipart/complete" "$mov_complete_body" >/dev/null

target/debug/g7mb-worker --config config/g7mb.example.toml once --worker-id full-stack-1 >/dev/null
target/debug/g7mb-worker --config config/g7mb.example.toml once --worker-id full-stack-2 >/dev/null
target/debug/g7mb-worker --config config/g7mb.example.toml once --worker-id full-stack-3 >/dev/null

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

mov_status="$(signed_request GET "/v1/uploads/$mov_id" '')"
[[ "$(jq -r '.state' <<<"$mov_status")" == "ready" ]]
[[ "$(jq -r '.detected_content_type' <<<"$mov_status")" == "video/quicktime" ]]
[[ "$(jq -r '[.derivatives[].variant] | sort | join(",")' <<<"$mov_status")" == "master,thumbnail" ]]
[[ "$(jq -r '.derivatives[] | select(.variant == "master") | .content_type' <<<"$mov_status")" == "video/quicktime" ]]
download_derivative "$mov_id" master "$TMP/mov-master.mov"
download_derivative "$mov_id" thumbnail "$TMP/mov-thumbnail.jpg"
cmp -s "$TMP/video.mov" "$TMP/mov-master.mov"
[[ "$(vipsheader -f format "$TMP/mov-thumbnail.jpg")" == "uchar" ]]
mov_format="$(ffprobe -v error -show_entries format=format_name -of default=nw=1:nk=1 "$TMP/mov-master.mov")"
[[ "$mov_format" == *"mov"* ]]

sanitized_metadata="$(vipsheader -a "$TMP/$single_id-master.jpg" 2>/dev/null)"
if [[ "$sanitized_metadata" == *"PrivateCamera"* \
    || "$sanitized_metadata" == *"GPSLatitude"* \
    || "$sanitized_metadata" == *"exif-data"* ]]; then
    echo "full-stack derivative retained private EXIF metadata" >&2
    exit 1
fi

if [[ "$POLICY_SMOKE" == true ]]; then
    ffmpeg -hide_banner -loglevel error -nostdin \
        -f lavfi -i "color=c=blue:s=320x160" \
        -frames:v 1 -threads 1 -y "$TMP/watermark.png"
    watermark_upload_id="$(upload_single_image "$TMP/watermark.png" watermark-source image/png)"
    target/debug/g7mb-worker --config config/g7mb.example.toml once --worker-id full-stack-watermark-source >/dev/null
    watermark_status="$(signed_request GET "/v1/uploads/$watermark_upload_id" '')"
    [[ "$(jq -r '.state' <<<"$watermark_status")" == "ready" ]]
    [[ "$(jq -r '.detected_content_type' <<<"$watermark_status")" == "image/png" ]]

    policy_result="$(
        G7MB_POLICY_ENDPOINT="$API_BASE" \
        G7MB_POLICY_HMAC_SECRET="$HMAC_SECRET" \
        G7MB_POLICY_ASSET_UPLOAD_ID="$watermark_upload_id" \
        G7MB_POLICY_REVISION=1 \
        php "$ROOT/adapters/gnuboard7/jiwonpapa-g7mediabooster/tests/Live/publish-site-policy.php"
    )"
    [[ "$(jq -r '.revision' <<<"$policy_result")" == "1" ]]
    [[ "$(jq -r '.watermark.asset_upload_id' <<<"$policy_result")" == "$watermark_upload_id" ]]
    watermark_sha256="$(jq -er '.watermark.asset_sha256' <<<"$policy_result")"
    [[ "$watermark_sha256" =~ ^[a-f0-9]{64}$ ]]

    policy_upload_id="$(upload_single_image "$TMP/private-exif.jpg" policy-enabled)"
    target/debug/g7mb-worker --config config/g7mb.example.toml once --worker-id full-stack-policy >/dev/null
    policy_status="$(signed_request GET "/v1/uploads/$policy_upload_id" '')"
    [[ "$(jq -r '.state' <<<"$policy_status")" == "ready" ]]
    expected_policy_preset="board-default-v1-wm-g7-r1-$watermark_sha256"
    [[ "$(jq -r '[.derivatives[].preset_id] | unique | join(",")' <<<"$policy_status")" == "$expected_policy_preset" ]]
    download_derivative "$policy_upload_id" master "$TMP/policy-master.jpg"
    download_derivative "$policy_upload_id" thumbnail "$TMP/policy-thumbnail.jpg"
    if cmp -s "$TMP/$single_id-master.jpg" "$TMP/policy-master.jpg"; then
        echo "watermark policy did not change the deterministic master bytes" >&2
        exit 1
    fi

    rollback_result="$(
        G7MB_POLICY_ENDPOINT="$API_BASE" \
        G7MB_POLICY_HMAC_SECRET="$HMAC_SECRET" \
        G7MB_POLICY_ASSET_UPLOAD_ID='' \
        G7MB_POLICY_REVISION=2 \
        php "$ROOT/adapters/gnuboard7/jiwonpapa-g7mediabooster/tests/Live/publish-site-policy.php"
    )"
    [[ "$(jq -r '.revision' <<<"$rollback_result")" == "2" ]]
    [[ "$(jq -r '.watermark == null' <<<"$rollback_result")" == "true" ]]

    rollback_upload_id="$(upload_single_image "$TMP/private-exif.jpg" policy-disabled)"
    target/debug/g7mb-worker --config config/g7mb.example.toml once --worker-id full-stack-rollback >/dev/null
    rollback_status="$(signed_request GET "/v1/uploads/$rollback_upload_id" '')"
    [[ "$(jq -r '.state' <<<"$rollback_status")" == "ready" ]]
    [[ "$(jq -r '[.derivatives[].preset_id] | unique | join(",")' <<<"$rollback_status")" == "board-default-v1" ]]
    download_derivative "$rollback_upload_id" master "$TMP/rollback-master.jpg"
    if ! cmp -s "$TMP/$single_id-master.jpg" "$TMP/rollback-master.jpg"; then
        echo "disabled policy did not restore the deterministic unwatermarked master" >&2
        exit 1
    fi

    printf 'g7-policy-smoke PASS php_hmac=1 applied_revision=1 rollback_revision=2 worker_pinned=1\n'
fi

printf 'full-stack-smoke PASS single_put=1 multipart_parts=%s ready=3 derivatives=6 mov_h264=1 exif_removed=1\n' "$part_number"
