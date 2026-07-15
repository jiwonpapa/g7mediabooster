#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/g7mb-heavy-avif.XXXXXX")"
REPORT="$ROOT/reports/heavy-avif.json"
MAX_RSS_KIB="${G7MB_HEAVY_AVIF_MAX_RSS_KIB:-1572864}"
MAX_WALL_SECONDS="${G7MB_HEAVY_AVIF_MAX_WALL_SECONDS:-90}"
PROCESS_PID=""

cleanup() {
    if [[ -n "$PROCESS_PID" ]] && kill -0 "$PROCESS_PID" 2>/dev/null; then
        kill "$PROCESS_PID" 2>/dev/null || true
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

case "$MAX_RSS_KIB" in
    ''|*[!0-9]*) echo "G7MB_HEAVY_AVIF_MAX_RSS_KIB must be a positive integer" >&2; exit 2 ;;
esac
case "$MAX_WALL_SECONDS" in
    ''|*[!0-9]*) echo "G7MB_HEAVY_AVIF_MAX_WALL_SECONDS must be a positive integer" >&2; exit 2 ;;
esac

command -v vips >/dev/null
command -v vipsheader >/dev/null
command -v perl >/dev/null

cd "$ROOT"
export VIPS_CONCURRENCY=1

now_ms() {
    perl -MTime::HiRes=time -e 'printf "%.0f\n", time() * 1000'
}

fixture_started_ms="$(now_ms)"
# 64 MP is the AVIF/HEIF production cap under the default 2 GiB worker cgroup.
vips black "$TMP/source.avif" 8000 8000 --bands 3
# Exactly 200 MP: this must be rejected after header probing, before full-frame decode.
vips black "$TMP/rejected-200mp.avif" 16000 12500 --bands 3
fixture_elapsed_ms=$(( $(now_ms) - fixture_started_ms ))
[[ "$(vipsheader -f width "$TMP/source.avif")" == "8000" ]]
[[ "$(vipsheader -f height "$TMP/source.avif")" == "8000" ]]
[[ "$(vipsheader -f width "$TMP/rejected-200mp.avif")" == "16000" ]]
[[ "$(vipsheader -f height "$TMP/rejected-200mp.avif")" == "12500" ]]
if command -v sha256sum >/dev/null; then
    fixture_sha256="$(sha256sum "$TMP/source.avif" | awk '{print $1}')"
else
    fixture_sha256="$(shasum -a 256 "$TMP/source.avif" | awk '{print $1}')"
fi
if stat -f%z "$TMP/source.avif" >/dev/null 2>&1; then
    fixture_bytes="$(stat -f%z "$TMP/source.avif")"
    rejected_bytes="$(stat -f%z "$TMP/rejected-200mp.avif")"
else
    fixture_bytes="$(stat -c%s "$TMP/source.avif")"
    rejected_bytes="$(stat -c%s "$TMP/rejected-200mp.avif")"
fi

cargo build --quiet --locked --package g7mb-sandbox --features native-vips

set +e
"$ROOT/target/debug/g7mb-sandbox" probe \
    --input "$TMP/rejected-200mp.avif" \
    --declared-kind image \
    --byte-len "$rejected_bytes" \
    --threads 1 >"$TMP/rejected.json" 2>"$TMP/rejected.log"
rejected_status=$?
set -e
if (( rejected_status == 0 )); then
    echo "200 MP AVIF was unexpectedly accepted" >&2
    exit 1
fi
grep -q 'image resource policy rejected the source' "$TMP/rejected.log"

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

started_ms="$(now_ms)"
started_epoch="$(date +%s)"
(
    "$ROOT/target/debug/g7mb-sandbox" probe \
        --input "$TMP/source.avif" \
        --declared-kind image \
        --byte-len "$fixture_bytes" \
        --threads 1 >"$TMP/probe.json"
    "$ROOT/target/debug/g7mb-sandbox" image-thumbnail \
        --input "$TMP/source.avif" \
        --output "$TMP/thumbnail.jpg" \
        --max-edge 1280 \
        --format jpeg \
        --threads 1
) >"$TMP/process.log" 2>&1 &
PROCESS_PID=$!

peak_rss_kib=0
timed_out=0
while kill -0 "$PROCESS_PID" 2>/dev/null; do
    current_rss_kib="$(tree_rss_kib "$PROCESS_PID")"
    if (( current_rss_kib > peak_rss_kib )); then
        peak_rss_kib="$current_rss_kib"
    fi
    now_epoch="$(date +%s)"
    if (( now_epoch - started_epoch > MAX_WALL_SECONDS )); then
        timed_out=1
        kill "$PROCESS_PID" 2>/dev/null || true
        break
    fi
    sleep 0.05
done

set +e
wait "$PROCESS_PID"
process_status=$?
set -e
PROCESS_PID=""
elapsed_ms=$(( $(now_ms) - started_ms ))

if (( timed_out == 1 )); then
    echo "heavy-avif gate exceeded ${MAX_WALL_SECONDS}s" >&2
    exit 1
fi
if (( process_status != 0 )); then
    tail -n 120 "$TMP/process.log" >&2
    exit "$process_status"
fi
if (( peak_rss_kib > MAX_RSS_KIB )); then
    echo "heavy-avif peak RSS ${peak_rss_kib} KiB exceeded ${MAX_RSS_KIB} KiB" >&2
    exit 1
fi

grep -q '"format":"avif"' "$TMP/probe.json"
grep -q '"width":8000' "$TMP/probe.json"
grep -q '"height":8000' "$TMP/probe.json"
[[ "$(vipsheader -f width "$TMP/thumbnail.jpg")" == "1280" ]]
[[ "$(vipsheader -f height "$TMP/thumbnail.jpg")" == "1280" ]]
[[ "$(vipsheader -f bands "$TMP/thumbnail.jpg")" == "3" ]]

mkdir -p "$(dirname "$REPORT")"
generated_at="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
cat >"$REPORT" <<JSON
{
  "schema_version": 1,
  "generated_at": "$generated_at",
  "accepted_fixture": {"format": "avif", "width": 8000, "height": 8000, "pixels": 64000000, "bytes": $fixture_bytes, "sha256": "$fixture_sha256", "generation_ms": $fixture_elapsed_ms},
  "rejected_boundary": {"format": "avif", "width": 16000, "height": 12500, "pixels": 200000000, "bytes": $rejected_bytes, "reason": "decoder_memory_policy"},
  "worker_class": "heavy",
  "native_threads": 1,
  "output": {"format": "jpeg", "width": 1280, "height": 1280, "metadata_policy": "strip"},
  "elapsed_ms": $elapsed_ms,
  "peak_process_tree_rss_kib": $peak_rss_kib,
  "max_process_tree_rss_kib": $MAX_RSS_KIB,
  "result": "pass"
}
JSON

printf 'heavy-avif PASS accepted=8000x8000/64MP rejected=16000x12500/200MP elapsed_ms=%s peak_rss_kib=%s class=heavy\n' \
    "$elapsed_ms" "$peak_rss_kib"
printf 'report=%s\n' "$REPORT"
