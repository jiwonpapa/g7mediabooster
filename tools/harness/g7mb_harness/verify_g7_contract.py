"""Verify the G7 source contract and, optionally, the DB-resolved runtime layout."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from .g7_contract_support import patched_paths, verify_module_host, verify_packaged_module
from .process import output, require_programs, run


@dataclass(frozen=True)
class PatternRequirement:
    """One source-level compatibility requirement."""

    relative: str
    pattern: str
    label: str
    minimum: int = 1


REQUIREMENTS = (
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Repositories/AttachmentRepository.php",
        r"linkAttachmentsByIds\(string \$slug, array \$ids, int \$postId, int \$ownerId\)",
        "owner-scoped attachment repository signature",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Repositories/AttachmentRepository.php",
        r"where\('created_by', \$ownerId\)",
        "created_by owner predicate",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Services/PostService.php",
        r"\$linkedCount !== count\(\$normalizedIds\)",
        "all-or-nothing attachment link check",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Services/PostService.php",
        r"recalculateAttachmentsCount\(\$postId\)",
        "bulk attachment link synchronizes post count",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Http/Controllers/User/PostController.php",
        r"attachmentIds: \$attachmentIds",
        "user create passes attachment IDs",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Http/Controllers/User/PostController.php",
        r"updatePost\(\$slug, \$id, \$data, \$attachmentIds\)",
        "user update passes attachment IDs",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Http/Requests/StorePostRequest.php",
        r"'attachment_ids'.*'list'.*'max:'",
        "admin create bounds attachment ID list",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Http/Requests/UpdatePostRequest.php",
        r"'list',",
        "admin update requires attachment ID list",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Http/Requests/User/StorePostRequest.php",
        r"'attachment_ids'.*'list'.*'max:'",
        "user create bounds attachment ID list",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Http/Requests/User/StorePostRequest.php",
        r"'attachment_ids\.\*'.*'distinct:strict'",
        "user create rejects duplicate attachment IDs",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Http/Requests/User/UpdatePostRequest.php",
        r"max_file_count",
        "user update bounds attachment ID list",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Http/Requests/User/UpdatePostRequest.php",
        r"'attachment_ids\.\*'.*'distinct:strict'",
        "user update rejects duplicate attachment IDs",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Services/AttachmentService.php",
        r"public function authorizeDelivery\(",
        "byte-free attachment delivery authorization",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Repositories/Contracts/AttachmentRepositoryInterface.php",
        r"findPostForAttachmentDelivery\(string \$slug, int \$postId\)",
        "visibility-aware attachment repository contract",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Repositories/AttachmentRepository.php",
        r"Post::withTrashed\(\)",
        "deleted post metadata remains available to delivery guard",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Services/AttachmentService.php",
        r"PostStatus::Blinded",
        "blinded post attachment guard",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Services/AttachmentService.php",
        r"posts\.read-secret",
        "secret post attachment permission guard",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Services/AttachmentService.php",
        r"assertPostAttachmentAccess\(\$slug, \$attachment, 'user'\)",
        "native preview paths reuse post visibility guard",
        2,
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Models/Attachment.php",
        r"sirsoft-board\.attachment\.filter_download_url",
        "download URL filter",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Models/Attachment.php",
        r"sirsoft-board\.attachment\.filter_preview_url",
        "preview URL filter",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Models/Attachment.php",
        r"\$defaultUrl = \$this->is_image",
        "video poster filter fallback",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/src/Models/Attachment.php",
        r"\$boardSlug,",
        "URL filter receives validated board slug",
    ),
    PatternRequirement(
        "templates/_bundled/sirsoft-basic/layouts/partials/board/form/_post_form.json",
        r'"id": "board_native_attachment_section"',
        "user attachment layout target",
    ),
    PatternRequirement(
        "templates/_bundled/sirsoft-basic/layouts/partials/board/form/_post_form.json",
        r'"id": "board_native_file_uploader"',
        "user attachment uploader layout target",
    ),
    PatternRequirement(
        "templates/_bundled/sirsoft-basic/layouts/partials/board/form/_post_form.json",
        r'"id": "board_post_submit"',
        "user submit layout target",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/resources/layouts/admin/partials/"
        "admin_board_post_form/_attachments.json",
        r'"id": "admin_board_native_file_uploader"',
        "admin attachment uploader layout target",
    ),
    PatternRequirement(
        "modules/_bundled/sirsoft-board/resources/layouts/admin/admin_board_post_form.json",
        r'"id": "footer_save_button"',
        "admin submit layout target",
    ),
    PatternRequirement(
        "app/Services/LayoutExtensionService.php",
        r"return \$injected;",
        "layout overlay applies to every matching target",
    ),
)


def repository_root() -> Path:
    """Return the current source repository root."""

    return Path(__file__).resolve().parents[3]


def verify_source(root: Path, repo: Path, support_root: Path | None = None) -> list[str]:
    """Verify source patterns plus parser validity for every patched file."""

    failures: list[str] = []
    for requirement in REQUIREMENTS:
        path = root / requirement.relative
        count = 0
        if path.is_file():
            count = len(re.findall(requirement.pattern, path.read_text(encoding="utf-8")))
        if count < requirement.minimum:
            failures.append(requirement.label)
            print(f"FAIL: {requirement.label}", file=sys.stderr)
        else:
            print(f"PASS: {requirement.label}")

    require_programs(["php"])
    patch_root = (
        support_root
        if support_root is not None
        else repo / "adapters" / "gnuboard7" / "upstream-contract"
    )
    if not patch_root.is_dir():
        failures.append("upstream patch directory is missing")
        return failures
    for relative in patched_paths(patch_root):
        path = root / relative
        if not path.is_file():
            failures.append(f"patched file missing: {relative}")
            continue
        if path.suffix == ".php":
            completed = run(["php", "-l", str(path)], capture=True, check=False)
            if completed.returncode != 0:
                failures.append(f"invalid PHP syntax: {relative}")
        elif path.suffix == ".json":
            try:
                json.loads(path.read_text(encoding="utf-8"))
            except json.JSONDecodeError:
                failures.append(f"invalid JSON syntax: {relative}")
    if not any("syntax" in failure or "missing:" in failure for failure in failures):
        print("PASS: patched PHP/JSON parser validation")

    if support_root is None:
        host_verifier = repo / "scripts" / "verify-gnuboard7-module-host.php"
        module_root = repo / "adapters" / "gnuboard7" / "jiwonpapa-g7mediabooster"
        if not verify_module_host(root, host_verifier, module_root):
            failures.append("module activation contract")
    else:
        if not verify_packaged_module(root, support_root):
            failures.append("module activation contract")
    return failures


def runtime_layout_status(root: Path) -> dict[str, int]:
    """Resolve the installed layout from the database instead of reading source JSON."""

    artisan = root / "artisan"
    if not artisan.is_file():
        raise RuntimeError(f"artisan is missing: {artisan}")
    php = (
        "$service = app(\\App\\Services\\LayoutService::class); "
        '$user = json_encode($service->getLayout("sirsoft-basic", "board/form", true), '
        "JSON_UNESCAPED_SLASHES); "
        '$admin = json_encode($service->getLayout("sirsoft-admin_basic", '
        '"sirsoft-board.admin_board_post_form", true), JSON_UNESCAPED_SLASHES); '
        'echo json_encode(["user_mount"=>substr_count($user, "g7mb-user-uploader-mount"),'
        '"user_handler"=>substr_count($user, "jiwonpapa-g7mediabooster.mountUploader"),'
        '"admin_mount"=>substr_count($admin, "g7mb-admin-uploader-mount"),'
        '"admin_handler"=>substr_count($admin, "jiwonpapa-g7mediabooster.mountUploader")]);'
    )
    rendered = output(["php", str(artisan), "tinker", f"--execute={php}"], cwd=root)
    try:
        data = json.loads(rendered.splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        raise RuntimeError(f"G7 runtime layout evidence is invalid: {rendered}") from error
    result = {key: int(value) for key, value in data.items()}
    required = ("user_mount", "user_handler", "admin_mount", "admin_handler")
    if any(result.get(key, 0) < 1 for key in required):
        raise RuntimeError(f"G7 runtime layout overlay is not applied: {result}")
    return result


def add_arguments(parser: argparse.ArgumentParser) -> None:
    """Register command arguments."""

    parser.add_argument("root", type=Path)
    parser.add_argument("--runtime", action="store_true")
    parser.add_argument("--support-root", type=Path)


def main(args: argparse.Namespace) -> int:
    """Run source and optional runtime verification."""

    root = args.root.resolve()
    try:
        support_root = args.support_root.resolve() if args.support_root else None
        failures = verify_source(root, repository_root(), support_root)
        if failures:
            print(f"Gnuboard7 media contract: FAIL ({len(failures)} missing)", file=sys.stderr)
            return 1
        if args.runtime:
            status = runtime_layout_status(root)
            print(f"PASS: DB-resolved layout overlay {json.dumps(status, sort_keys=True)}")
    except (RuntimeError, subprocess.SubprocessError) as error:
        print(f"Gnuboard7 media contract: FAIL ({error})", file=sys.stderr)
        return 1
    print(f"Gnuboard7 media contract: PASS ({len(REQUIREMENTS)}/28 + parser/activation)")
    return 0
