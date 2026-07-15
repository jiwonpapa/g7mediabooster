#!/usr/bin/env bash
set -euo pipefail

root="${1:-${GNUBOARD7_ROOT:-}}"
if [[ -z "$root" || ! -d "$root" ]]; then
  echo "usage: $0 /absolute/path/to/gnuboard7" >&2
  exit 64
fi

script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
patch_root="$script_root/../adapters/gnuboard7/upstream-contract"
board="$root/modules/_bundled/sirsoft-board"
template="$root/templates/_bundled/sirsoft-basic/layouts/partials/board/form/_post_form.json"
admin_attachments="$board/resources/layouts/admin/partials/admin_board_post_form/_attachments.json"
admin_form="$board/resources/layouts/admin/admin_board_post_form.json"
user_store_request="$board/src/Http/Requests/User/StorePostRequest.php"
user_update_request="$board/src/Http/Requests/User/UpdatePostRequest.php"
layout_extension_service="$root/app/Services/LayoutExtensionService.php"
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

require_pattern_count_at_least() {
  local file="$1"
  local pattern="$2"
  local minimum="$3"
  local label="$4"
  local count=0
  if [[ -f "$file" ]]; then
    count="$(rg -c -- "$pattern" "$file" || true)"
  fi
  if [[ ! "$count" =~ ^[0-9]+$ ]] || (( count < minimum )); then
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
require_pattern "$board/src/Services/PostService.php" \
  'recalculateAttachmentsCount\(\$postId\)' \
  'bulk attachment link synchronizes post count'
require_pattern "$board/src/Http/Controllers/User/PostController.php" \
  'attachmentIds: \$attachmentIds' \
  'user create passes attachment IDs'
require_pattern "$board/src/Http/Controllers/User/PostController.php" \
  'updatePost\(\$slug, \$id, \$data, \$attachmentIds\)' \
  'user update passes attachment IDs'
require_pattern "$board/src/Http/Requests/StorePostRequest.php" \
  "'attachment_ids'.*'list'.*'max:'" \
  'admin create bounds attachment ID list'
require_pattern "$board/src/Http/Requests/UpdatePostRequest.php" \
  "'list'," \
  'admin update requires attachment ID list'
require_pattern "$user_store_request" \
  "'attachment_ids'.*'list'.*'max:'" \
  'user create bounds attachment ID list'
require_pattern "$user_store_request" \
  "'attachment_ids\.\*'.*'distinct:strict'" \
  'user create rejects duplicate attachment IDs'
require_pattern "$user_update_request" \
  'max_file_count' \
  'user update bounds attachment ID list'
require_pattern "$user_update_request" \
  "'attachment_ids\.\*'.*'distinct:strict'" \
  'user update rejects duplicate attachment IDs'
require_pattern "$board/src/Services/AttachmentService.php" \
  'public function authorizeDelivery\(' \
  'byte-free attachment delivery authorization'
require_pattern "$board/src/Repositories/Contracts/AttachmentRepositoryInterface.php" \
  'findPostForAttachmentDelivery\(string \$slug, int \$postId\)' \
  'visibility-aware attachment repository contract'
require_pattern "$board/src/Repositories/AttachmentRepository.php" \
  'Post::withTrashed\(\)' \
  'deleted post metadata remains available to delivery guard'
require_pattern "$board/src/Services/AttachmentService.php" \
  'PostStatus::Blinded' \
  'blinded post attachment guard'
require_pattern "$board/src/Services/AttachmentService.php" \
  'posts\.read-secret' \
  'secret post attachment permission guard'
require_pattern_count_at_least "$board/src/Services/AttachmentService.php" \
  "assertPostAttachmentAccess\\(\\\$slug, \\\$attachment, 'user'\\)" \
  2 \
  'native preview paths reuse post visibility guard'
require_pattern "$board/src/Models/Attachment.php" \
  'sirsoft-board\.attachment\.filter_download_url' \
  'download URL filter'
require_pattern "$board/src/Models/Attachment.php" \
  'sirsoft-board\.attachment\.filter_preview_url' \
  'preview URL filter'
require_pattern "$board/src/Models/Attachment.php" \
  '\$defaultUrl = \$this->is_image' \
  'video poster filter fallback'
require_pattern "$board/src/Models/Attachment.php" \
  '\$boardSlug,' \
  'URL filter receives validated board slug'
require_pattern "$template" '"id": "board_native_attachment_section"' \
  'user attachment layout target'
require_pattern "$template" '"id": "board_native_file_uploader"' \
  'user attachment uploader layout target'
require_pattern "$template" '"id": "board_post_submit"' \
  'user submit layout target'
require_pattern "$admin_attachments" '"id": "admin_board_native_file_uploader"' \
  'admin attachment uploader layout target'
require_pattern "$admin_form" '"id": "footer_save_button"' \
  'admin submit layout target'
require_pattern "$layout_extension_service" \
  'return \$injected;' \
  'layout overlay applies to every matching target'

# 패턴 일치만으로 문법이 깨진 overlay를 PASS 처리하지 않습니다. 공식 patch가 만지는
# PHP/JSON 전체를 실제 parser로 검사하되 credential이나 DB 연결은 요구하지 않습니다.
if ! command -v php >/dev/null 2>&1; then
  echo "FAIL: php CLI is required for upstream contract syntax validation" >&2
  failures=$((failures + 1))
elif [[ ! -d "$patch_root" ]]; then
  echo "FAIL: upstream patch directory is missing" >&2
  failures=$((failures + 1))
else
  syntax_failures=0
  while IFS= read -r relative_path; do
    file="$root/$relative_path"
    if [[ ! -f "$file" ]]; then
      echo "FAIL: patched file is missing: $relative_path" >&2
      syntax_failures=$((syntax_failures + 1))
      continue
    fi
    case "$relative_path" in
      *.php)
        if ! php -l "$file" >/dev/null; then
          echo "FAIL: invalid PHP syntax: $relative_path" >&2
          syntax_failures=$((syntax_failures + 1))
        fi
        ;;
      *.json)
        if ! php -r \
          'json_decode(file_get_contents($argv[1]), true, 512, JSON_THROW_ON_ERROR);' \
          "$file"; then
          echo "FAIL: invalid JSON syntax: $relative_path" >&2
          syntax_failures=$((syntax_failures + 1))
        fi
        ;;
    esac
  done < <(
    rg --no-filename '^\+\+\+ b/' "$patch_root"/*.patch \
      | sed 's#^+++ b/##' \
      | sort -u
  )
  if (( syntax_failures > 0 )); then
    failures=$((failures + syntax_failures))
  else
    echo "PASS: patched PHP/JSON parser validation"
  fi
fi

if (( failures > 0 )); then
  echo "Gnuboard7 media contract: FAIL ($failures missing)" >&2
  exit 1
fi

echo "Gnuboard7 media contract: PASS (28/28 + parser validation)"
