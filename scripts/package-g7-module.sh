#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODULE_REL="adapters/gnuboard7/jiwonpapa-g7mediabooster"
MODULE="$ROOT/$MODULE_REL"
FEATURES="$ROOT/deploy/official-features-v1.json"
OUTPUT_DIR="${G7MB_RELEASE_DIR:-$ROOT/output/releases}"

for command_name in cmp find git gzip php sort tar unzip zip; do
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
tar_archive="$OUTPUT_DIR/$name.tar.gz"
zip_archive="$OUTPUT_DIR/$name.zip"
tar_temporary="$tar_archive.tmp"
zip_temporary="$zip_archive.tmp"
tar_reproducibility_copy="$tar_archive.repro.tmp"
zip_reproducibility_copy="$zip_archive.repro.tmp"
zip_source_one=""
zip_source_two=""
module_commit="$(git -C "$ROOT" log -1 --format=%H -- "$MODULE_REL")"
commit_time="$(git -C "$ROOT" show -s --format=%cI "$module_commit")"
if [[ -z "$module_commit" || -z "$commit_time" ]]; then
    echo "failed to resolve last module commit and release time" >&2
    exit 2
fi
mkdir -p "$OUTPUT_DIR"
rm -f \
    "$tar_temporary" "$zip_temporary" \
    "$tar_reproducibility_copy" "$zip_reproducibility_copy"
cleanup() {
    rm -f \
        "$tar_temporary" "$zip_temporary" \
        "$tar_reproducibility_copy" "$zip_reproducibility_copy"
    if [[ -n "$zip_source_one" ]]; then
        rm -rf -- "$zip_source_one"
    fi
    if [[ -n "$zip_source_two" ]]; then
        rm -rf -- "$zip_source_two"
    fi
}
trap cleanup EXIT
zip_source_one="$(mktemp -d "$OUTPUT_DIR/.g7mb-zip-source-one.XXXXXX")"
zip_source_two="$(mktemp -d "$OUTPUT_DIR/.g7mb-zip-source-two.XXXXXX")"

release_paths=(
    CHANGELOG.md README.md composer.json composer.lock config database dist
    module.json module.php package.json package-lock.json resources src
    tsconfig.json vite.config.ts
    ':(exclude)resources/**/*.test.ts'
)
build_tar_archive() {
    local destination="$1"
    git -C "$ROOT" archive \
        --format=tar \
        --mtime="$commit_time" \
        --prefix="jiwonpapa-g7mediabooster/" \
        "HEAD:$MODULE_REL" \
        "${release_paths[@]}" \
        | gzip -n -9 >"$destination"
}
build_zip_archive() {
    local destination="$1"
    local source_directory="$2"
    git -C "$ROOT" archive \
        --format=tar \
        --mtime="$commit_time" \
        --prefix="jiwonpapa-g7mediabooster/" \
        "HEAD:$MODULE_REL" \
        "${release_paths[@]}" \
        | tar -xf - -C "$source_directory"
    (
        cd "$source_directory"
        find jiwonpapa-g7mediabooster -type f -print \
            | LC_ALL=C sort \
            | zip -X -q "$destination" -@
    )
}
build_tar_archive "$tar_temporary"
build_tar_archive "$tar_reproducibility_copy"
build_zip_archive "$zip_temporary" "$zip_source_one"
build_zip_archive "$zip_reproducibility_copy" "$zip_source_two"
if ! cmp -s "$tar_temporary" "$tar_reproducibility_copy"; then
    echo "G7 module tar.gz is not byte-for-byte reproducible" >&2
    exit 1
fi
if ! cmp -s "$zip_temporary" "$zip_reproducibility_copy"; then
    echo "G7 module ZIP is not byte-for-byte reproducible" >&2
    exit 1
fi
rm -f "$tar_reproducibility_copy" "$zip_reproducibility_copy"
mv "$tar_temporary" "$tar_archive"
mv "$zip_temporary" "$zip_archive"

validate_entries() {
    local entries="$1"
    local label="$2"
    if [[ "$entries" == *'/vendor/'* \
        || "$entries" == *'/node_modules/'* \
        || "$entries" == *'/tests/'* \
        || "$entries" == *'.test.'* \
        || "$entries" == *'.phpunit'* ]]; then
        echo "$label contains development-only files" >&2
        exit 1
    fi
}
validate_entries "$(tar -tzf "$tar_archive")" "release tar.gz"
validate_entries "$(unzip -Z1 "$zip_archive")" "release ZIP"

tar_version="$(tar -xOzf "$tar_archive" jiwonpapa-g7mediabooster/module.json \
    | php -r '$data = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR); echo $data["version"] ?? "";')"
zip_version="$(unzip -p "$zip_archive" jiwonpapa-g7mediabooster/module.json \
    | php -r '$data = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR); echo $data["version"] ?? "";')"
if [[ "$tar_version" != "$version" || "$zip_version" != "$version" ]]; then
    echo "release archive module version mismatch" >&2
    exit 1
fi

sha256_file() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print $1}'
    else
        shasum -a 256 "$file" | awk '{print $1}'
    fi
}
write_checksum() {
    local file="$1"
    local digest="$2"
    printf '%s  %s\n' "$digest" "${file##*/}" >"$file.sha256"
}
tar_sha256="$(sha256_file "$tar_archive")"
zip_sha256="$(sha256_file "$zip_archive")"
write_checksum "$tar_archive" "$tar_sha256"
write_checksum "$zip_archive" "$zip_sha256"
tar_bytes="$(wc -c <"$tar_archive" | tr -d ' ')"
zip_bytes="$(wc -c <"$zip_archive" | tr -d ' ')"
if [[ -n "${GNUBOARD7_ROOT:-}" ]]; then
    php "$ROOT/scripts/verify-gnuboard7-module-zip.php" \
        "$GNUBOARD7_ROOT" "$zip_archive" "$version"
fi
printf 'g7-module-package PASS version=%s module_commit=%s tar_bytes=%s tar_sha256=%s tar=%s zip_bytes=%s zip_sha256=%s zip=%s\n' \
    "$version" "$module_commit" "$tar_bytes" "$tar_sha256" "$tar_archive" \
    "$zip_bytes" "$zip_sha256" "$zip_archive"
