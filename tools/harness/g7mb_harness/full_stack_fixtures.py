"""Native fixture and binary preparation for full-stack scenarios."""

from __future__ import annotations

import base64
import json
from pathlib import Path
from typing import TYPE_CHECKING, Any

from .process import output, run

if TYPE_CHECKING:
    from .full_stack_runtime import FullStackRuntime


def build_fixtures(runtime: FullStackRuntime) -> dict[str, Path]:
    """Create deterministic image/video fixtures using native tools."""

    fixtures = {
        "single": runtime.temp / "private-exif.jpg",
        "multipart": runtime.temp / "multipart.jpg",
        "mov": runtime.temp / "video.mov",
    }
    encoded = (runtime.root / "tests/fixtures/private-exif.jpg.b64").read_text().strip()
    fixtures["single"].write_bytes(base64.b64decode(encoded))
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
            "nullsrc=s=4000x3000,noise=alls=100:allf=t",
            "-frames:v",
            "1",
            "-c:v",
            "mjpeg",
            "-q:v",
            "1",
            "-threads",
            "1",
            "-y",
            str(fixtures["multipart"]),
        ]
    )
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
            "color=c=blue:s=320x180:r=10",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-threads",
            "1",
            "-map_metadata",
            "-1",
            "-f",
            "mov",
            "-y",
            str(fixtures["mov"]),
        ]
    )
    if not 0 < fixtures["single"].stat().st_size < 5 * 1024 * 1024:
        raise RuntimeError("single fixture is outside the single-PUT policy")
    if fixtures["multipart"].stat().st_size <= 5 * 1024 * 1024:
        raise RuntimeError("multipart fixture did not cross the multipart threshold")
    return fixtures


def build_binaries(runtime: FullStackRuntime) -> dict[str, Any]:
    """Build API, worker, sandbox and validate native capability output."""

    run(
        [
            "cargo",
            "build",
            "--quiet",
            "--locked",
            "--package",
            "g7mb-api",
            "--package",
            "g7mb-worker",
        ],
        cwd=runtime.root,
    )
    run(
        [
            "cargo",
            "build",
            "--quiet",
            "--locked",
            "--package",
            "g7mb-sandbox",
            "--features",
            "native-vips",
        ],
        cwd=runtime.root,
    )
    decoded = json.loads(output(["target/debug/g7mb-sandbox", "capabilities"], cwd=runtime.root))
    if not isinstance(decoded, dict) or not all(isinstance(key, str) for key in decoded):
        raise RuntimeError("sandbox capability output is not a JSON object")
    capabilities: dict[str, Any] = {str(key): value for key, value in decoded.items()}
    required = {
        "image_inputs": {"avif", "gif", "heif", "jpeg", "png", "webp"},
        "image_outputs": {"avif", "jpeg", "png", "webp"},
        "video_inputs": {"mov", "mp4"},
    }
    for key, expected in required.items():
        if not expected.issubset(set(capabilities.get(key, []))):
            raise RuntimeError(f"sandbox capability is incomplete: {key}")
    if not capabilities.get("mp4_thumbnail") or not capabilities.get("mp4_h264_fallback"):
        raise RuntimeError("sandbox video fallbacks are incomplete")
    return capabilities
