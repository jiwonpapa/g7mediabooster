#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HARNESS="$ROOT/scripts/live-storage-conformance.sh"
clean_env=(env -i "PATH=$PATH" "HOME=${HOME:-}")

set +e
missing_output="$("${clean_env[@]}" bash "$HARNESS" 2>&1)"
missing_status=$?
set -e
[[ "$missing_status" == "2" ]]
for name in \
    G7MB_LIVE_S3_LABEL \
    G7MB_LIVE_S3_REGION \
    G7MB_LIVE_S3_RAW_BUCKET \
    G7MB_LIVE_S3_DERIVATIVE_BUCKET \
    G7MB_LIVE_S3_ACCESS_KEY \
    G7MB_LIVE_S3_SECRET_KEY; do
    [[ "$missing_output" == *"$name"* ]]
done

preflight_output="$(
    "${clean_env[@]}" \
        G7MB_LIVE_S3_LABEL=r2 \
        G7MB_LIVE_S3_REGION=auto \
        G7MB_LIVE_S3_RAW_BUCKET=private-raw \
        G7MB_LIVE_S3_DERIVATIVE_BUCKET=private-media \
        G7MB_LIVE_S3_ACCESS_KEY=do-not-print-access \
        G7MB_LIVE_S3_SECRET_KEY=do-not-print-secret \
        G7MB_LIVE_S3_ENDPOINT=https://example.invalid \
        G7MB_LIVE_S3_PREFLIGHT_ONLY=true \
        bash "$HARNESS"
)"
[[ "$preflight_output" == \
    "live-storage-preflight PASS label=r2 endpoint=custom-https multipart_bytes=6291456" ]]
[[ "$preflight_output" != *"do-not-print"* ]]
[[ "$preflight_output" != *"private-raw"* ]]
[[ "$preflight_output" != *"private-media"* ]]

set +e
invalid_output="$(
    "${clean_env[@]}" \
        G7MB_LIVE_S3_LABEL=r2 \
        G7MB_LIVE_S3_REGION=auto \
        G7MB_LIVE_S3_RAW_BUCKET=raw \
        G7MB_LIVE_S3_DERIVATIVE_BUCKET=media \
        G7MB_LIVE_S3_ACCESS_KEY=access \
        G7MB_LIVE_S3_SECRET_KEY=secret \
        G7MB_LIVE_S3_ENDPOINT=http://example.invalid \
        G7MB_LIVE_S3_PREFLIGHT_ONLY=true \
        bash "$HARNESS" 2>&1
)"
invalid_status=$?
set -e
[[ "$invalid_status" == "2" ]]
[[ "$invalid_output" == *"must be a non-empty https://"* ]]

set +e
label_output="$(
    "${clean_env[@]}" \
        G7MB_LIVE_S3_LABEL='r2 unsafe' \
        G7MB_LIVE_S3_REGION=auto \
        G7MB_LIVE_S3_RAW_BUCKET=raw \
        G7MB_LIVE_S3_DERIVATIVE_BUCKET=media \
        G7MB_LIVE_S3_ACCESS_KEY=access \
        G7MB_LIVE_S3_SECRET_KEY=secret \
        G7MB_LIVE_S3_ENDPOINT=https://example.invalid \
        G7MB_LIVE_S3_PREFLIGHT_ONLY=true \
        bash "$HARNESS" 2>&1
)"
label_status=$?
set -e
[[ "$label_status" == "2" ]]
[[ "$label_output" == *"safe identifier characters"* ]]

echo "live-storage-preflight-smoke PASS missing_all=6 secret_redaction=1 https_guard=1 label_guard=1"
