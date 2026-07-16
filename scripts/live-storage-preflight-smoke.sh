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
    G7MB_LIVE_S3_PROFILE \
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
        G7MB_LIVE_S3_PROFILE=r2 \
        G7MB_LIVE_S3_REGION=auto \
        G7MB_LIVE_S3_RAW_BUCKET=private-raw \
        G7MB_LIVE_S3_DERIVATIVE_BUCKET=private-media \
        G7MB_LIVE_S3_ACCESS_KEY=do-not-print-access \
        G7MB_LIVE_S3_SECRET_KEY=do-not-print-secret \
        G7MB_LIVE_S3_ENDPOINT=https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com \
        G7MB_LIVE_S3_PREFLIGHT_ONLY=true \
        bash "$HARNESS"
)"
[[ "$preflight_output" == \
    "live-storage-preflight PASS profile=r2 label=r2 endpoint=custom-https multipart_bytes=6291456" ]]
[[ "$preflight_output" != *"do-not-print"* ]]
[[ "$preflight_output" != *"private-raw"* ]]
[[ "$preflight_output" != *"private-media"* ]]

set +e
invalid_output="$(
    "${clean_env[@]}" \
        G7MB_LIVE_S3_LABEL=r2 \
        G7MB_LIVE_S3_PROFILE=r2 \
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
        G7MB_LIVE_S3_PROFILE=r2 \
        G7MB_LIVE_S3_REGION=auto \
        G7MB_LIVE_S3_RAW_BUCKET=raw \
        G7MB_LIVE_S3_DERIVATIVE_BUCKET=media \
        G7MB_LIVE_S3_ACCESS_KEY=access \
        G7MB_LIVE_S3_SECRET_KEY=secret \
        G7MB_LIVE_S3_ENDPOINT=https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com \
        G7MB_LIVE_S3_PREFLIGHT_ONLY=true \
        bash "$HARNESS" 2>&1
)"
label_status=$?
set -e
[[ "$label_status" == "2" ]]
[[ "$label_output" == *"safe identifier characters"* ]]

set +e
profile_output="$(
    "${clean_env[@]}" \
        G7MB_LIVE_S3_PROFILE=r2 \
        G7MB_LIVE_S3_LABEL=r2 \
        G7MB_LIVE_S3_REGION=us-east-1 \
        G7MB_LIVE_S3_RAW_BUCKET=raw \
        G7MB_LIVE_S3_DERIVATIVE_BUCKET=media \
        G7MB_LIVE_S3_ACCESS_KEY=access \
        G7MB_LIVE_S3_SECRET_KEY=secret \
        G7MB_LIVE_S3_ENDPOINT=https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com \
        G7MB_LIVE_S3_PREFLIGHT_ONLY=true \
        bash "$HARNESS" 2>&1
)"
profile_status=$?
set -e
[[ "$profile_status" == "2" ]]
[[ "$profile_output" == *"region=auto"* ]]

lightsail_output="$(
    "${clean_env[@]}" \
        G7MB_LIVE_S3_PROFILE=lightsail \
        G7MB_LIVE_S3_LABEL=lightsail \
        G7MB_LIVE_S3_REGION=ap-northeast-2 \
        G7MB_LIVE_S3_RAW_BUCKET=one-private-bucket \
        G7MB_LIVE_S3_DERIVATIVE_BUCKET=one-private-bucket \
        G7MB_LIVE_S3_ACCESS_KEY=do-not-print-access \
        G7MB_LIVE_S3_SECRET_KEY=do-not-print-secret \
        G7MB_LIVE_S3_PREFLIGHT_ONLY=true \
        bash "$HARNESS"
)"
[[ "$lightsail_output" == \
    "live-storage-preflight PASS profile=lightsail label=lightsail endpoint=aws-default multipart_bytes=6291456" ]]

echo "live-storage-preflight-smoke PASS missing_all=7 secret_redaction=1 https_guard=1 label_guard=1 profile_guard=1 lightsail_shape=1"
