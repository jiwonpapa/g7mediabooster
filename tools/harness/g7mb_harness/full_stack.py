"""Python-owned API, MinIO, worker, and native-media full-stack harness."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

from .evidence import Evidence
from .full_stack_large import large_multipart_scenario
from .full_stack_runtime import HMAC_SECRET, FullStackRuntime
from .full_stack_scenarios import normal_scenario
from .media_protocol import ControlClient


def repository_root() -> Path:
    """Return the source repository root."""

    return Path(__file__).resolve().parents[3]


def environment_bool(name: str, default: bool = False) -> bool:
    """Parse one strict boolean environment option."""

    value = os.environ.get(name, str(default)).lower()
    if value not in {"true", "false"}:
        raise RuntimeError(f"{name} must be true or false")
    return value == "true"


def environment_bytes(name: str, default: int = 0) -> int:
    """Parse and bound a large multipart byte count."""

    raw = os.environ.get(name, str(default))
    if not raw.isdigit():
        raise RuntimeError(f"{name} must be an integer")
    value = int(raw)
    if value and not 5 * 1024 * 1024 <= value <= 5 * 1024 * 1024 * 1024:
        raise RuntimeError(f"{name} must be 0 or between 5MiB and 5GiB")
    return value


def add_arguments(parser: argparse.ArgumentParser) -> None:
    """Register full-stack options while retaining environment compatibility."""

    parser.add_argument(
        "--api-addr",
        default=os.environ.get("G7MB_FULL_STACK_API_ADDR", "127.0.0.1:18088"),
    )
    parser.add_argument("--policy-smoke", action="store_true")
    parser.add_argument("--large-multipart-bytes", type=int)


def main(args: argparse.Namespace) -> int:
    """Execute the selected scenario and emit structured evidence."""

    evidence = Evidence("full-stack-smoke")
    try:
        policy_smoke = args.policy_smoke or environment_bool("G7MB_FULL_STACK_POLICY_SMOKE")
        large_bytes = (
            args.large_multipart_bytes
            if args.large_multipart_bytes is not None
            else environment_bytes("G7MB_FULL_STACK_LARGE_MULTIPART_BYTES")
        )
        if large_bytes is not None and large_bytes < 0:
            raise RuntimeError("large multipart bytes cannot be negative")
        root = repository_root()
        with FullStackRuntime(root, args.api_addr, large_bytes) as runtime:
            evidence.set_phase("dependencies")
            runtime.require_tools(policy_smoke)
            evidence.set_phase("minio-conformance")
            runtime.start_minio()
            evidence.set_phase("media-fixtures")
            fixtures = runtime.build_fixtures()
            evidence.set_phase("native-capabilities")
            capabilities = runtime.build_binaries()
            evidence.add("sandbox_capabilities", capabilities)
            runtime.configure()
            runtime.start_api()
            client = ControlClient(runtime.api_base, HMAC_SECRET)
            if large_bytes:
                evidence.set_phase("large-multipart")
                facts = large_multipart_scenario(runtime, client, large_bytes)
            else:
                evidence.set_phase("media-processing")
                facts = normal_scenario(runtime, client, fixtures, policy_smoke)
            for key, value in facts.items():
                evidence.add(key, value)
        evidence.set_phase("complete")
        print(evidence.render("PASS"))
        return 0
    except (
        RuntimeError,
        OSError,
        ValueError,
        json.JSONDecodeError,
        subprocess.SubprocessError,
    ) as error:
        print(evidence.render("FAIL", str(error)), file=sys.stderr)
        return 1
