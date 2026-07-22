"""Native image and load gates with bounded resources and JSON evidence."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
import time
from datetime import UTC, datetime
from pathlib import Path

from .process import output, require_programs, run
from .resource_monitor import ResourceLimitError, ResourceUsage, positive_int_env, run_monitored


def repository_root() -> Path:
    """Resolve the repository root independently of the caller directory."""

    return Path(__file__).resolve().parents[3]


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _expect_header(path: Path, field: str, expected: str) -> None:
    actual = output(["vipsheader", "-f", field, str(path)])
    if actual != expected:
        raise RuntimeError(f"{path.name} {field}: expected {expected}, got {actual}")


def _expect_probe(document: dict[str, object], image_format: str, width: int, height: int) -> None:
    details = document.get("probe")
    dimensions = None
    if isinstance(details, dict):
        dimensions = (details.get("width"), details.get("height"))
    if document.get("format") != image_format or dimensions != (width, height):
        raise RuntimeError(f"{image_format} probe contract changed")


def _write_report(root: Path, name: str, values: dict[str, object]) -> Path:
    report = root / "reports" / f"{name}.json"
    report.parent.mkdir(parents=True, exist_ok=True)
    generated = datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    document = {"schema_version": 1, "generated_at": generated, **values}
    report.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    return report


def _build_sandbox(root: Path, env: dict[str, str]) -> Path:
    command = ["cargo", "build", "--quiet", "--locked", "--package", "g7mb-sandbox"]
    run([*command, "--features", "native-vips"], cwd=root, env=env)
    return root / "target" / "debug" / "g7mb-sandbox"


def _probe_command(sandbox: Path, source: Path, byte_len: int) -> list[str]:
    return [str(sandbox), "probe", "--input", str(source), "--declared-kind", "image",
            "--byte-len", str(byte_len), "--threads", "1"]


def _thumbnail_command(sandbox: Path, source: Path, thumbnail: Path) -> list[str]:
    return [str(sandbox), "image-thumbnail", "--input", str(source), "--output",
            str(thumbnail), "--max-edge", "1280", "--format", "jpeg", "--threads", "1"]


def _accepted_image(
    root: Path, sandbox: Path, source: Path, thumbnail: Path, byte_len: int,
    timeout: int, max_rss: int, env: dict[str, str],
) -> tuple[dict[str, object], ResourceUsage]:
    probe, log = source.parent / "probe.json", source.parent / "process.log"
    started = time.monotonic()
    usages: list[ResourceUsage] = []
    commands = ((_probe_command(sandbox, source, byte_len), probe),
                (_thumbnail_command(sandbox, source, thumbnail), None))
    for command, stdout_path in commands:
        remaining = timeout - (time.monotonic() - started)
        if remaining <= 0:
            raise ResourceLimitError(f"image gate exceeded {timeout}s")
        usages.append(run_monitored(
            command, cwd=root, env=env, log_path=log, stdout_path=stdout_path,
            timeout_seconds=remaining, max_rss_kib=max_rss,
        ))
    usage = ResourceUsage(round((time.monotonic() - started) * 1000),
                          max(item.peak_rss_kib for item in usages))
    return json.loads(probe.read_text(encoding="utf-8")), usage


def heavy_image() -> None:
    """Validate a real 25,000px JPEG inside the heavy-lane resource budget."""

    root = repository_root()
    require_programs(["cargo", "ps", "vips", "vipsheader"])
    max_rss = positive_int_env("G7MB_HEAVY_MAX_RSS_KIB", 1_048_576)
    timeout = positive_int_env("G7MB_HEAVY_MAX_WALL_SECONDS", 60)
    env = {"VIPS_CONCURRENCY": "1"}
    with tempfile.TemporaryDirectory(prefix="g7mb-heavy-image.") as temporary:
        work = Path(temporary)
        source, thumbnail = work / "source.jpg", work / "thumbnail.jpg"
        run(["vips", "black", str(source), "25000", "4000", "--bands", "3"], env=env)
        _expect_header(source, "width", "25000")
        _expect_header(source, "height", "4000")
        byte_len = source.stat().st_size
        probe, usage = _accepted_image(root, _build_sandbox(root, env), source, thumbnail,
                                       byte_len, timeout, max_rss, env)
        _expect_probe(probe, "jpeg", 25000, 4000)
        _expect_header(thumbnail, "width", "1280")
        _expect_header(thumbnail, "bands", "3")
        fixture = {"format": "jpeg", "width": 25000, "height": 4000,
                   "pixels": 100_000_000, "bytes": byte_len, "sha256": _sha256(source)}
        report = _write_report(root, "heavy-image", {
            "fixture": fixture, "worker_class": "heavy", "native_threads": 1,
            "output": {"format": "jpeg", "max_edge": 1280, "metadata_policy": "strip"},
            "elapsed_ms": usage.elapsed_ms, "peak_process_tree_rss_kib": usage.peak_rss_kib,
            "max_process_tree_rss_kib": max_rss, "result": "pass",
        })
    print("heavy-image PASS dimensions=25000x4000 "
          f"elapsed_ms={usage.elapsed_ms} peak_rss_kib={usage.peak_rss_kib} class=heavy")
    print(f"report={report}")


def heavy_avif() -> None:
    """Validate 64MP AVIF processing and the 200MP pre-decode rejection boundary."""

    root = repository_root()
    require_programs(["cargo", "ps", "vips", "vipsheader"])
    max_rss = positive_int_env("G7MB_HEAVY_AVIF_MAX_RSS_KIB", 1_572_864)
    timeout = positive_int_env("G7MB_HEAVY_AVIF_MAX_WALL_SECONDS", 90)
    env = {"VIPS_CONCURRENCY": "1"}
    with tempfile.TemporaryDirectory(prefix="g7mb-heavy-avif.") as temporary:
        work = Path(temporary)
        source, rejected = work / "source.avif", work / "rejected-200mp.avif"
        fixture_started = time.monotonic()
        run(["vips", "black", str(source), "8000", "8000", "--bands", "3"], env=env)
        run(["vips", "black", str(rejected), "16000", "12500", "--bands", "3"], env=env)
        generation_ms = round((time.monotonic() - fixture_started) * 1000)
        for path, width, height in ((source, "8000", "8000"),
                                    (rejected, "16000", "12500")):
            _expect_header(path, "width", width)
            _expect_header(path, "height", height)
        sandbox = _build_sandbox(root, env)
        refusal = run(_probe_command(sandbox, rejected, rejected.stat().st_size),
                      cwd=root, env=env, capture=True, check=False)
        rejection = "image resource policy rejected the source"
        if refusal.returncode == 0 or rejection not in refusal.stderr:
            raise RuntimeError("200 MP AVIF was not rejected by the decoder memory policy")
        thumbnail = work / "thumbnail.jpg"
        probe, usage = _accepted_image(root, sandbox, source, thumbnail,
                                       source.stat().st_size, timeout, max_rss, env)
        _expect_probe(probe, "avif", 8000, 8000)
        for field in ("width", "height"):
            _expect_header(thumbnail, field, "1280")
        _expect_header(thumbnail, "bands", "3")
        accepted = {"format": "avif", "width": 8000, "height": 8000,
                    "pixels": 64_000_000, "bytes": source.stat().st_size,
                    "sha256": _sha256(source), "generation_ms": generation_ms}
        boundary = {"format": "avif", "width": 16000, "height": 12500,
                    "pixels": 200_000_000, "bytes": rejected.stat().st_size,
                    "reason": "decoder_memory_policy"}
        report = _write_report(root, "heavy-avif", {
            "accepted_fixture": accepted, "rejected_boundary": boundary,
            "worker_class": "heavy", "native_threads": 1,
            "output": {"format": "jpeg", "width": 1280, "height": 1280,
                       "metadata_policy": "strip"},
            "elapsed_ms": usage.elapsed_ms, "peak_process_tree_rss_kib": usage.peak_rss_kib,
            "max_process_tree_rss_kib": max_rss, "result": "pass",
        })
    print("heavy-avif PASS accepted=8000x8000/64MP rejected=16000x12500/200MP "
          f"elapsed_ms={usage.elapsed_ms} peak_rss_kib={usage.peak_rss_kib} class=heavy")
    print(f"report={report}")


def _load_fields(log: Path) -> dict[str, str]:
    lines = [line for line in log.read_text(encoding="utf-8", errors="replace").splitlines()
             if "G7MB_LOAD_RESULT" in line]
    if not lines:
        raise RuntimeError("load test did not emit G7MB_LOAD_RESULT")
    fields = dict(field.split("=", 1) for field in lines[-1].split() if "=" in field)
    required = {"jobs", "elapsed_ms", "throughput_per_second", "p50_ms", "p95_ms", "p99_ms",
                "ready", "completed", "derivatives", "recovered", "dead_letter"}
    if missing := sorted(required - fields.keys()):
        raise RuntimeError(f"load result is missing fields: {', '.join(missing)}")
    return fields


def load_100() -> None:
    """Process 100 real JPEG jobs under concurrency, RSS, disk, and wall budgets."""

    root = repository_root()
    require_programs(["cargo", "ffmpeg", "ps", "vipsheader"])
    max_rss = positive_int_env("G7MB_LOAD_MAX_RSS_KIB", 1_572_864)
    max_disk = positive_int_env("G7MB_LOAD_MAX_TEMP_DISK_KIB", 1_048_576)
    timeout = positive_int_env("G7MB_LOAD_MAX_WALL_SECONDS", 240)
    concurrency = positive_int_env("G7MB_LOAD_CONCURRENCY", 4)
    env = {"VIPS_CONCURRENCY": "1"}
    with tempfile.TemporaryDirectory(prefix="g7mb-load-100.") as temporary:
        work, log = Path(temporary), Path(temporary) / "load-100.log"
        fixture, runtime = work / "fixture.jpg", work / "runtime"
        ffmpeg = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-f", "lavfi"]
        run([*ffmpeg, "-i", "testsrc2=size=4000x3000:rate=1", "-frames:v", "1",
             "-c:v", "mjpeg", "-q:v", "3", "-threads", "1", "-y", str(fixture)])
        _expect_header(fixture, "width", "4000")
        _expect_header(fixture, "height", "3000")
        sandbox = _build_sandbox(root, env)
        test = ["cargo", "test", "--quiet", "--locked", "--package", "g7mb-worker",
                "--test", "load_100"]
        run([*test, "--no-run"], cwd=root)
        runtime.mkdir()
        load_env = {**env, "G7MB_LOAD_FIXTURE": str(fixture),
                    "G7MB_SANDBOX_BIN": str(sandbox),
                    "G7MB_LOAD_CONCURRENCY": str(concurrency),
                    "G7MB_LOAD_RUNTIME_PARENT": str(runtime)}
        command = [*test, "load_100_real_jpeg_recovers_expired_leases", "--",
                   "--ignored", "--exact", "--nocapture"]
        usage = run_monitored(
            command, cwd=root, env=load_env, log_path=log, timeout_seconds=timeout,
            max_rss_kib=max_rss, disk_path=runtime, max_disk_kib=max_disk,
            sample_seconds=0.1,
        )
        fields = _load_fields(log)
        names = ("jobs", "elapsed_ms", "p50_ms", "p95_ms", "p99_ms", "ready", "completed",
                 "derivatives", "recovered", "dead_letter")
        integer = {name: int(fields[name]) for name in names}
        report = _write_report(root, "load-100", {
            "fixture": {"format": "jpeg", "width": 4000, "height": 3000,
                        "sha256": _sha256(fixture)},
            "jobs": integer["jobs"], "worker_concurrency": concurrency,
            "native_threads_per_job": 1, "elapsed_ms": integer["elapsed_ms"],
            "throughput_per_second": float(fields["throughput_per_second"]),
            "processing_latency_ms": {"p50": integer["p50_ms"], "p95": integer["p95_ms"],
                                      "p99": integer["p99_ms"]},
            "peak_process_tree_rss_kib": usage.peak_rss_kib,
            "max_process_tree_rss_kib": max_rss, "peak_temp_disk_kib": usage.peak_disk_kib,
            "max_temp_disk_kib": max_disk, "ready": integer["ready"],
            "completed_jobs": integer["completed"], "derivatives": integer["derivatives"],
            "recovered_expired_leases": integer["recovered"],
            "dead_letter": integer["dead_letter"], "result": "pass",
        })
    print(f"load-100 PASS jobs={fields['jobs']} elapsed_ms={fields['elapsed_ms']} "
          f"throughput_per_second={fields['throughput_per_second']} p95_ms={fields['p95_ms']} "
          f"peak_rss_kib={usage.peak_rss_kib} peak_temp_disk_kib={usage.peak_disk_kib} "
          f"recovered={fields['recovered']}")
    print(f"report={report}")


def main(command: str) -> int:
    """Dispatch one resource gate with concise failure output."""

    actions = {"heavy-image": heavy_image, "heavy-avif": heavy_avif, "load-100": load_100}
    try:
        actions[command]()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"{command} FAIL: {error}", file=sys.stderr)
        return 1
    return 0
