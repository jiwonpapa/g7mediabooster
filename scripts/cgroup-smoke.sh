#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="g7mediabooster-cgroup-smoke:rust-1.96.0"
REPORT="$ROOT/reports/cgroup-smoke.json"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/g7mb-cgroup-outer.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

command -v docker >/dev/null
docker info >/dev/null

cd "$ROOT"
if ! docker build \
    --progress plain \
    --file tests/docker/cgroup-smoke.Dockerfile \
    --tag "$IMAGE" \
    . >"$TMP/build.log" 2>&1; then
    tail -n 120 "$TMP/build.log" >&2
    exit 1
fi
printf 'cgroup-smoke image built: %s\n' "$IMAGE"

docker run --rm \
    --cpus 2 \
    --memory 2g \
    --memory-swap 2g \
    --pids-limit 64 \
    --network none \
    --cap-drop ALL \
    --security-opt no-new-privileges \
    "$IMAGE" | tee "$TMP/output.log"

result_line="$(grep 'G7MB_CGROUP_RESULT' "$TMP/output.log" | tail -n 1)"
[[ -n "$result_line" ]]

field() {
    printf '%s\n' "$result_line" | tr ' ' '\n' | awk -F= -v name="$1" '$1 == name { print $2 }'
}

cpu_max="$(field cpu_max)"
memory_max="$(field memory_max)"
pids_max="$(field pids_max)"
memory_peak="$(field memory_peak)"
cpu_usage_usec="$(field cpu_usage_usec)"
health_ok="$(field health_ok)"
health_failed="$(field health_failed)"
image_id="$(docker image inspect --format '{{.Id}}' "$IMAGE")"
generated_at="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"

mkdir -p "$(dirname "$REPORT")"
cat >"$REPORT" <<JSON
{
  "schema_version": 1,
  "generated_at": "$generated_at",
  "image": "$IMAGE",
  "image_id": "$image_id",
  "network": "none",
  "cgroup": {"cpu_max": "$cpu_max", "memory_max": $memory_max, "pids_max": $pids_max},
  "memory_peak_bytes": $memory_peak,
  "cpu_usage_usec": $cpu_usage_usec,
  "api_health": {"success": $health_ok, "failed": $health_failed},
  "worker_gate": "load100",
  "result": "pass"
}
JSON

printf 'cgroup-smoke PASS cpu_max=%s memory_max=%s pids_max=%s memory_peak=%s health_ok=%s\n' \
    "$cpu_max" "$memory_max" "$pids_max" "$memory_peak" "$health_ok"
printf 'report=%s\n' "$REPORT"
