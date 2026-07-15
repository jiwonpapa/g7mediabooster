#!/usr/bin/env bash
set -euo pipefail

root="${1:-${GNUBOARD7_ROOT:-}}"
if [[ -z "$root" || ! -f "$root/artisan" ]]; then
  echo "usage: $0 /absolute/path/to/patched-gnuboard7" >&2
  exit 64
fi
root="$(cd "$root" && pwd -P)"

module=""
for candidate in \
  "$root/modules/jiwonpapa-g7mediabooster" \
  "$root/modules/_bundled/jiwonpapa-g7mediabooster"; do
  if [[ -f "$candidate/tests/Host/AttachmentRetentionHostTest.php" ]]; then
    module="$candidate"
    break
  fi
done
if [[ -z "$module" ]]; then
  echo "G7MediaBooster module with host tests is not installed" >&2
  exit 66
fi

php_bin="${PHP_BIN:-php}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
module_test="${module#"$root"/}/tests/Host/AttachmentRetentionHostTest.php"

"$repo_root/scripts/verify-gnuboard7-media-contract.sh" "$root"
(
  cd "$root"
  "$php_bin" artisan test \
    modules/_bundled/sirsoft-board/tests/Unit/AttachmentServiceTest.php \
    --filter='authorize_delivery|get_file_info_denies'
  "$php_bin" artisan test "$module_test"
)

echo "Gnuboard7 host security gate: PASS"
