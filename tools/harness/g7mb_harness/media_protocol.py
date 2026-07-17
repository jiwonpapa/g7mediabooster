"""Typed HTTP client used by media integration harnesses."""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
import secrets
import time
from pathlib import Path
from typing import Any
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

from .process import run


class ControlClient:
    """Minimal HMAC control-plane client with streaming direct PUT support."""

    def __init__(self, base_url: str, secret: str, key_id: str = "g7-primary") -> None:
        self.base_url = base_url.rstrip("/")
        if urlsplit(self.base_url).scheme not in {"http", "https"}:
            raise ValueError("control API URL must use HTTP or HTTPS")
        self.secret = secret.encode()
        self.key_id = key_id

    def request(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Send one authenticated JSON request."""

        body = b"" if payload is None else json.dumps(payload, separators=(",", ":")).encode()
        timestamp = str(int(time.time()))
        nonce = secrets.token_hex(16)
        body_sha = hashlib.sha256(body).hexdigest()
        canonical = "\n".join(
            ["G7MB-HMAC-SHA256", self.key_id, timestamp, nonce, method, path, body_sha]
        ).encode()
        signature = base64.urlsafe_b64encode(
            hmac.new(self.secret, canonical, hashlib.sha256).digest()
        ).rstrip(b"=")
        request = Request(  # noqa: S310 - constructor URL scheme validated
            f"{self.base_url}{path}",
            data=body,
            method=method,
            headers={
                "accept": "application/json",
                "content-type": "application/json",
                "x-g7mb-key-id": self.key_id,
                "x-g7mb-timestamp": timestamp,
                "x-g7mb-nonce": nonce,
                "x-g7mb-content-sha256": body_sha,
                "x-g7mb-signature": signature.decode(),
            },
        )
        try:
            with urlopen(request, timeout=30) as response:  # noqa: S310
                raw = response.read()
        except HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise RuntimeError(
                f"control API {method} {path} failed: {error.code} {detail}"
            ) from error
        if not raw:
            return {}
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise RuntimeError(f"control API returned non-object JSON: {path}")
        return value

    def upload_batch(self, files: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Create a batch and return validated upload instructions."""

        result = self.request("POST", "/v1/upload-batches", {"files": files})
        uploads = result.get("uploads")
        if not isinstance(uploads, list) or not all(isinstance(item, dict) for item in uploads):
            raise RuntimeError("upload batch returned invalid instructions")
        return uploads

    def complete_single(self, upload_id: str) -> None:
        """Confirm completion of a single PUT."""

        self.request("POST", f"/v1/uploads/{upload_id}/complete")

    def status(self, upload_id: str) -> dict[str, Any]:
        """Return upload processing state."""

        return self.request("GET", f"/v1/uploads/{upload_id}")

    def delivery(self, upload_id: str, variant: str) -> dict[str, Any]:
        """Create a derivative delivery URL."""

        return self.request("GET", f"/v1/uploads/{upload_id}/derivatives/{variant}/delivery")

    def presign_part(self, upload_id: str, part_number: int, length: int) -> dict[str, Any]:
        """Create a signed multipart part URL."""

        return self.request(
            "POST",
            f"/v1/uploads/{upload_id}/parts/{part_number}/presign",
            {"content_length": length},
        )

    def complete_multipart(self, upload_id: str, parts: list[dict[str, Any]]) -> None:
        """Publish a complete ordered multipart list."""

        self.request("POST", f"/v1/uploads/{upload_id}/multipart/complete", {"parts": parts})


def direct_put(instruction: dict[str, Any], source: Path) -> str:
    """Stream a file to a presigned URL and return the exposed ETag."""

    url = instruction.get("upload_url")
    headers = instruction.get("required_headers")
    if not isinstance(url, str) or not isinstance(headers, dict):
        raise RuntimeError("invalid presigned PUT instruction")
    args = [
        "curl",
        "--fail-with-body",
        "--silent",
        "--show-error",
        "--request",
        "PUT",
        "--dump-header",
        "-",
        "--output",
        "/dev/null",
    ]
    for name, value in headers.items():
        if not isinstance(name, str) or not isinstance(value, str):
            raise RuntimeError("invalid required upload header")
        args.extend(["--header", f"{name}: {value}"])
    args.extend(["--data-binary", f"@{source}", url])
    completed = run(args, capture=True)
    etag = ""
    for line in completed.stdout.splitlines():
        name, separator, value = line.partition(":")
        if separator and name.lower() == "etag":
            etag = value.strip()
    return etag


def download(url: str, destination: Path) -> None:
    """Download a signed object without loading it into Python memory."""

    run(
        [
            "curl",
            "--fail-with-body",
            "--silent",
            "--show-error",
            "--output",
            str(destination),
            url,
        ]
    )
