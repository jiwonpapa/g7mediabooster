#!/usr/bin/env bash
set -euo pipefail

vips --version
ffmpeg -version
ffmpeg -buildconf

if command -v brew >/dev/null 2>&1; then
    brew list --versions vips ffmpeg libheif x265 2>/dev/null || true
elif command -v dpkg-query >/dev/null 2>&1; then
    dpkg-query -W 'libvips*' 'ffmpeg*' 'libheif*' 2>/dev/null || true
fi
