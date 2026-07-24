#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${G7MB_RELEASE_DIR:-$ROOT/output/releases}"

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "server bundle must be built on Linux" >&2
    exit 2
fi
for command_name in cargo cmp find git gzip install jq python3 sha256sum tar; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "required command is unavailable: $command_name" >&2
        exit 2
    fi
done
if [[ -n "$(git -C "$ROOT" status --porcelain --untracked-files=all)" \
    && "${G7MB_ALLOW_DIRTY:-0}" != "1" ]]; then
    echo "server bundle requires a clean committed tree" >&2
    exit 2
fi

version="$(awk '
    $0 == "[workspace.package]" { package = 1; next }
    package && /^\[/ { exit }
    package && /^version = "/ {
        value = $0
        sub(/^version = "/, "", value)
        sub(/"$/, "", value)
        print value
        exit
    }
' "$ROOT/Cargo.toml")"
module_version="$(jq -er '.version' \
    "$ROOT/adapters/gnuboard7/jiwonpapa-g7mediabooster/module.json")"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ \
    || ! "$module_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "workspace or module version is invalid" >&2
    exit 2
fi

case "$(uname -m)" in
    x86_64) architecture="x86_64" ;;
    aarch64 | arm64) architecture="aarch64" ;;
    *)
        echo "unsupported server architecture: $(uname -m)" >&2
        exit 2
        ;;
esac

cd "$ROOT"
cargo xtask g7-module-package
cargo build --release --locked \
    --package g7mbctl \
    --package g7mb-api \
    --package g7mb-worker
cargo build --release --locked --package g7mb-sandbox --features native-vips
target/release/g7mb-sandbox doctor
target/release/g7mb-sandbox capabilities >/dev/null

mkdir -p "$OUTPUT_DIR"
name="g7mediabooster-server-$version-linux-$architecture"
archive="$OUTPUT_DIR/$name.tar.gz"
temporary_one="$archive.one.tmp"
temporary_two="$archive.two.tmp"
stage="$(mktemp -d "$OUTPUT_DIR/.g7mb-server-stage.XXXXXX")"
bundle="$stage/$name"
cleanup() {
    rm -rf -- "$stage"
    rm -f -- "$temporary_one" "$temporary_two"
}
trap cleanup EXIT
install -d -m 0755 \
    "$bundle/bin" \
    "$bundle/libexec" \
    "$bundle/systemd" "$bundle/nginx" \
    "$bundle/gnuboard7"
install -m 0755 target/release/g7mbctl "$bundle/bin/g7mbctl"
install -m 0755 target/release/g7mb-api "$bundle/bin/g7mb-api"
install -m 0755 target/release/g7mb-worker "$bundle/bin/g7mb-worker"
install -m 0755 target/release/g7mb-sandbox "$bundle/libexec/g7mb-sandbox"
install -m 0644 deploy/systemd/* "$bundle/systemd/"
install -m 0644 deploy/nginx/g7mediabooster-public.conf "$bundle/nginx/"
install -m 0644 docs/SERVER_INSTALL.md "$bundle/INSTALL.md"
install -m 0644 deploy/official-features-v1.json "$bundle/gnuboard7/official-features-v1.json"
install -m 0755 scripts/verify-gnuboard7-media-contract.sh \
    "$bundle/gnuboard7/verify-gnuboard7-media-contract.sh"
python3 -m tools.harness.g7mb_harness package-zipapp \
    "$bundle/gnuboard7/g7mb-harness.pyz"
install -m 0644 scripts/verify-gnuboard7-module-host.php \
    "$bundle/gnuboard7/verify-gnuboard7-module-host.php"
install -m 0644 \
    "output/releases/jiwonpapa-g7mediabooster-$module_version.zip" \
    "$bundle/gnuboard7/jiwonpapa-g7mediabooster.zip"
(
    cd "$bundle/gnuboard7"
    sha256sum jiwonpapa-g7mediabooster.zip >jiwonpapa-g7mediabooster.zip.sha256
)

patch_index=1
for patch in adapters/gnuboard7/upstream-contract/*.patch; do
    install -m 0644 "$patch" "$bundle/gnuboard7/$(printf '%04d.patch' "$patch_index")"
    (( patch_index += 1 ))
done
if [[ "$patch_index" -ne 7 ]]; then
    echo "expected exactly six Gnuboard7 contract patches" >&2
    exit 2
fi
printf '%s\n' "$version" >"$bundle/VERSION"
(
    cd "$bundle"
    find bin libexec systemd nginx gnuboard7 -type f -print0 \
        | LC_ALL=C sort -z \
        | xargs -0 sha256sum >MANIFEST.sha256
    sha256sum INSTALL.md VERSION >>MANIFEST.sha256
    LC_ALL=C sort -o MANIFEST.sha256 MANIFEST.sha256
)

source_date_epoch="$(git show -s --format=%ct HEAD)"
build_archive() {
    local destination="$1"
    tar \
        --sort=name \
        --mtime="@$source_date_epoch" \
        --owner=0 \
        --group=0 \
        --numeric-owner \
        -C "$stage" \
        -cf - "$name" \
        | gzip -n -9 >"$destination"
}
build_archive "$temporary_one"
build_archive "$temporary_two"
if ! cmp -s "$temporary_one" "$temporary_two"; then
    echo "server bundle is not byte-for-byte reproducible" >&2
    exit 1
fi
if tar -tzf "$temporary_one" | grep -E '(^/|(^|/)\.\.(/|$))' >/dev/null; then
    echo "server bundle contains an unsafe archive path" >&2
    exit 1
fi
mv "$temporary_one" "$archive"
rm -f "$temporary_two"
(
    cd "$OUTPUT_DIR"
    sha256sum "${archive##*/}" >"${archive##*/}.sha256"
)

bytes="$(wc -c <"$archive" | tr -d ' ')"
digest="$(sha256sum "$archive" | awk '{print $1}')"
printf 'server-package PASS version=%s module=%s architecture=%s bytes=%s sha256=%s archive=%s\n' \
    "$version" "$module_version" "$architecture" "$bytes" "$digest" "$archive"
