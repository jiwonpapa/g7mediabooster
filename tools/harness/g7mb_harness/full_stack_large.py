"""Sparse large multipart scenario with API RSS and restart checks."""

from __future__ import annotations

import sys
from typing import Any

from .full_stack_runtime import FullStackRuntime
from .full_stack_scenarios import require_string
from .media_protocol import ControlClient, direct_put


def large_multipart_scenario(
    runtime: FullStackRuntime,
    client: ControlClient,
    total_bytes: int,
) -> dict[str, Any]:
    """Upload a sparse large object while proving the API does not proxy its body."""

    instruction = client.upload_batch(
        [
            {
                "client_ref": "large-video",
                "declared_kind": "video",
                "content_length": total_bytes,
                "content_type_hint": "video/mp4",
            }
        ]
    )[0]
    if instruction.get("method") != "multipart":
        raise RuntimeError("large object did not select multipart")
    upload_id = require_string(instruction.get("upload_id"), "large upload_id")
    part_size = int(instruction.get("part_size_bytes", 0))
    part_count = (total_bytes + part_size - 1) // part_size
    if not 1 <= part_count <= 10_000:
        raise RuntimeError(f"large multipart count is invalid: {part_count}")
    sparse = runtime.temp / "large-part.bin"
    rss_start = runtime.rss_kib()
    rss_peak = rss_start
    restart_at = max(1, part_count // 2)
    restarts = 0
    parts: list[dict[str, Any]] = []
    for part_number in range(1, part_count + 1):
        offset = (part_number - 1) * part_size
        length = min(part_size, total_bytes - offset)
        with sparse.open("wb") as handle:
            handle.truncate(length)
        presigned = client.presign_part(upload_id, part_number, length)
        etag = direct_put(presigned, sparse)
        if not etag:
            raise RuntimeError(f"large multipart PUT returned no ETag: {part_number}")
        parts.append({"part_number": part_number, "etag": etag})
        rss_peak = max(rss_peak, runtime.rss_kib())
        if part_number == restart_at:
            runtime.stop_api()
            runtime.start_api()
            restarts += 1
            rss_peak = max(rss_peak, runtime.rss_kib())
        if part_number % 20 == 0 or part_number == part_count:
            print(f"large-multipart progress parts={part_number}/{part_count}", file=sys.stderr)
    client.complete_multipart(upload_id, parts)
    client.complete_multipart(upload_id, parts)
    if client.status(upload_id).get("state") != "quarantined":
        raise RuntimeError("large multipart object is not quarantined")
    rss_peak = max(rss_peak, runtime.rss_kib())
    rss_delta = rss_peak - rss_start
    if rss_delta > 32 * 1024 or restarts != 1:
        raise RuntimeError(
            f"large multipart resource gate failed: rss={rss_delta} restarts={restarts}"
        )
    return {
        "bytes": total_bytes,
        "parts": part_count,
        "api_rss_start_kib": rss_start,
        "api_rss_peak_kib": rss_peak,
        "api_rss_delta_kib": rss_delta,
        "direct_body": True,
        "api_restarts": restarts,
        "duplicate_complete": True,
        "quarantined": True,
    }
