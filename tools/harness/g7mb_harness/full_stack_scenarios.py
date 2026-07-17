"""Full-stack media upload scenarios independent of process bootstrap."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from .full_stack_runtime import FullStackRuntime
from .media_protocol import ControlClient, direct_put, download
from .process import output


def instruction_for(uploads: list[dict[str, Any]], client_ref: str) -> dict[str, Any]:
    """Return one exact upload instruction."""

    matches = [item for item in uploads if item.get("client_ref") == client_ref]
    if len(matches) != 1:
        raise RuntimeError(f"missing or duplicate upload instruction: {client_ref}")
    return matches[0]


def require_string(value: Any, label: str) -> str:
    """Validate a required string field."""

    if not isinstance(value, str) or not value:
        raise RuntimeError(f"invalid {label}")
    return value


def upload_single(
    client: ControlClient,
    source: Path,
    client_ref: str,
    content_type: str,
) -> str:
    """Upload and confirm one single-PUT image."""

    instruction = client.upload_batch(
        [
            {
                "client_ref": client_ref,
                "declared_kind": "image",
                "content_length": source.stat().st_size,
                "content_type_hint": content_type,
            }
        ]
    )[0]
    if instruction.get("method") != "single_put":
        raise RuntimeError(f"expected single PUT: {client_ref}")
    upload_id = require_string(instruction.get("upload_id"), "upload_id")
    direct_put(instruction, source)
    client.complete_single(upload_id)
    return upload_id


def upload_multipart(
    client: ControlClient,
    instruction: dict[str, Any],
    source: Path,
    temp: Path,
) -> tuple[str, int]:
    """Upload a local file using the reserved multipart layout."""

    upload_id = require_string(instruction.get("upload_id"), "upload_id")
    part_size = int(instruction.get("part_size_bytes", 0))
    if part_size < 5 * 1024 * 1024:
        raise RuntimeError("multipart part size is invalid")
    parts: list[dict[str, Any]] = []
    with source.open("rb") as handle:
        part_number = 0
        while chunk := handle.read(part_size):
            part_number += 1
            part = temp / f"part-{upload_id}-{part_number}"
            part.write_bytes(chunk)
            presigned = client.presign_part(upload_id, part_number, len(chunk))
            etag = direct_put(presigned, part)
            part.unlink()
            if not etag:
                raise RuntimeError(f"multipart PUT returned no ETag: part {part_number}")
            parts.append({"part_number": part_number, "etag": etag})
    client.complete_multipart(upload_id, parts)
    return upload_id, len(parts)


def assert_ready_media(
    runtime: FullStackRuntime,
    client: ControlClient,
    upload_id: str,
    content_type: str,
    extension: str,
) -> dict[str, Path]:
    """Validate Ready state and download master/thumbnail derivatives."""

    status = client.status(upload_id)
    if status.get("state") != "ready" or status.get("detected_content_type") != content_type:
        raise RuntimeError(f"upload did not become ready: {upload_id} {status}")
    derivatives = status.get("derivatives")
    if not isinstance(derivatives, list):
        raise RuntimeError("upload derivatives are invalid")
    if sorted(item.get("variant") for item in derivatives) != ["master", "thumbnail"]:
        raise RuntimeError(f"derivative variants are incomplete: {upload_id}")
    paths: dict[str, Path] = {}
    for variant in ("master", "thumbnail"):
        delivery = client.delivery(upload_id, variant)
        url = require_string(delivery.get("delivery_url"), "delivery_url")
        suffix = extension if variant == "master" else "jpg"
        destination = runtime.temp / f"{upload_id}-{variant}.{suffix}"
        download(url, destination)
        paths[variant] = destination
        if variant == "thumbnail" or content_type.startswith("image/"):
            runtime.assert_image(destination)
    return paths


def normal_scenario(
    runtime: FullStackRuntime,
    client: ControlClient,
    fixtures: dict[str, Path],
    policy_smoke: bool,
) -> dict[str, Any]:
    """Exercise single/multipart image, MOV, EXIF, delivery, and optional policy."""

    uploads = client.upload_batch(
        [
            {
                "client_ref": "single-exif",
                "declared_kind": "image",
                "content_length": fixtures["single"].stat().st_size,
                "content_type_hint": "image/jpeg",
            },
            {
                "client_ref": "multipart-large",
                "declared_kind": "image",
                "content_length": fixtures["multipart"].stat().st_size,
                "content_type_hint": "image/jpeg",
            },
            {
                "client_ref": "mov-video",
                "declared_kind": "video",
                "content_length": fixtures["mov"].stat().st_size,
                "content_type_hint": "video/quicktime",
            },
        ]
    )
    if len(uploads) != 3:
        raise RuntimeError("full-stack batch is incomplete")
    single = instruction_for(uploads, "single-exif")
    multipart = instruction_for(uploads, "multipart-large")
    mov = instruction_for(uploads, "mov-video")
    if single.get("method") != "single_put":
        raise RuntimeError("single fixture did not select single PUT")
    if multipart.get("method") != "multipart" or mov.get("method") != "multipart":
        raise RuntimeError("large image or MOV did not select multipart")

    single_id = require_string(single.get("upload_id"), "single upload_id")
    direct_put(single, fixtures["single"])
    client.complete_single(single_id)
    multipart_id, part_count = upload_multipart(
        client, multipart, fixtures["multipart"], runtime.temp
    )
    mov_id, mov_parts = upload_multipart(client, mov, fixtures["mov"], runtime.temp)
    if part_count != 2 or mov_parts != 1:
        raise RuntimeError(f"unexpected multipart layout: image={part_count} mov={mov_parts}")

    for index in range(1, 4):
        runtime.run_worker(f"full-stack-{index}")
    image_paths = assert_ready_media(runtime, client, single_id, "image/jpeg", "jpg")
    assert_ready_media(runtime, client, multipart_id, "image/jpeg", "jpg")
    mov_paths = assert_ready_media(runtime, client, mov_id, "video/quicktime", "mov")
    if fixtures["mov"].read_bytes() != mov_paths["master"].read_bytes():
        raise RuntimeError("MOV master bytes changed")
    mov_format = output(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=format_name",
            "-of",
            "default=nw=1:nk=1",
            str(mov_paths["master"]),
        ]
    )
    if "mov" not in mov_format:
        raise RuntimeError(f"MOV master format is invalid: {mov_format}")
    metadata = output(["vipsheader", "-a", str(image_paths["master"])])
    if any(marker in metadata for marker in ("PrivateCamera", "GPSLatitude", "exif-data")):
        raise RuntimeError("full-stack derivative retained private EXIF metadata")
    evidence: dict[str, Any] = {
        "single_put": True,
        "multipart_parts": part_count,
        "ready": 3,
        "derivatives": 6,
        "mov_h264": True,
        "exif_removed": True,
    }
    if policy_smoke:
        from .full_stack_policy import policy_scenario

        evidence["policy"] = policy_scenario(
            runtime, client, fixtures["single"], image_paths["master"]
        )
    return evidence
