#!/usr/bin/env bash
set -euo pipefail

required=(
    G7MB_LIVE_S3_LABEL
    G7MB_LIVE_S3_REGION
    G7MB_LIVE_S3_RAW_BUCKET
    G7MB_LIVE_S3_DERIVATIVE_BUCKET
    G7MB_LIVE_S3_ACCESS_KEY
    G7MB_LIVE_S3_SECRET_KEY
)
missing=()
for name in "${required[@]}"; do
    if [[ -z "${!name:-}" ]]; then
        missing+=("$name")
    fi
done
if (( ${#missing[@]} > 0 )); then
    for name in "${missing[@]}"; do
        echo "missing required environment variable: $name" >&2
    done
    exit 2
fi
if [[ ! "$G7MB_LIVE_S3_LABEL" =~ ^[A-Za-z0-9._-]{1,64}$ ]]; then
    echo "G7MB_LIVE_S3_LABEL must use 1-64 safe identifier characters" >&2
    exit 2
fi

force_path_style="${G7MB_LIVE_S3_FORCE_PATH_STYLE:-false}"
if [[ "$force_path_style" != "true" && "$force_path_style" != "false" ]]; then
    echo "G7MB_LIVE_S3_FORCE_PATH_STYLE must be true or false" >&2
    exit 2
fi
if [[ -n "${G7MB_LIVE_S3_ENDPOINT:-}" \
    && ! "$G7MB_LIVE_S3_ENDPOINT" =~ ^https://[^[:space:]]+$ ]]; then
    echo "G7MB_LIVE_S3_ENDPOINT must be a non-empty https:// URL without whitespace" >&2
    exit 2
fi

large_bytes="${G7MB_LIVE_S3_LARGE_BYTES:-6291456}"
if [[ ! "$large_bytes" =~ ^[0-9]+$ ]]; then
    echo "G7MB_LIVE_S3_LARGE_BYTES must be an integer between 5 MiB and 5 GiB" >&2
    exit 2
fi
large_bytes_number=$((10#$large_bytes))
if (( large_bytes_number < 5 * 1024 * 1024 \
    || large_bytes_number > 5 * 1024 * 1024 * 1024 )); then
    echo "G7MB_LIVE_S3_LARGE_BYTES must be an integer between 5 MiB and 5 GiB" >&2
    exit 2
fi

preflight_only="${G7MB_LIVE_S3_PREFLIGHT_ONLY:-false}"
if [[ "$preflight_only" != "true" && "$preflight_only" != "false" ]]; then
    echo "G7MB_LIVE_S3_PREFLIGHT_ONLY must be true or false" >&2
    exit 2
fi
for command_name in cargo curl; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "required command is unavailable: $command_name" >&2
        exit 2
    fi
done

endpoint_mode="aws-default"
if [[ -n "${G7MB_LIVE_S3_ENDPOINT:-}" ]]; then
    endpoint_mode="custom-https"
fi
echo "live-storage-preflight PASS label=$G7MB_LIVE_S3_LABEL endpoint=$endpoint_mode multipart_bytes=$large_bytes"
if [[ "$preflight_only" == "true" ]]; then
    exit 0
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
cargo test --locked --package g7mb-object-store-s3 \
    --test live_provider_conformance \
    -- --ignored --nocapture --test-threads=1
