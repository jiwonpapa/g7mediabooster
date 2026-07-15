#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ADAPTER="$ROOT/adapters/gnuboard7/jiwonpapa-g7mediabooster"

command -v composer >/dev/null
command -v php >/dev/null

composer install --working-dir="$ADAPTER" --no-interaction --no-progress >/dev/null
G7MB_FULL_STACK_POLICY_SMOKE=true bash "$ROOT/scripts/full-stack-smoke.sh"
