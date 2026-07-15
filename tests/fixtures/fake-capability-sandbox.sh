#!/bin/sh
set -eu

test "${1:-}" = "capabilities"
printf '%s' '{"image_inputs":["avif","gif","heif","jpeg","png","webp"],"image_outputs":["avif","jpeg","png","webp"],"mp4_thumbnail":true,"mp4_h264_fallback":true,"native_versions":{"ffmpeg":"fixture","ffprobe":"fixture","vips":"fixture"}}'
