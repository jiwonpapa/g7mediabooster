#!/usr/bin/env bash
set -euo pipefail

SCRIPT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_ROOT/.." && pwd)"
if [[ -f "$REPO_ROOT/tools/harness/g7mb_harness/__main__.py" ]]; then
    cd "$REPO_ROOT"
    exec python3 -m tools.harness.g7mb_harness verify-g7-contract "$@"
fi
if [[ -f "$SCRIPT_ROOT/g7mb-harness.pyz" ]]; then
    exec python3 "$SCRIPT_ROOT/g7mb-harness.pyz" \
        verify-g7-contract --support-root "$SCRIPT_ROOT" "$@"
fi
echo "G7MediaBooster Python harness is missing" >&2
exit 2
