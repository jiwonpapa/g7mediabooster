#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/g7mb-native.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

export VIPS_CONCURRENCY=1

cd "$ROOT"

file_size() {
    if stat -f%z "$1" >/dev/null 2>&1; then
        stat -f%z "$1"
    else
        stat -c%s "$1"
    fi
}

# Doctor fails on dynamic-loader warnings even when the executable returns zero.
cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- doctor
runtime_capabilities="$(cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- capabilities)"
[[ "$runtime_capabilities" == *'"image_inputs":["avif","gif","heif","jpeg","png","webp"]'* ]]
[[ "$runtime_capabilities" == *'"image_outputs":["avif","jpeg","png","webp"]'* ]]
[[ "$runtime_capabilities" == *'"video_inputs":["mov","mp4"]'* ]]
[[ "$runtime_capabilities" == *'"mp4_thumbnail":true'* ]]
[[ "$runtime_capabilities" == *'"mp4_h264_fallback":true'* ]]

for format in jpeg webp avif png; do
    output="$TMP/thumb.$format"
    if [[ "$format" == "jpeg" ]]; then
        output="$TMP/thumb.jpg"
    fi
    cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
        image-thumbnail \
        --input tests/fixtures/tiny.ppm \
        --output "$output" \
        --max-edge 4 \
        --format "$format" \
        --threads 1
    test -s "$output"
    vips copy "$output" "$TMP/decoded-$format.png"
    test -s "$TMP/decoded-$format.png"
done

# Exercise a real HEIC/HEIF input through signature detection, decoder probe, and JPEG output.
vips copy tests/fixtures/tiny.ppm "$TMP/tiny.heic"
heif_probe="$(cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    probe \
    --input "$TMP/tiny.heic" \
    --declared-kind image \
    --byte-len "$(file_size "$TMP/tiny.heic")" \
    --threads 1)"
[[ "$heif_probe" == *'"format":"heif"'* ]]
cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    image-thumbnail \
    --input "$TMP/tiny.heic" \
    --output "$TMP/heif-thumb.jpg" \
    --max-edge 4 \
    --format jpeg \
    --threads 1
test -s "$TMP/heif-thumb.jpg"

cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    image-thumbnail \
    --input tests/fixtures/tiny.ppm \
    --output "$TMP/watermarked.jpg" \
    --max-edge 8 \
    --format jpeg \
    --threads 1 \
    --watermark tests/fixtures/tiny.ppm \
    --watermark-position bottom-right \
    --watermark-margin-px 0 \
    --watermark-max-width-percent 50 \
    --watermark-opacity-percent 75
test -s "$TMP/watermarked.jpg"
[[ "$(vipsheader -f bands "$TMP/watermarked.jpg")" == "3" ]]
if cmp -s "$TMP/thumb.jpg" "$TMP/watermarked.jpg"; then
    echo "watermark did not change the rendered image bytes" >&2
    exit 1
fi

if base64 --decode <tests/fixtures/private-exif.jpg.b64 >"$TMP/private-exif.jpg" 2>/dev/null; then
    :
else
    base64 -D <tests/fixtures/private-exif.jpg.b64 >"$TMP/private-exif.jpg"
fi
cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    image-thumbnail \
    --input "$TMP/private-exif.jpg" \
    --output "$TMP/sanitized.jpg" \
    --max-edge 8 \
    --format jpeg \
    --threads 1
sanitized_metadata="$(vipsheader -a "$TMP/sanitized.jpg" 2>/dev/null)"
if [[ "$sanitized_metadata" == *"PrivateCamera"* \
    || "$sanitized_metadata" == *"exif-data"* \
    || "$sanitized_metadata" == *"GPSLatitude"* ]]; then
    echo "sanitized image retained private metadata" >&2
    exit 1
fi

image_probe="$(cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    probe \
    --input "$TMP/thumb.jpg" \
    --declared-kind image \
    --byte-len "$(file_size "$TMP/thumb.jpg")" \
    --threads 1)"
[[ "$image_probe" == *'"kind":"image"'* ]]
[[ "$image_probe" == *'"format":"jpeg"'* ]]

printf '%s' '<?php echo "not an image";' >"$TMP/fake.jpg"
if cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    probe \
    --input "$TMP/fake.jpg" \
    --declared-kind image \
    --byte-len "$(file_size "$TMP/fake.jpg")" \
    --threads 1; then
    echo "sandbox accepted a disguised non-image" >&2
    exit 1
fi

ffmpeg -hide_banner -loglevel error -nostdin \
    -f lavfi -i "color=c=blue:s=320x180:r=10" \
    -t 1 -c:v libx264 -pix_fmt yuv420p -threads 1 -y "$TMP/source.mp4"

video_probe="$(cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    probe \
    --input "$TMP/source.mp4" \
    --declared-kind video \
    --byte-len "$(file_size "$TMP/source.mp4")" \
    --timeout-seconds 15 \
    --threads 1)"
[[ "$video_probe" == *'"kind":"video"'* ]]
[[ "$video_probe" == *'"codec":"h264"'* ]]

cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    video-thumbnail \
    --input "$TMP/source.mp4" \
    --output "$TMP/frame.jpg" \
    --timestamp-ms 100 \
    --duration-ms 1000 \
    --max-width 160 \
    --timeout-seconds 15 \
    --threads 1

test -s "$TMP/frame.jpg"
vips copy "$TMP/frame.jpg" "$TMP/frame.png"
test -s "$TMP/frame.png"

# Prove that runtime thumbnail extraction survives an unavailable FFmpeg binary for MP4/H.264.
cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    video-thumbnail \
    --input "$TMP/source.mp4" \
    --output "$TMP/frame-openh264.jpg" \
    --timestamp-ms 100 \
    --duration-ms 1000 \
    --max-width 160 \
    --timeout-seconds 15 \
    --threads 1 \
    --ffmpeg-bin "$TMP/missing-ffmpeg" \
    --allow-openh264-fallback
test -s "$TMP/frame-openh264.jpg"
[[ "$(vipsheader -f width "$TMP/frame-openh264.jpg")" -le 160 ]]
[[ "$(vipsheader -f bands "$TMP/frame-openh264.jpg")" == "3" ]]
test ! -e "$TMP/frame-openh264.fallback.ppm"

cargo run --quiet --locked --package g7mb-sandbox --features native-vips -- \
    video-thumbnail \
    --input "$TMP/source.mp4" \
    --output "$TMP/frame-watermarked.jpg" \
    --timestamp-ms 100 \
    --duration-ms 1000 \
    --max-width 160 \
    --timeout-seconds 15 \
    --threads 1 \
    --watermark tests/fixtures/tiny.ppm \
    --watermark-position bottom-right \
    --watermark-margin-px 0 \
    --watermark-max-width-percent 50 \
    --watermark-opacity-percent 75
test -s "$TMP/frame-watermarked.jpg"
test ! -e "$TMP/frame-watermarked.frame.jpg"
[[ "$(vipsheader -f bands "$TMP/frame-watermarked.jpg")" == "3" ]]
