#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/g7mb-load-100.XXXXXX")"
LOG="$TMP/load-100.log"
REPORT="$ROOT/reports/load-100.json"
MAX_RSS_KIB="${G7MB_LOAD_MAX_RSS_KIB:-1572864}"
MAX_WALL_SECONDS="${G7MB_LOAD_MAX_WALL_SECONDS:-240}"
CONCURRENCY="${G7MB_LOAD_CONCURRENCY:-4}"
TEST_PID=""

cleanup() {
    if [[ -n "$TEST_PID" ]] && kill -0 "$TEST_PID" 2>/dev/null; then
        kill "$TEST_PID" 2>/dev/null || true
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

case "$MAX_RSS_KIB" in
    ''|*[!0-9]*) echo "G7MB_LOAD_MAX_RSS_KIB must be a positive integer" >&2; exit 2 ;;
esac
case "$MAX_WALL_SECONDS" in
    ''|*[!0-9]*) echo "G7MB_LOAD_MAX_WALL_SECONDS must be a positive integer" >&2; exit 2 ;;
esac

command -v ffmpeg >/dev/null
command -v vipsheader >/dev/null

cd "$ROOT"
export VIPS_CONCURRENCY=1

ffmpeg -hide_banner -loglevel error -nostdin \
    -f lavfi -i "testsrc2=size=4000x3000:rate=1" \
    -frames:v 1 -c:v mjpeg -q:v 3 -threads 1 -y "$TMP/fixture.jpg"
[[ "$(vipsheader -f width "$TMP/fixture.jpg")" == "4000" ]]
[[ "$(vipsheader -f height "$TMP/fixture.jpg")" == "3000" ]]
fixture_sha256="$(shasum -a 256 "$TMP/fixture.jpg" | awk '{print $1}')"

cargo build --quiet --locked --package g7mb-sandbox --features native-vips
cargo test --quiet --locked --package g7mb-worker --test load_100 --no-run

tree_rss_kib() {
    ps -axo pid=,ppid=,rss= | awk -v root="$1" '
        { pid[count] = $1; parent[count] = $2; rss[count] = $3; count++ }
        END {
            selected[root] = 1
            changed = 1
            while (changed) {
                changed = 0
                for (row = 0; row < count; row++) {
                    if (selected[parent[row]] && !selected[pid[row]]) {
                        selected[pid[row]] = 1
                        changed = 1
                    }
                }
            }
            total = 0
            for (row = 0; row < count; row++) {
                if (selected[pid[row]]) total += rss[row]
            }
            print total
        }
    '
}

started_epoch="$(date +%s)"
G7MB_LOAD_FIXTURE="$TMP/fixture.jpg" \
G7MB_SANDBOX_BIN="$ROOT/target/debug/g7mb-sandbox" \
G7MB_LOAD_CONCURRENCY="$CONCURRENCY" \
cargo test --quiet --locked --package g7mb-worker --test load_100 \
    load_100_real_jpeg_recovers_expired_leases -- --ignored --exact --nocapture \
    >"$LOG" 2>&1 &
TEST_PID=$!

peak_rss_kib=0
timed_out=0
while kill -0 "$TEST_PID" 2>/dev/null; do
    current_rss_kib="$(tree_rss_kib "$TEST_PID")"
    if (( current_rss_kib > peak_rss_kib )); then
        peak_rss_kib="$current_rss_kib"
    fi
    now_epoch="$(date +%s)"
    if (( now_epoch - started_epoch > MAX_WALL_SECONDS )); then
        timed_out=1
        kill "$TEST_PID" 2>/dev/null || true
        break
    fi
    sleep 0.1
done

set +e
wait "$TEST_PID"
test_status=$?
set -e
TEST_PID=""

if (( timed_out == 1 )); then
    echo "load gate exceeded ${MAX_WALL_SECONDS}s" >&2
    tail -n 80 "$LOG" >&2
    exit 1
fi
if (( test_status != 0 )); then
    tail -n 120 "$LOG" >&2
    exit "$test_status"
fi

result_line="$(grep 'G7MB_LOAD_RESULT' "$LOG" | tail -n 1)"
if [[ -z "$result_line" ]]; then
    echo "load test did not emit G7MB_LOAD_RESULT" >&2
    exit 1
fi

field() {
    printf '%s\n' "$result_line" | tr ' ' '\n' | awk -F= -v name="$1" '$1 == name { print $2 }'
}

jobs="$(field jobs)"
elapsed_ms="$(field elapsed_ms)"
throughput="$(field throughput_per_second)"
p50_ms="$(field p50_ms)"
p95_ms="$(field p95_ms)"
p99_ms="$(field p99_ms)"
ready="$(field ready)"
completed="$(field completed)"
derivatives="$(field derivatives)"
recovered="$(field recovered)"
dead_letter="$(field dead_letter)"

if (( peak_rss_kib > MAX_RSS_KIB )); then
    echo "load gate peak RSS ${peak_rss_kib} KiB exceeded ${MAX_RSS_KIB} KiB" >&2
    exit 1
fi

mkdir -p "$(dirname "$REPORT")"
generated_at="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
cat >"$REPORT" <<JSON
{
  "schema_version": 1,
  "generated_at": "$generated_at",
  "fixture": {"format": "jpeg", "width": 4000, "height": 3000, "sha256": "$fixture_sha256"},
  "jobs": $jobs,
  "worker_concurrency": $CONCURRENCY,
  "native_threads_per_job": 1,
  "elapsed_ms": $elapsed_ms,
  "throughput_per_second": $throughput,
  "processing_latency_ms": {"p50": $p50_ms, "p95": $p95_ms, "p99": $p99_ms},
  "peak_process_tree_rss_kib": $peak_rss_kib,
  "max_process_tree_rss_kib": $MAX_RSS_KIB,
  "ready": $ready,
  "completed_jobs": $completed,
  "derivatives": $derivatives,
  "recovered_expired_leases": $recovered,
  "dead_letter": $dead_letter,
  "result": "pass"
}
JSON

printf 'load-100 PASS jobs=%s elapsed_ms=%s throughput_per_second=%s p95_ms=%s peak_rss_kib=%s recovered=%s\n' \
    "$jobs" "$elapsed_ms" "$throughput" "$p95_ms" "$peak_rss_kib" "$recovered"
printf 'report=%s\n' "$REPORT"
