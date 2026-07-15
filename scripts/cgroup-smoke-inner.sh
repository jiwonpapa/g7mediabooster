#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d /tmp/g7mb-cgroup-inner.XXXXXX)"
API_PID=""
LOAD_PID=""

cleanup() {
    if [[ -n "$LOAD_PID" ]] && kill -0 "$LOAD_PID" 2>/dev/null; then
        kill "$LOAD_PID" 2>/dev/null || true
    fi
    if [[ -n "$API_PID" ]] && kill -0 "$API_PID" 2>/dev/null; then
        kill "$API_PID" 2>/dev/null || true
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

cd "$ROOT"

cpu_max="$(tr -d '\n' </sys/fs/cgroup/cpu.max)"
memory_max="$(tr -d '\n' </sys/fs/cgroup/memory.max)"
pids_max="$(tr -d '\n' </sys/fs/cgroup/pids.max)"
[[ "$cpu_max" == "200000 100000" ]]
[[ "$memory_max" == "2147483648" ]]
[[ "$pids_max" == "64" ]]

cat >"$TMP/g7mb.toml" <<'TOML'
[server]
bind_addr = "127.0.0.1:8088"
request_body_limit_bytes = 1048576

[auth]
key_id = "cgroup-smoke"
tenant_id = "cgroup-site"
hmac_secret = "0123456789abcdef0123456789abcdef"
allowed_skew_seconds = 300

[storage]
endpoint_url = "http://127.0.0.1:9000"
region = "us-east-1"
raw_bucket = "raw-private"
derivative_bucket = "derivatives"
access_key_id = "credential-free-health-smoke"
secret_access_key = "credential-free-health-smoke"
force_path_style = true

[database]
url = "sqlite:///tmp/g7mb-cgroup-api.db"
max_connections = 2
TOML

"$ROOT/target/debug/g7mb-api" --config "$TMP/g7mb.toml" >"$TMP/api.log" 2>&1 &
API_PID=$!
for _ in $(seq 1 100); do
    if curl --silent --fail --max-time 1 http://127.0.0.1:8088/health/live >/dev/null; then
        break
    fi
    if ! kill -0 "$API_PID" 2>/dev/null; then
        tail -n 80 "$TMP/api.log" >&2
        exit 1
    fi
    sleep 0.05
done
curl --silent --fail --max-time 1 http://127.0.0.1:8088/health/live >/dev/null

cargo xtask load100 >"$TMP/load.log" 2>&1 &
LOAD_PID=$!
health_ok=0
health_failed=0
while kill -0 "$LOAD_PID" 2>/dev/null; do
    if curl --silent --fail --max-time 1 http://127.0.0.1:8088/health/live >/dev/null; then
        health_ok=$((health_ok + 1))
    else
        health_failed=$((health_failed + 1))
    fi
    sleep 0.05
done

set +e
wait "$LOAD_PID"
load_status=$?
set -e
LOAD_PID=""
if (( load_status != 0 )); then
    tail -n 120 "$TMP/load.log" >&2
    exit "$load_status"
fi
if (( health_ok == 0 || health_failed != 0 )); then
    echo "API health failed during cgroup worker load" >&2
    exit 1
fi
if ! kill -0 "$API_PID" 2>/dev/null; then
    tail -n 80 "$TMP/api.log" >&2
    exit 1
fi

load_result="$(grep 'load-100 PASS' "$TMP/load.log" | tail -n 1)"
[[ -n "$load_result" ]]
memory_peak="$(tr -d '\n' </sys/fs/cgroup/memory.peak)"
cpu_usage_usec="$(awk '$1 == "usage_usec" {print $2}' /sys/fs/cgroup/cpu.stat)"

printf 'G7MB_CGROUP_RESULT cpu_max=%s memory_max=%s pids_max=%s memory_peak=%s cpu_usage_usec=%s health_ok=%s health_failed=%s\n' \
    "${cpu_max// /,}" "$memory_max" "$pids_max" "$memory_peak" "$cpu_usage_usec" \
    "$health_ok" "$health_failed"
printf '%s\n' "$load_result"
