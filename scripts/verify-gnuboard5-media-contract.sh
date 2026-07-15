#!/usr/bin/env bash
set -euo pipefail

root="${1:-${GNUBOARD5_ROOT:-}}"
if [[ -z "$root" || ! -f "$root/common.php" || ! -f "$root/version.php" ]]; then
  echo "usage: $0 /absolute/path/to/gnuboard5" >&2
  exit 64
fi
root="$(cd "$root" && pwd -P)"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
adapter="$repo_root/adapters/gnuboard5/jiwonpapa-g7mediabooster"
failures=0
checks=0

require_pattern() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  checks=$((checks + 1))
  if [[ ! -f "$file" ]] || ! rg -q -- "$pattern" "$file"; then
    echo "FAIL: $label" >&2
    failures=$((failures + 1))
  else
    echo "PASS: $label"
  fi
}

require_literal() {
  local file="$1"
  local literal="$2"
  local label="$3"
  checks=$((checks + 1))
  if [[ ! -f "$file" ]] || ! rg -Fq -- "$literal" "$file"; then
    echo "FAIL: $label" >&2
    failures=$((failures + 1))
  else
    echo "PASS: $label"
  fi
}

require_pattern "$root/version.php" "G5_GNUBOARD_VER', '5\\.6\\.24'" 'pinned Gnuboard 5.6.24 contract'
require_pattern "$root/common.php" 'G5_EXTEND_PATH' 'core-free extend loader'
require_pattern "$root/bbs/write.php" "run_event\\('bbs_write'" 'write form hook'
require_pattern "$root/bbs/write_update.php" "run_event\\('write_update_before'" 'pre-write ownership hook'
require_pattern "$root/bbs/write_update.php" "run_event\\('write_update_after'" 'post-write attachment hook'
require_pattern "$root/lib/common.lib.php" "run_replace\\('get_files'" 'remote attachment presentation filter'
require_pattern "$root/lib/get_data.lib.php" 'function get_board_db' 'board lookup contract'
require_pattern "$root/bbs/download.php" "run_replace\\('download_file_exist_check'" 'remote file existence filter'
require_pattern "$root/bbs/download.php" "run_event\\('download_file_header'" 'authorized download redirect hook'
require_pattern "$root/bbs/write_update.php" "run_replace\\('delete_file_path'" 'attachment deletion scheduling hook'
require_pattern "$root/bbs/write_update.php" 'bf_fileurl' 'remote file URL columns'
require_pattern "$root/bbs/write_update.php" 'bf_thumburl' 'remote thumbnail URL column'
require_pattern "$root/bbs/write_update.php" 'bf_storage' 'remote storage discriminator'
require_pattern "$adapter/extend/g7mediabooster.extend.php" 'Plugin::register' 'adapter extend entrypoint'
require_pattern "$adapter/plugin/g7mediabooster/src/Plugin.php" "add_event\\('write_update_after'" 'adapter attachment registration'
require_literal "$adapter/plugin/g7mediabooster/src/Plugin.php" "static fn (array \$board" 'PHP 8-safe legacy hook closures'
require_pattern "$adapter/plugin/g7mediabooster/src/Plugin.php" "add_replace\\('download_file_exist_check'" 'adapter private delivery registration'
require_literal "$adapter/plugin/g7mediabooster/src/Hooks.php" "explode(',', \$raw)" 'G5 POST escaping-safe upload ID contract'
require_pattern "$adapter/plugin/g7mediabooster/src/ApiEndpoint.php" 'MAX_JSON_BYTES' 'bounded PHP control body'
require_literal "$adapter/plugin/g7mediabooster/src/ApiEndpoint.php" "unset(\$file['original_filename'])" 'local-only filename boundary'
require_pattern "$adapter/plugin/g7mediabooster/assets/uploader.iife.js" 'g7mb_upload_ids' 'built browser form bridge'

if (( failures > 0 )); then
  echo "Gnuboard5 media contract: FAIL ($failures/$checks)" >&2
  exit 1
fi

echo "Gnuboard5 media contract: PASS ($checks/$checks)"
