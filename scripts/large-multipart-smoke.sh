#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export G7MB_FULL_STACK_LARGE_MULTIPART_BYTES=5368709120

exec bash "$ROOT/scripts/full-stack-smoke.sh"
