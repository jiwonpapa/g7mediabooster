"""Watermark policy apply and rollback scenario."""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

from .full_stack_runtime import HMAC_SECRET, FullStackRuntime
from .full_stack_scenarios import assert_ready_media, require_string, upload_single
from .media_protocol import ControlClient
from .process import output, run


def publish_policy(runtime: FullStackRuntime, upload_id: str, revision: int) -> dict[str, Any]:
    """Publish one policy through the real PHP HMAC client."""

    script = (
        runtime.root
        / "adapters/gnuboard7/jiwonpapa-g7mediabooster/tests/Live/publish-site-policy.php"
    )
    rendered = output(
        ["php", str(script)],
        cwd=runtime.root,
        env={
            **runtime.environment,
            "G7MB_POLICY_ENDPOINT": runtime.api_base,
            "G7MB_POLICY_HMAC_SECRET": HMAC_SECRET,
            "G7MB_POLICY_ASSET_UPLOAD_ID": upload_id,
            "G7MB_POLICY_REVISION": str(revision),
        },
    )
    value = json.loads(rendered)
    if not isinstance(value, dict):
        raise RuntimeError("policy client returned invalid JSON")
    return value


def policy_scenario(
    runtime: FullStackRuntime,
    client: ControlClient,
    base_image: Path,
    unwatermarked_master: Path,
) -> dict[str, Any]:
    """Apply and roll back a real watermark policy."""

    watermark = runtime.temp / "watermark.png"
    run(
        [
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x160",
            "-frames:v",
            "1",
            "-threads",
            "1",
            "-y",
            str(watermark),
        ]
    )
    asset_id = upload_single(client, watermark, "watermark-source", "image/png")
    runtime.run_worker("full-stack-watermark-source")
    if client.status(asset_id).get("detected_content_type") != "image/png":
        raise RuntimeError("watermark source is not Ready PNG")
    applied = publish_policy(runtime, asset_id, 1)
    watermark_data = applied.get("watermark")
    if applied.get("revision") != 1 or not isinstance(watermark_data, dict):
        raise RuntimeError("watermark policy was not applied")
    watermark_sha = require_string(watermark_data.get("asset_sha256"), "watermark SHA-256")
    if not re.fullmatch(r"[a-f0-9]{64}", watermark_sha):
        raise RuntimeError("watermark policy SHA-256 is invalid")

    policy_id = upload_single(client, base_image, "policy-enabled", "image/jpeg")
    runtime.run_worker("full-stack-policy")
    policy_status = client.status(policy_id)
    expected = f"board-default-v1-wm-g7-r1-{watermark_sha}"
    presets = {item.get("preset_id") for item in policy_status.get("derivatives", [])}
    if policy_status.get("state") != "ready" or presets != {expected}:
        raise RuntimeError("worker did not pin the applied watermark revision")
    policy_paths = assert_ready_media(runtime, client, policy_id, "image/jpeg", "jpg")
    if unwatermarked_master.read_bytes() == policy_paths["master"].read_bytes():
        raise RuntimeError("watermark policy did not change master bytes")

    rolled_back = publish_policy(runtime, "", 2)
    if rolled_back.get("revision") != 2 or rolled_back.get("watermark") is not None:
        raise RuntimeError("watermark policy rollback failed")
    rollback_id = upload_single(client, base_image, "policy-disabled", "image/jpeg")
    runtime.run_worker("full-stack-rollback")
    rollback_status = client.status(rollback_id)
    presets = {item.get("preset_id") for item in rollback_status.get("derivatives", [])}
    if rollback_status.get("state") != "ready" or presets != {"board-default-v1"}:
        raise RuntimeError("rollback upload did not use the default preset")
    rollback_paths = assert_ready_media(runtime, client, rollback_id, "image/jpeg", "jpg")
    if unwatermarked_master.read_bytes() != rollback_paths["master"].read_bytes():
        raise RuntimeError("policy rollback did not restore deterministic bytes")
    return {"applied_revision": 1, "rollback_revision": 2, "worker_pinned": True}
