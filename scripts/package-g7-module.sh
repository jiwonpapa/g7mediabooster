#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODULE_REL="adapters/gnuboard7/jiwonpapa-g7mediabooster"
MODULE="$ROOT/$MODULE_REL"
FEATURES="$ROOT/deploy/official-features-v1.json"
OUTPUT_DIR="${G7MB_RELEASE_DIR:-$ROOT/output/releases}"

for command_name in cmp git gzip php tar; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "required command is unavailable: $command_name" >&2
        exit 2
    fi
done
if [[ ! -f "$MODULE/module.json" || ! -f "$FEATURES" ]]; then
    echo "module manifest or official feature manifest is missing" >&2
    exit 2
fi
if [[ -n "$(git -C "$ROOT" status --porcelain --untracked-files=all -- "$MODULE_REL")" ]]; then
    echo "G7 module has uncommitted changes; package only a committed tree" >&2
    exit 2
fi

version="$(php -r '
    $data = json_decode(file_get_contents($argv[1]), true, 512, JSON_THROW_ON_ERROR);
    echo $data["version"] ?? "";
' "$MODULE/module.json")"
feature_version="$(php -r '
    $data = json_decode(file_get_contents($argv[1]), true, 512, JSON_THROW_ON_ERROR);
    echo $data["module"]["version"] ?? "";
' "$FEATURES")"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || "$feature_version" != "$version" ]]; then
    echo "module and official feature manifest versions must match semver" >&2
    exit 2
fi
spec_version="$(sed -n 's/^- 스펙 버전: //p' "$ROOT/SPEC.md" | head -n 1)"
php -r '
    $data = json_decode(file_get_contents($argv[1]), true, 512, JSON_THROW_ON_ERROR);
    $fail = static function (string $message): never { throw new RuntimeException($message); };
    ($data["schema_version"] ?? null) === 1 || $fail("official feature schema_version must be 1");
    ($data["spec_version"] ?? null) === $argv[2] || $fail("official feature spec_version drift");
    ($data["release_status"] ?? null) === "candidate" || $fail("external profiles are pending; release must remain candidate");
    $publishable = $data["publishable_features"] ?? null;
    $withheld = $data["withheld_until_verified"] ?? null;
    is_array($publishable) && $publishable !== [] || $fail("publishable feature list is empty");
    is_array($withheld) && $withheld !== [] || $fail("withheld feature list is empty");
    $publishableIds = [];
    foreach ($publishable as $feature) {
        is_string($feature["id"] ?? null) && is_string($feature["title"] ?? null)
            || $fail("publishable feature requires id and title");
        $publishableIds[] = $feature["id"];
    }
    $withheldIds = [];
    foreach ($withheld as $feature) {
        is_string($feature["id"] ?? null) && is_string($feature["reason"] ?? null)
            || $fail("withheld feature requires id and reason");
        $withheldIds[] = $feature["id"];
    }
    count($publishableIds) === count(array_unique($publishableIds)) || $fail("duplicate publishable feature id");
    count($withheldIds) === count(array_unique($withheldIds)) || $fail("duplicate withheld feature id");
    array_intersect($publishableIds, $withheldIds) === [] || $fail("feature cannot be both publishable and withheld");
    foreach (["cloudflare_r2_profile", "lightsail_object_storage_profile", "live_provider_retention_delete", "multi_node_postgresql"] as $required) {
        in_array($required, $withheldIds, true) || $fail("required withheld feature is missing: ".$required);
    }
' "$FEATURES" "$spec_version"

name="jiwonpapa-g7mediabooster-$version"
archive="$OUTPUT_DIR/$name.tar.gz"
checksum="$archive.sha256"
temporary="$archive.tmp"
reproducibility_copy="$archive.repro.tmp"
module_commit="$(git -C "$ROOT" log -1 --format=%H -- "$MODULE_REL")"
commit_time="$(git -C "$ROOT" show -s --format=%cI "$module_commit")"
if [[ -z "$module_commit" || -z "$commit_time" ]]; then
    echo "failed to resolve last module commit and release time" >&2
    exit 2
fi
mkdir -p "$OUTPUT_DIR"
rm -f "$temporary" "$reproducibility_copy"
trap 'rm -f "$temporary" "$reproducibility_copy"' EXIT

release_paths=(
    CHANGELOG.md README.md composer.json composer.lock config database dist
    module.json module.php package.json package-lock.json resources src
    tsconfig.json vite.config.ts
    ':(exclude)resources/**/*.test.ts'
)
build_archive() {
    local destination="$1"
    git -C "$ROOT" archive \
        --format=tar \
        --mtime="$commit_time" \
        --prefix="jiwonpapa-g7mediabooster/" \
        "HEAD:$MODULE_REL" \
        "${release_paths[@]}" \
        | gzip -n -9 >"$destination"
}
build_archive "$temporary"
build_archive "$reproducibility_copy"
if ! cmp -s "$temporary" "$reproducibility_copy"; then
    echo "G7 module archive is not byte-for-byte reproducible" >&2
    exit 1
fi
rm -f "$reproducibility_copy"
mv "$temporary" "$archive"

entries="$(tar -tzf "$archive")"
if [[ "$entries" == *'/vendor/'* \
    || "$entries" == *'/node_modules/'* \
    || "$entries" == *'/tests/'* \
    || "$entries" == *'.test.'* \
    || "$entries" == *'.phpunit'* ]]; then
    echo "release archive contains development-only files" >&2
    exit 1
fi
archive_version="$(tar -xOzf "$archive" jiwonpapa-g7mediabooster/module.json \
    | php -r '$data = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR); echo $data["version"] ?? "";')"
if [[ "$archive_version" != "$version" ]]; then
    echo "release archive module version mismatch" >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha256="$(sha256sum "$archive" | awk '{print $1}')"
else
    sha256="$(shasum -a 256 "$archive" | awk '{print $1}')"
fi
printf '%s  %s\n' "$sha256" "${archive##*/}" >"$checksum"
bytes="$(wc -c <"$archive" | tr -d ' ')"
printf 'g7-module-package PASS version=%s module_commit=%s bytes=%s sha256=%s archive=%s\n' \
    "$version" "$module_commit" "$bytes" "$sha256" "$archive"
