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
for name in "${required[@]}"; do
    if [[ -z "${!name:-}" ]]; then
        echo "missing required environment variable: $name" >&2
        exit 2
    fi
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
cargo test --locked --package g7mb-object-store-s3 \
    --test live_provider_conformance \
    live_provider_single_multipart_and_delete_conformance \
    -- --ignored --exact --nocapture
