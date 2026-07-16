#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/g7mb-setup.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
umask 077

printf '%s' 'offline-access-id' >"$TMP/input-access"
printf '%s' 'offline-secret-key' >"$TMP/input-secret"

cargo build --quiet --locked --package g7mbctl

CONFIG="$TMP/etc/g7mediabooster/g7mb.toml"
SECRETS="$TMP/etc/g7mediabooster/credentials"
SETUP=(
    "$ROOT/target/debug/g7mbctl" setup
    --non-interactive
    --provider r2
    --account-id 0123456789abcdef0123456789abcdef
    --bucket g7mb-smoke-private
    --origin https://example.com
    --access-key-id-file "$TMP/input-access"
    --secret-access-key-file "$TMP/input-secret"
    --config "$CONFIG"
    --secrets-dir "$SECRETS"
    --defer-storage
    --skip-ownership
)

"${SETUP[@]}" >"$TMP/first.log"
HMAC_BEFORE="$(shasum -a 256 "$SECRETS/g7-hmac-secret" | awk '{print $1}')"
"${SETUP[@]}" >"$TMP/second.log"
HMAC_AFTER="$(shasum -a 256 "$SECRETS/g7-hmac-secret" | awk '{print $1}')"

test "$HMAC_BEFORE" = "$HMAC_AFTER"
rg -q '^provider = "r2"$' "$CONFIG"
rg -q '^hmac_secret_file = ' "$CONFIG"
rg -q '^access_key_id_file = ' "$CONFIG"
rg -q '^secret_access_key_file = ' "$CONFIG"
if rg -q '^(hmac_secret|access_key_id|secret_access_key) = ' "$CONFIG"; then
    echo "setup smoke failed: inline secret was written to TOML" >&2
    exit 1
fi

mode_of() {
    if stat -f '%Lp' "$1" >/dev/null 2>&1; then
        stat -f '%Lp' "$1"
    else
        stat -c '%a' "$1"
    fi
}

test "$(mode_of "$SECRETS")" = "700"
test "$(mode_of "$SECRETS/storage-access-key-id")" = "600"
test "$(mode_of "$SECRETS/storage-secret-access-key")" = "600"
test "$(mode_of "$SECRETS/g7-hmac-secret")" = "600"
test "$(mode_of "$CONFIG")" = "640"

for unit in "$ROOT"/deploy/systemd/*.service; do
    if rg -q 'ExecStart=.*g7mb-(api|worker)' "$unit"; then
        test "$(rg -c '^LoadCredential=g7mb-' "$unit")" = "3"
        test "$(rg -c '^Environment=G7MB__.*_FILE=%d/' "$unit")" = "3"
    fi
done

printf '%s' 'changed-secret-key' >"$TMP/changed-secret"
CHANGED_SETUP=("${SETUP[@]}")
for index in "${!CHANGED_SETUP[@]}"; do
    if [[ "${CHANGED_SETUP[$index]}" == "$TMP/input-secret" ]]; then
        CHANGED_SETUP[$index]="$TMP/changed-secret"
    fi
done
if "${CHANGED_SETUP[@]}" >"$TMP/conflict.log" 2>&1; then
    echo "setup smoke failed: changed credential replaced without --force" >&2
    exit 1
fi
test "$HMAC_BEFORE" = "$(shasum -a 256 "$SECRETS/g7-hmac-secret" | awk '{print $1}')"

echo "g7mbctl offline setup smoke PASS"
