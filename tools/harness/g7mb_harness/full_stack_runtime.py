"""Process and fixture runtime for the full-stack media harness."""

from __future__ import annotations

import os
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any
from urllib.error import URLError
from urllib.parse import urlsplit
from urllib.request import urlopen

from .process import merged_env, output, require_programs, run

MINIO_IMAGE = (
    "quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e"
)
ACCESS_KEY = "g7mbtestaccess"
SECRET_KEY = "g7mbtestsecret0123456789"  # noqa: S105 - isolated MinIO fixture
RAW_BUCKET = "g7mb-full-stack-raw"
DERIVATIVE_BUCKET = "g7mb-full-stack-media"
HMAC_SECRET = "replace-with-at-least-32-characters"  # noqa: S105 - test-only fixture


class FullStackRuntime:
    """Own MinIO, API, fixtures, worker processes, and cleanup."""

    def __init__(self, root: Path, api_addr: str, large_bytes: int) -> None:
        self.root = root
        self.api_addr = api_addr
        self.api_base = f"http://{api_addr}"
        self.large_bytes = large_bytes
        self.container = f"g7mb-full-stack-minio-{os.getpid()}"
        self.temporary = tempfile.TemporaryDirectory(prefix="g7mb-full-stack.")
        self.temp = Path(self.temporary.name)
        self.api_log = self.temp / "api.log"
        self.api_process: subprocess.Popen[str] | None = None
        self.api_log_handle: Any | None = None
        self.endpoint = ""
        self.environment: dict[str, str] = {}

    def __enter__(self) -> FullStackRuntime:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def close(self) -> None:
        """Terminate owned processes and remove the MinIO container."""

        self.stop_api()
        run(["docker", "rm", "--force", self.container], check=False, capture=True)
        self.temporary.cleanup()

    def require_tools(self, policy_smoke: bool) -> None:
        """Check explicit external runtime dependencies."""

        tools = ["cargo", "curl", "docker", "ffmpeg", "ffprobe", "vipsheader"]
        if policy_smoke:
            tools.append("php")
        require_programs(tools)
        run(["docker", "info"], capture=True)

    def start_minio(self) -> None:
        """Start pinned MinIO and execute the Rust protocol conformance suite."""

        run(
            [
                "docker",
                "run",
                "--detach",
                "--rm",
                "--name",
                self.container,
                "--env",
                f"MINIO_ROOT_USER={ACCESS_KEY}",
                "--env",
                f"MINIO_ROOT_PASSWORD={SECRET_KEY}",
                "--publish",
                "127.0.0.1::9000",
                MINIO_IMAGE,
                "server",
                "/data",
            ],
            capture=True,
        )
        port_line = output(["docker", "port", self.container, "9000/tcp"])
        port = port_line.rsplit(":", 1)[-1]
        if not port.isdigit():
            raise RuntimeError(f"MinIO published port is invalid: {port_line}")
        self.endpoint = f"http://127.0.0.1:{port}"
        self.wait_url(f"{self.endpoint}/minio/health/live", attempts=120, interval=0.25)
        test_env = {
            "G7MB_TEST_S3_ENDPOINT": self.endpoint,
            "G7MB_TEST_S3_ACCESS_KEY": ACCESS_KEY,
            "G7MB_TEST_S3_SECRET_KEY": SECRET_KEY,
            "G7MB_TEST_S3_RAW_BUCKET": RAW_BUCKET,
            "G7MB_TEST_S3_DERIVATIVE_BUCKET": DERIVATIVE_BUCKET,
        }
        run(
            [
                "cargo",
                "test",
                "--quiet",
                "--locked",
                "--package",
                "g7mb-object-store-s3",
                "--test",
                "minio_conformance",
                "--",
                "--ignored",
                "--nocapture",
            ],
            cwd=self.root,
            env=test_env,
        )

    @staticmethod
    def wait_url(url: str, *, attempts: int, interval: float) -> None:
        """Wait for a successful HTTP response."""

        if urlsplit(url).scheme not in {"http", "https"}:
            raise RuntimeError(f"unsupported readiness URL scheme: {url}")
        for _ in range(attempts):
            try:
                with urlopen(url, timeout=1) as response:  # noqa: S310 - scheme checked
                    if 200 <= response.status < 300:
                        return
            except (OSError, URLError):
                pass
            time.sleep(interval)
        raise RuntimeError(f"HTTP readiness timed out: {url}")

    def build_fixtures(self) -> dict[str, Path]:
        """Create deterministic image/video fixtures."""

        from .full_stack_fixtures import build_fixtures

        return build_fixtures(self)

    def build_binaries(self) -> dict[str, Any]:
        """Build binaries and validate native capability output."""

        from .full_stack_fixtures import build_binaries

        return build_binaries(self)

    def configure(self) -> None:
        """Build isolated API/worker environment without secret command arguments."""

        (self.temp / "worker").mkdir()
        (self.temp / "backups").mkdir()
        part_size = 32 * 1024 * 1024 if self.large_bytes else 5 * 1024 * 1024
        self.environment = {
            "VIPS_CONCURRENCY": "1",
            "G7MB__SERVER__BIND_ADDR": self.api_addr,
            "G7MB__DATABASE__URL": f"sqlite://{self.temp / 'g7mb.db'}",
            "G7MB__DATABASE__BACKUP_DIRECTORY": str(self.temp / "backups"),
            "G7MB__STORAGE__PROVIDER": "generic",
            "G7MB__STORAGE__ENDPOINT_URL": self.endpoint,
            "G7MB__STORAGE__REGION": "us-east-1",
            "G7MB__STORAGE__RAW_BUCKET": RAW_BUCKET,
            "G7MB__STORAGE__DERIVATIVE_BUCKET": DERIVATIVE_BUCKET,
            "G7MB__STORAGE__ACCESS_KEY_ID": ACCESS_KEY,
            "G7MB__STORAGE__SECRET_ACCESS_KEY": SECRET_KEY,
            "G7MB__STORAGE__FORCE_PATH_STYLE": "true",
            "G7MB__UPLOAD__MULTIPART_THRESHOLD_BYTES": str(5 * 1024 * 1024),
            "G7MB__UPLOAD__MULTIPART_PART_SIZE_BYTES": str(part_size),
            "G7MB__WORKER__SANDBOX_BINARY": str(self.root / "target/debug/g7mb-sandbox"),
            "G7MB__WORKER__TEMP_DIRECTORY": str(self.temp / "worker"),
            "G7MB__WORKER__NATIVE_THREADS_PER_JOB": "1",
            "G7MB__WORKER__MAX_CONCURRENT_JOBS": "2",
            "G7MB__WORKER__MAX_CONCURRENT_HEAVY_IMAGES": "1",
            "G7MB__WORKER__MAX_CONCURRENT_VIDEOS": "1",
        }

    def start_api(self) -> None:
        """Start API and wait for readiness."""

        self.stop_api()
        self.api_log_handle = self.api_log.open("a", encoding="utf-8")
        self.api_process = subprocess.Popen(
            ["target/debug/g7mb-api", "--config", "config/g7mb.example.toml"],
            cwd=self.root,
            env=merged_env(self.environment),
            text=True,
            stdout=self.api_log_handle,
            stderr=subprocess.STDOUT,
        )
        for _ in range(180):
            if self.api_process.poll() is not None:
                break
            try:
                self.wait_url(f"{self.api_base}/health/ready", attempts=1, interval=0)
                return
            except RuntimeError:
                time.sleep(0.1)
        log = self.api_log.read_text(encoding="utf-8") if self.api_log.exists() else ""
        raise RuntimeError(f"API readiness failed:\n{log[-12000:]}")

    def stop_api(self) -> None:
        """Stop an owned API process."""

        if self.api_process is not None and self.api_process.poll() is None:
            self.api_process.terminate()
            try:
                self.api_process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.api_process.kill()
                self.api_process.wait(timeout=5)
        self.api_process = None
        if self.api_log_handle is not None:
            self.api_log_handle.close()
            self.api_log_handle = None

    def run_worker(self, worker_id: str) -> None:
        """Process one queued job."""

        run(
            [
                "target/debug/g7mb-worker",
                "--config",
                "config/g7mb.example.toml",
                "once",
                "--worker-id",
                worker_id,
            ],
            cwd=self.root,
            env=self.environment,
            capture=True,
        )

    def rss_kib(self) -> int:
        """Read current API RSS in KiB."""

        if self.api_process is None:
            raise RuntimeError("API is not running")
        return int(output(["ps", "-o", "rss=", "-p", str(self.api_process.pid)]))

    @staticmethod
    def assert_image(path: Path) -> None:
        """Require a decodable, non-empty image."""

        width = int(output(["vipsheader", "-f", "width", str(path)]))
        height = int(output(["vipsheader", "-f", "height", str(path)]))
        if width < 1 or height < 1:
            raise RuntimeError(f"derivative is not a valid image: {path}")
