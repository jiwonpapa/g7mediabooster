#!/usr/bin/env bash
set -euo pipefail

root="${1:-${GNUBOARD7_ROOT:-}}"
if [[ -z "$root" || ! -d "$root" ]]; then
  echo "usage: $0 /absolute/path/to/gnuboard7" >&2
  exit 64
fi

board="$root/modules/_bundled/sirsoft-board"
template="$root/templates/_bundled/sirsoft-basic/layouts/partials/board/form/_post_form.json"
failures=0

require_pattern() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  if [[ ! -f "$file" ]] || ! rg -q -- "$pattern" "$file"; then
    echo "FAIL: $label" >&2
    failures=$((failures + 1))
  else
    echo "PASS: $label"
  fi
}

require_pattern "$board/src/Repositories/AttachmentRepository.php" \
  'linkAttachmentsByIds\(string \$slug, array \$ids, int \$postId, int \$ownerId\)' \
  'owner-scoped attachment repository signature'
require_pattern "$board/src/Repositories/AttachmentRepository.php" \
  'where\('"'"'created_by'"'"', \$ownerId\)' \
  'created_by owner predicate'
require_pattern "$board/src/Services/PostService.php" \
  '\$linkedCount !== count\(\$normalizedIds\)' \
  'all-or-nothing attachment link check'
require_pattern "$board/src/Http/Controllers/User/PostController.php" \
  'attachmentIds: \$attachmentIds' \
  'user create passes attachment IDs'
require_pattern "$board/src/Http/Controllers/User/PostController.php" \
  'updatePost\(\$slug, \$id, \$data, \$attachmentIds\)' \
  'user update passes attachment IDs'
require_pattern "$board/src/Services/AttachmentService.php" \
  'public function authorizeDelivery\(' \
  'byte-free attachment delivery authorization'
require_pattern "$board/src/Models/Attachment.php" \
  'sirsoft-board\.attachment\.filter_download_url' \
  'download URL filter'
require_pattern "$board/src/Models/Attachment.php" \
  'sirsoft-board\.attachment\.filter_preview_url' \
  'preview URL filter'
require_pattern "$template" '"id": "board_native_attachment_section"' \
  'user attachment layout target'
require_pattern "$template" '"id": "board_post_submit"' \
  'user submit layout target'

if (( failures > 0 )); then
  echo "Gnuboard7 media contract: FAIL ($failures missing)" >&2
  exit 1
fi

echo "Gnuboard7 media contract: PASS"
