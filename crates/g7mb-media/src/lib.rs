//! Bounded native media execution. Native image support is feature-gated.

use std::{path::PathBuf, process::Stdio, time::Duration};

use g7mb_application::{WatermarkPosition, WatermarkSpec};
use g7mb_domain::{ImageProbe, MediaKind, VideoProbe};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{process::Command, time::timeout};

/// Bounded MP4 demux and OpenH264 first-frame fallback.
#[cfg(feature = "openh264-fallback")]
pub mod openh264_fallback;

const SIGNATURE_PREFIX_BYTES: u64 = 4096;
const MAX_PROBE_OUTPUT_BYTES: usize = 64 * 1024;

/// Supported encoded format determined from file bytes, never its extension.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaFormat {
    /// JPEG raster image.
    Jpeg,
    /// PNG raster image.
    Png,
    /// GIF raster or animation.
    Gif,
    /// WebP raster or animation.
    Webp,
    /// AVIF ISO-BMFF image.
    Avif,
    /// HEIC/HEIF ISO-BMFF image.
    Heif,
    /// ISO-BMFF MP4-family video.
    Mp4,
    /// QuickTime MOV video.
    QuickTime,
    /// WebM video.
    Webm,
}

impl MediaFormat {
    /// Returns the media class that must match the authenticated reservation.
    #[must_use]
    pub const fn kind(self) -> MediaKind {
        match self {
            Self::Jpeg | Self::Png | Self::Gif | Self::Webp | Self::Avif | Self::Heif => {
                MediaKind::Image
            }
            Self::Mp4 | Self::QuickTime | Self::Webm => MediaKind::Video,
        }
    }

    /// Returns the normalized detected content type.
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
            Self::Heif => "image/heif",
            Self::Mp4 => "video/mp4",
            Self::QuickTime => "video/quicktime",
            Self::Webm => "video/webm",
        }
    }
}

/// Returns decoder-aware image limits that fit the default 2 GiB worker cgroup.
///
/// AV1/HEVC still-image decoders commonly materialize substantially more full-frame
/// memory than JPEG/WebP shrink-on-load paths. Keep those formats at 64 MP so a
/// crafted 200 MP AVIF is rejected after header probing instead of being OOM-killed.
#[must_use]
pub fn image_limits_for_format(format: MediaFormat) -> g7mb_domain::ImageLimits {
    let mut limits = g7mb_domain::ImageLimits::default();
    if matches!(format, MediaFormat::Avif | MediaFormat::Heif) {
        limits.max_pixels = 64_000_000;
    }
    limits
}

/// Detects a narrow supported format from a bounded file prefix.
pub fn detect_media_format(prefix: &[u8]) -> Result<MediaFormat, MediaError> {
    if prefix.starts_with(&[0xff, 0xd8, 0xff]) {
        return Ok(MediaFormat::Jpeg);
    }
    if prefix.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(MediaFormat::Png);
    }
    if prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a") {
        return Ok(MediaFormat::Gif);
    }
    if prefix.len() >= 12 && prefix.starts_with(b"RIFF") && &prefix[8..12] == b"WEBP" {
        return Ok(MediaFormat::Webp);
    }
    if prefix.len() >= 12 && &prefix[4..8] == b"ftyp" {
        return detect_iso_bmff(prefix);
    }
    if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
        && prefix.windows(4).any(|window| window == b"webm")
    {
        return Ok(MediaFormat::Webm);
    }
    Err(MediaError::UnsupportedSignature)
}

fn detect_iso_bmff(prefix: &[u8]) -> Result<MediaFormat, MediaError> {
    let declared_box_size = u32::from_be_bytes(
        prefix[0..4]
            .try_into()
            .map_err(|_| MediaError::UnsupportedSignature)?,
    );
    let box_size = usize::try_from(declared_box_size)
        .map_err(|_| MediaError::UnsupportedSignature)?
        .min(prefix.len());
    if box_size < 12 || box_size % 4 != 0 {
        return Err(MediaError::UnsupportedSignature);
    }
    let brands = prefix[8..box_size].chunks_exact(4);
    let mut detected = None;
    for brand in brands {
        detected = match brand {
            b"avif" | b"avis" => Some(MediaFormat::Avif),
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => Some(MediaFormat::Heif),
            b"qt  " if detected.is_none() => Some(MediaFormat::QuickTime),
            b"isom" | b"iso2" | b"mp41" | b"mp42" | b"avc1" | b"dash" | b"M4V "
                if detected.is_none() =>
            {
                Some(MediaFormat::Mp4)
            }
            _ => detected,
        };
        if detected == Some(MediaFormat::Avif) {
            break;
        }
    }
    detected.ok_or(MediaError::UnsupportedSignature)
}

/// Reads only a bounded prefix and verifies it against the reserved media class.
pub async fn detect_file(
    path: &std::path::Path,
    declared_kind: MediaKind,
) -> Result<MediaFormat, MediaError> {
    use tokio::io::AsyncReadExt as _;

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(MediaError::Input)?;
    let mut prefix = Vec::with_capacity(SIGNATURE_PREFIX_BYTES as usize);
    (&mut file)
        .take(SIGNATURE_PREFIX_BYTES)
        .read_to_end(&mut prefix)
        .await
        .map_err(MediaError::Input)?;
    let format = detect_media_format(&prefix)?;
    if format.kind() != declared_kind {
        return Err(MediaError::DeclaredKindMismatch);
    }
    Ok(format)
}

/// Strict FFprobe request for one trusted local file.
#[derive(Clone, Debug)]
pub struct VideoProbeRequest {
    /// Trusted local source path.
    pub input: PathBuf,
    /// Exact encoded source length already verified against object storage.
    pub byte_len: u64,
    /// Signature-detected video container family.
    pub format: MediaFormat,
}

/// Parsed video facts and codec identity returned by FFprobe.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VideoInspection {
    /// Normalized resource facts consumed by domain policy.
    pub probe: VideoProbe,
    /// Primary video codec selected by FFprobe.
    pub codec: String,
    /// Demuxer name returned by FFprobe.
    pub container: String,
}

/// Stable credential-free sandbox probe response consumed by the worker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaInspection {
    /// Valid image header facts, before full derivative decode.
    Image {
        /// Signature-detected encoded format.
        format: MediaFormat,
        /// Normalized detected content type.
        content_type: String,
        /// libvips header facts.
        probe: ImageProbe,
    },
    /// Valid video demuxer and stream facts.
    Video {
        /// Signature-detected container family.
        format: MediaFormat,
        /// Normalized detected content type.
        content_type: String,
        /// FFprobe facts and primary codec.
        inspection: VideoInspection,
    },
}

/// Bounded FFprobe process wrapper.
#[derive(Clone, Debug)]
pub struct FfprobeInspector {
    binary: PathBuf,
    timeout: Duration,
}

impl FfprobeInspector {
    /// Creates an inspector with an explicit executable and wall-clock limit.
    pub fn new(binary: PathBuf, timeout: Duration) -> Result<Self, MediaError> {
        if timeout.is_zero() {
            return Err(MediaError::InvalidRequest(
                "FFprobe timeout must be non-zero".to_owned(),
            ));
        }
        Ok(Self { binary, timeout })
    }

    /// Probes only selected stream fields with network protocols disabled.
    pub async fn inspect(
        &self,
        request: &VideoProbeRequest,
    ) -> Result<VideoInspection, MediaError> {
        if !matches!(
            request.format,
            MediaFormat::Mp4 | MediaFormat::QuickTime | MediaFormat::Webm
        ) {
            return Err(MediaError::InvalidRequest(
                "FFprobe input is not a supported video signature".to_owned(),
            ));
        }
        let output = timeout(
            self.timeout,
            Command::new(&self.binary)
                .args([
                    "-v",
                    "error",
                    "-protocol_whitelist",
                    "file,crypto,data",
                    "-show_entries",
                    "format=format_name,duration:stream=codec_type,codec_name,width,height,duration",
                    "-of",
                    "json",
                    "-i",
                ])
                .arg(&request.input)
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| MediaError::Timeout("ffprobe".to_owned()))?
        .map_err(|error| MediaError::Spawn {
            tool: "ffprobe".to_owned(),
            source: error,
        })?;
        if !output.status.success() {
            return Err(MediaError::ExitStatus {
                tool: "ffprobe".to_owned(),
                status: output.status.to_string(),
            });
        }
        if output.stdout.len() > MAX_PROBE_OUTPUT_BYTES
            || output.stderr.len() > MAX_PROBE_OUTPUT_BYTES
        {
            return Err(MediaError::ProbeOutputTooLarge);
        }
        let parsed: RawFfprobeOutput =
            serde_json::from_slice(&output.stdout).map_err(MediaError::ProbeJson)?;
        video_inspection_from_raw(request, parsed)
    }
}

#[derive(Debug, Deserialize)]
struct RawFfprobeOutput {
    #[serde(default)]
    streams: Vec<RawFfprobeStream>,
    format: Option<RawFfprobeFormat>,
}

#[derive(Debug, Deserialize)]
struct RawFfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawFfprobeFormat {
    format_name: Option<String>,
    duration: Option<String>,
}

fn video_inspection_from_raw(
    request: &VideoProbeRequest,
    raw: RawFfprobeOutput,
) -> Result<VideoInspection, MediaError> {
    let format = raw.format.ok_or(MediaError::InvalidProbe)?;
    let container = format.format_name.ok_or(MediaError::InvalidProbe)?;
    let container_matches = match request.format {
        MediaFormat::Mp4 | MediaFormat::QuickTime => container
            .split(',')
            .any(|name| matches!(name, "mov" | "mp4")),
        MediaFormat::Webm => container.split(',').any(|name| name == "webm"),
        _ => false,
    };
    if !container_matches {
        return Err(MediaError::ContainerMismatch);
    }
    let total_streams = u32::try_from(raw.streams.len()).map_err(|_| MediaError::InvalidProbe)?;
    let video_streams = raw
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("video"))
        .count();
    let video_streams = u32::try_from(video_streams).map_err(|_| MediaError::InvalidProbe)?;
    let primary = raw
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or(MediaError::InvalidProbe)?;
    let codec = primary.codec_name.clone().ok_or(MediaError::InvalidProbe)?;
    if !codec_allowed(request.format, &codec) {
        return Err(MediaError::UnsupportedCodec(codec));
    }
    let duration = format
        .duration
        .as_deref()
        .or(primary.duration.as_deref())
        .ok_or(MediaError::InvalidProbe)?;
    let duration_ms = parse_duration_ms(duration)?;
    Ok(VideoInspection {
        probe: VideoProbe {
            byte_len: request.byte_len,
            duration_ms,
            width: primary.width.ok_or(MediaError::InvalidProbe)?,
            height: primary.height.ok_or(MediaError::InvalidProbe)?,
            video_streams,
            total_streams,
        },
        codec,
        container,
    })
}

fn parse_duration_ms(value: &str) -> Result<u64, MediaError> {
    let (seconds, fraction) = value.split_once('.').unwrap_or((value, ""));
    if seconds.is_empty()
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(MediaError::InvalidProbe);
    }
    let seconds = seconds
        .parse::<u64>()
        .map_err(|_| MediaError::InvalidProbe)?;
    let mut millis = 0_u64;
    for (index, byte) in fraction.bytes().take(3).enumerate() {
        let digit = u64::from(byte - b'0');
        millis +=
            digit * 10_u64.pow(u32::try_from(2 - index).map_err(|_| MediaError::InvalidProbe)?);
    }
    if fraction
        .as_bytes()
        .get(3..)
        .is_some_and(|tail| tail.iter().any(|byte| *byte != b'0'))
    {
        millis = millis.checked_add(1).ok_or(MediaError::InvalidProbe)?;
    }
    seconds
        .checked_mul(1000)
        .and_then(|base| base.checked_add(millis))
        .filter(|duration| *duration > 0)
        .ok_or(MediaError::InvalidProbe)
}

fn codec_allowed(format: MediaFormat, codec: &str) -> bool {
    match format {
        MediaFormat::Mp4 | MediaFormat::QuickTime => {
            matches!(codec, "h264" | "hevc" | "av1" | "vp9" | "mjpeg" | "prores")
        }
        MediaFormat::Webm => matches!(codec, "vp8" | "vp9" | "av1"),
        _ => false,
    }
}

/// Version and warning output for one native executable.
#[derive(Clone, Debug, Serialize)]
pub struct NativeToolReport {
    /// Executable name.
    pub tool: String,
    /// First stdout line.
    pub version: String,
    /// Whether startup produced a loader warning.
    pub healthy: bool,
    /// Sanitized startup warning, if any.
    pub warning: Option<String>,
}

/// Probes required native executables without treating a zero status as sufficient health.
pub async fn native_tool_report() -> Result<Vec<NativeToolReport>, MediaError> {
    let vips = probe_tool("vips", &["--version"]).await?;
    let ffmpeg = probe_tool("ffmpeg", &["-version"]).await?;
    let ffprobe = probe_tool("ffprobe", &["-version"]).await?;
    Ok(vec![vips, ffmpeg, ffprobe])
}

async fn probe_tool(tool: &str, args: &[&str]) -> Result<NativeToolReport, MediaError> {
    let output = timeout(
        Duration::from_secs(5),
        Command::new(tool).args(args).stdin(Stdio::null()).output(),
    )
    .await
    .map_err(|_| MediaError::Timeout(tool.to_owned()))?
    .map_err(|error| MediaError::Spawn {
        tool: tool.to_owned(),
        source: error,
    })?;
    if !output.status.success() {
        return Err(MediaError::ExitStatus {
            tool: tool.to_owned(),
            status: output.status.to_string(),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let warning = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::to_owned);
    let healthy = !stderr.contains("unable to load") && !stderr.contains("Library not loaded");
    Ok(NativeToolReport {
        tool: tool.to_owned(),
        version: stdout.lines().next().unwrap_or_default().to_owned(),
        healthy,
        warning,
    })
}

/// Strict MP4 thumbnail extraction request.
#[derive(Clone, Debug)]
pub struct VideoThumbnailRequest {
    /// Trusted local MP4 input path.
    pub input: PathBuf,
    /// Trusted local JPEG output path.
    pub output: PathBuf,
    /// Seek time in milliseconds after trusted FFprobe validation.
    pub timestamp_ms: u64,
    /// Trusted total duration used to select one bounded fallback timestamp.
    pub duration_ms: u64,
    /// Maximum output width without upscaling.
    pub max_width: u32,
}

/// FFmpeg process wrapper with timeout and kill-on-timeout behavior.
#[derive(Clone, Debug)]
pub struct FfmpegThumbnailer {
    binary: PathBuf,
    timeout: Duration,
    threads: u32,
}

impl FfmpegThumbnailer {
    /// Creates a bounded FFmpeg wrapper.
    pub fn new(binary: PathBuf, timeout: Duration, threads: u32) -> Result<Self, MediaError> {
        if timeout < Duration::from_secs(2) || !(1..=16).contains(&threads) {
            return Err(MediaError::InvalidRequest(
                "timeout must be at least two seconds and thread count must be between 1 and 16"
                    .to_owned(),
            ));
        }
        Ok(Self {
            binary,
            timeout,
            threads,
        })
    }

    /// Extracts one JPEG without invoking a shell or allowing network protocols.
    pub async fn extract(&self, request: &VideoThumbnailRequest) -> Result<(), MediaError> {
        if !(1..=4096).contains(&request.max_width)
            || request.duration_ms == 0
            || request.timestamp_ms >= request.duration_ms
        {
            return Err(MediaError::InvalidRequest(
                "thumbnail width, duration, or timestamp is outside its safe range".to_owned(),
            ));
        }
        let attempt_timeout = self.timeout / 2;
        let first = self
            .extract_attempt(
                request,
                request.timestamp_ms,
                FfmpegSeekMode::Fast,
                attempt_timeout,
            )
            .await;
        if first.is_ok() {
            return Ok(());
        }
        let _remove_result = tokio::fs::remove_file(&request.output).await;
        let fallback_timestamp = (request.duration_ms / 2).min(request.duration_ms - 1);
        self.extract_attempt(
            request,
            fallback_timestamp,
            FfmpegSeekMode::Exact,
            attempt_timeout,
        )
        .await
    }

    async fn extract_attempt(
        &self,
        request: &VideoThumbnailRequest,
        timestamp_ms: u64,
        seek_mode: FfmpegSeekMode,
        attempt_timeout: Duration,
    ) -> Result<(), MediaError> {
        let seek_seconds = format!("{:.3}", timestamp_ms as f64 / 1000.0);
        let scale = format!("scale=w='min({},iw)':h=-2:flags=lanczos", request.max_width);
        let threads = self.threads.to_string();
        let mut command = Command::new(&self.binary);
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file,crypto,data",
            "-threads",
            &threads,
        ]);
        if seek_mode == FfmpegSeekMode::Fast {
            command.args(["-ss", &seek_seconds]);
        }
        command.arg("-i").arg(&request.input);
        if seek_mode == FfmpegSeekMode::Exact {
            command.args(["-ss", &seek_seconds]);
        }
        let mut child = command
            .args([
                "-map",
                "0:v:0",
                "-frames:v",
                "1",
                "-an",
                "-sn",
                "-dn",
                "-vf",
                &scale,
                "-c:v",
                "mjpeg",
                "-q:v",
                "3",
                "-threads",
                &threads,
                "-y",
            ])
            .arg(&request.output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| MediaError::Spawn {
                tool: "ffmpeg".to_owned(),
                source: error,
            })?;

        let status = match timeout(attempt_timeout, child.wait()).await {
            Ok(result) => result.map_err(|error| MediaError::Wait {
                tool: "ffmpeg".to_owned(),
                source: error,
            })?,
            Err(_) => {
                let _kill_result = child.kill().await;
                let _wait_result = child.wait().await;
                return Err(MediaError::Timeout("ffmpeg".to_owned()));
            }
        };
        if !status.success() {
            return Err(MediaError::ExitStatus {
                tool: "ffmpeg".to_owned(),
                status: status.to_string(),
            });
        }
        let metadata = tokio::fs::metadata(&request.output)
            .await
            .map_err(MediaError::Output)?;
        if metadata.len() == 0 {
            return Err(MediaError::InvalidOutput);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FfmpegSeekMode {
    Fast,
    Exact,
}

/// Native media execution failure.
#[derive(Debug, Error)]
pub enum MediaError {
    /// Request violates a hard resource bound.
    #[error("invalid media request: {0}")]
    InvalidRequest(String),
    /// Source prefix could not be read.
    #[error("failed to read media input: {0}")]
    Input(std::io::Error),
    /// File bytes do not match an enabled media format.
    #[error("media signature is unsupported")]
    UnsupportedSignature,
    /// Detected bytes disagree with the authenticated reservation class.
    #[error("detected media kind differs from the reservation")]
    DeclaredKindMismatch,
    /// FFprobe demuxer disagrees with the signature-detected container.
    #[error("probed container differs from the detected signature")]
    ContainerMismatch,
    /// Video codec is not enabled for the detected container.
    #[error("video codec is unsupported: {0}")]
    UnsupportedCodec(String),
    /// Native probe output exceeded its narrow response budget.
    #[error("native probe output exceeded its byte limit")]
    ProbeOutputTooLarge,
    /// FFprobe returned malformed JSON.
    #[error("native probe returned malformed JSON: {0}")]
    ProbeJson(serde_json::Error),
    /// Native probe omitted or returned invalid required facts.
    #[error("native probe returned invalid media facts")]
    InvalidProbe,
    /// Executable could not start.
    #[error("failed to start {tool}: {source}")]
    Spawn {
        /// Sanitized executable label.
        tool: String,
        /// Operating-system error.
        source: std::io::Error,
    },
    /// Child wait failed.
    #[error("failed while waiting for {tool}: {source}")]
    Wait {
        /// Sanitized executable label.
        tool: String,
        /// Operating-system error.
        source: std::io::Error,
    },
    /// Child exceeded its wall-clock budget.
    #[error("{0} exceeded its timeout")]
    Timeout(String),
    /// Child exited unsuccessfully.
    #[error("{tool} exited unsuccessfully: {status}")]
    ExitStatus {
        /// Sanitized executable label.
        tool: String,
        /// Portable status text.
        status: String,
    },
    /// Output metadata could not be read.
    #[error("failed to inspect media output: {0}")]
    Output(std::io::Error),
    /// Child succeeded without a usable output.
    #[error("media command produced an empty output")]
    InvalidOutput,
    /// MP4 or codec shape is outside the intentionally narrow OpenH264 fallback.
    #[cfg(feature = "openh264-fallback")]
    #[error("media is unsupported by the OpenH264 MP4 fallback")]
    FallbackUnsupported,
    /// Fallback input exceeded a sample, byte, dimension, or pixel budget.
    #[cfg(feature = "openh264-fallback")]
    #[error("OpenH264 MP4 fallback resource limit exceeded")]
    FallbackLimit,
    /// MP4 sample framing or H.264 decoding failed inside the fallback.
    #[cfg(feature = "openh264-fallback")]
    #[error("OpenH264 MP4 fallback rejected the bitstream")]
    FallbackDecode,
    /// libvips binding failed inside the sandbox.
    #[cfg(feature = "native-vips")]
    #[error("libvips operation failed: {0}")]
    Vips(String),
    /// A path cannot be represented for the C API.
    #[cfg(feature = "native-vips")]
    #[error("native media path is not valid UTF-8")]
    NonUtf8Path,
}

/// Integer placement calculated before any native watermark operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatermarkPlacement {
    /// Overlay x coordinate.
    pub x: u32,
    /// Overlay y coordinate.
    pub y: u32,
    /// Bounded rendered watermark width.
    pub width: u32,
    /// Bounded rendered watermark height.
    pub height: u32,
}

/// Validates a server-controlled watermark preset and calculates a bounded placement.
pub fn plan_watermark(
    base_width: u32,
    base_height: u32,
    watermark_width: u32,
    watermark_height: u32,
    spec: &WatermarkSpec<'_>,
) -> Result<WatermarkPlacement, MediaError> {
    if base_width == 0
        || base_height == 0
        || watermark_width == 0
        || watermark_height == 0
        || !(1..=50).contains(&spec.max_width_percent)
        || !(1..=100).contains(&spec.opacity_percent)
        || spec.margin_px > 1024
    {
        return Err(MediaError::InvalidRequest(
            "watermark dimensions or preset limits are invalid".to_owned(),
        ));
    }
    let doubled_margin = spec
        .margin_px
        .checked_mul(2)
        .ok_or_else(|| MediaError::InvalidRequest("watermark margin overflow".to_owned()))?;
    let available_width = base_width
        .checked_sub(doubled_margin)
        .filter(|value| *value > 0);
    let available_height = base_height
        .checked_sub(doubled_margin)
        .filter(|value| *value > 0);
    let (available_width, available_height) = match (available_width, available_height) {
        (Some(width), Some(height)) => (width, height),
        _ => {
            return Err(MediaError::InvalidRequest(
                "watermark margin leaves no drawable area".to_owned(),
            ));
        }
    };
    let percent_cap = (u64::from(base_width) * u64::from(spec.max_width_percent) / 100).max(1);
    let height_cap =
        u64::from(watermark_width) * u64::from(available_height) / u64::from(watermark_height);
    let width = [
        u64::from(watermark_width),
        u64::from(available_width),
        percent_cap,
        height_cap,
    ]
    .into_iter()
    .min()
    .filter(|value| *value > 0)
    .and_then(|value| u32::try_from(value).ok())
    .ok_or_else(|| MediaError::InvalidRequest("watermark cannot fit the derivative".to_owned()))?;
    let height =
        (u64::from(watermark_height) * u64::from(width) / u64::from(watermark_width)).max(1);
    let height = u32::try_from(height)
        .map_err(|_| MediaError::InvalidRequest("watermark height overflow".to_owned()))?;
    if height > available_height {
        return Err(MediaError::InvalidRequest(
            "watermark cannot fit the derivative".to_owned(),
        ));
    }

    let right = base_width - width - spec.margin_px;
    let bottom = base_height - height - spec.margin_px;
    let (x, y) = match spec.position {
        WatermarkPosition::Center => ((base_width - width) / 2, (base_height - height) / 2),
        WatermarkPosition::TopLeft => (spec.margin_px, spec.margin_px),
        WatermarkPosition::TopRight => (right, spec.margin_px),
        WatermarkPosition::BottomLeft => (spec.margin_px, bottom),
        WatermarkPosition::BottomRight => (right, bottom),
    };
    Ok(WatermarkPlacement {
        x,
        y,
        width,
        height,
    })
}

/// In-process libvips engine intended only for the sandbox process.
#[cfg(feature = "native-vips")]
pub mod vips {
    use g7mb_application::{ImageOutputFormat, ImageThumbnailRequest, WatermarkSpec};
    use g7mb_domain::ImageProbe;
    use libvips::{VipsApp, VipsImage, ops};

    use super::{MediaError, plan_watermark};

    /// Owns the process-global libvips lifecycle.
    pub struct VipsEngine {
        app: VipsApp,
    }

    impl std::fmt::Debug for VipsEngine {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.debug_struct("VipsEngine").finish_non_exhaustive()
        }
    }

    impl VipsEngine {
        /// Initializes libvips once and fixes its internal thread budget.
        pub fn new(threads: i32) -> Result<Self, MediaError> {
            if threads <= 0 {
                return Err(MediaError::InvalidRequest(
                    "libvips thread count must be positive".to_owned(),
                ));
            }
            let app = VipsApp::new("g7mb-sandbox", false)
                .map_err(|error| MediaError::Vips(format!("initialization: {error:?}")))?;
            app.concurrency_set(threads);
            app.cache_set_max(100);
            app.cache_set_max_mem(128 * 1024 * 1024);
            app.cache_set_max_files(32);
            Ok(Self { app })
        }

        /// Returns the linked libvips version.
        pub fn version(&self) -> Result<&str, MediaError> {
            self.app
                .version_string()
                .map_err(|error| MediaError::Vips(format!("version: {error:?}")))
        }

        /// Reads lazy decoder header facts before any full-pixel transform.
        pub fn probe(
            &self,
            input: &std::path::Path,
            byte_len: u64,
        ) -> Result<ImageProbe, MediaError> {
            let input = input.to_str().ok_or(MediaError::NonUtf8Path)?;
            let image = VipsImage::new_from_file_access(input, ops::Access::Sequential, false)
                .map_err(|error| MediaError::Vips(format!("probe: {error:?}")))?;
            let width = u32::try_from(image.get_width()).map_err(|_| MediaError::InvalidProbe)?;
            let full_height = image.get_height();
            let page_height = image.get_page_height();
            let height = u32::try_from(if page_height > 0 {
                page_height
            } else {
                full_height
            })
            .map_err(|_| MediaError::InvalidProbe)?;
            let frames =
                u32::try_from(image.get_n_pages().max(1)).map_err(|_| MediaError::InvalidProbe)?;
            Ok(ImageProbe {
                byte_len,
                width,
                height,
                frames,
            })
        }

        /// Produces one metadata-stripped derivative with no upscaling.
        pub fn thumbnail(&self, request: &ImageThumbnailRequest<'_>) -> Result<(), MediaError> {
            let input = request.input.to_str().ok_or(MediaError::NonUtf8Path)?;
            let output = request.output.to_str().ok_or(MediaError::NonUtf8Path)?;
            let max_edge = i32::try_from(request.max_edge).map_err(|_| {
                MediaError::InvalidRequest("image edge exceeds native range".to_owned())
            })?;
            if !(1..=8192).contains(&max_edge) {
                return Err(MediaError::InvalidRequest(
                    "image edge must be between 1 and 8192".to_owned(),
                ));
            }
            // The generated `thumbnail_with_opts` binding passes every optional
            // property, including profile names introduced after libvips 8.15.
            // Use the stable base operation and calculate a downscale-only width
            // ourselves so the supported 8.15+ runtime range remains real.
            let header = VipsImage::new_from_file_access(input, ops::Access::Sequential, false)
                .map_err(|error| MediaError::Vips(format!("thumbnail header: {error:?}")))?;
            let source_width =
                u64::try_from(header.get_width()).map_err(|_| MediaError::InvalidProbe)?;
            let full_height = header.get_height();
            let page_height = header.get_page_height();
            let source_height = u64::try_from(if page_height > 0 {
                page_height
            } else {
                full_height
            })
            .map_err(|_| MediaError::InvalidProbe)?;
            if source_width == 0 || source_height == 0 {
                return Err(MediaError::InvalidProbe);
            }
            let source_max_edge = source_width.max(source_height);
            let target_width = if source_max_edge <= u64::from(request.max_edge) {
                source_width
            } else {
                (source_width * u64::from(request.max_edge) / source_max_edge).max(1)
            };
            let target_width = i32::try_from(target_width).map_err(|_| {
                MediaError::InvalidRequest("image width exceeds native range".to_owned())
            })?;
            drop(header);

            // `fail-on=error` is a loader option available throughout the
            // supported range and makes the eventual encode fully decode or
            // reject corrupt/truncated content.
            let decoder_input = format!("{input}[fail-on=error]");
            let image = ops::thumbnail(&decoder_input, target_width)
                .map_err(|error| MediaError::Vips(format!("thumbnail: {error:?}")))?;
            let rendered_max_edge = image.get_width().max(image.get_height());
            let image = if rendered_max_edge > max_edge {
                ops::resize(&image, f64::from(max_edge) / f64::from(rendered_max_edge))
                    .map_err(|error| MediaError::Vips(format!("thumbnail bound: {error:?}")))?
            } else {
                image
            };
            let image = ops::colourspace(&image, ops::Interpretation::Srgb)
                .map_err(|error| MediaError::Vips(format!("colourspace: {error:?}")))?;
            let image = match request.watermark.as_ref() {
                Some(spec) => apply_watermark(&image, spec)?,
                None => image,
            };
            let image = if request.format == ImageOutputFormat::Jpeg && image.get_bands() == 4 {
                ops::flatten_with_opts(
                    &image,
                    &ops::FlattenOptions {
                        background: vec![255.0, 255.0, 255.0],
                        ..ops::FlattenOptions::default()
                    },
                )
                .map_err(|error| MediaError::Vips(format!("flatten: {error:?}")))?
            } else {
                image
            };

            match request.format {
                ImageOutputFormat::Jpeg => ops::jpegsave_with_opts(
                    &image,
                    output,
                    &ops::JpegsaveOptions {
                        q: 82,
                        optimize_coding: true,
                        interlace: true,
                        keep: ops::ForeignKeep::None,
                        profile: None,
                        ..ops::JpegsaveOptions::default()
                    },
                ),
                // The generated 8.18 bindings pass every known optional
                // property. libvips 8.15 rejects that call when newer WebP or
                // HEIF properties are absent. Filename options let libvips
                // parse only this stable subset while preserving streaming
                // file output and metadata stripping.
                ImageOutputFormat::Webp => image.image_write_to_file(&filename_with_options(
                    request.output,
                    "webp",
                    "Q=80,smart-subsample,effort=4,strip",
                )?),
                ImageOutputFormat::Avif => image.image_write_to_file(&filename_with_options(
                    request.output,
                    "avif",
                    "Q=50,bitdepth=8,compression=av1,effort=4,strip",
                )?),
                ImageOutputFormat::Png => ops::pngsave_with_opts(
                    &image,
                    output,
                    &ops::PngsaveOptions {
                        compression: 6,
                        keep: ops::ForeignKeep::None,
                        profile: None,
                        ..ops::PngsaveOptions::default()
                    },
                ),
            }
            .map_err(|error| MediaError::Vips(format!("encode: {error:?}")))
        }
    }

    fn filename_with_options(
        output: &std::path::Path,
        expected_extension: &str,
        options: &str,
    ) -> Result<String, MediaError> {
        let output_extension = output
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or_else(|| {
                MediaError::InvalidRequest("image output extension is required".to_owned())
            })?;
        if !output_extension.eq_ignore_ascii_case(expected_extension) {
            return Err(MediaError::InvalidRequest(format!(
                "image output extension must be .{expected_extension}"
            )));
        }
        let output = output.to_str().ok_or(MediaError::NonUtf8Path)?;
        if output.contains(['[', ']']) {
            return Err(MediaError::InvalidRequest(
                "image output path contains libvips option delimiters".to_owned(),
            ));
        }
        Ok(format!("{output}[{options}]"))
    }

    fn apply_watermark(
        base: &VipsImage,
        spec: &WatermarkSpec<'_>,
    ) -> Result<VipsImage, MediaError> {
        let input = spec.input.to_str().ok_or(MediaError::NonUtf8Path)?;
        let watermark = VipsImage::new_from_file_access(input, ops::Access::Sequential, false)
            .map_err(|error| MediaError::Vips(format!("watermark decode: {error:?}")))?;
        let watermark_width = u32::try_from(watermark.get_width())
            .map_err(|_| MediaError::InvalidRequest("watermark width is invalid".to_owned()))?;
        let watermark_height = u32::try_from(watermark.get_height())
            .map_err(|_| MediaError::InvalidRequest("watermark height is invalid".to_owned()))?;
        if watermark_width > 4096
            || watermark_height > 4096
            || u64::from(watermark_width) * u64::from(watermark_height) > 16_777_216
        {
            return Err(MediaError::InvalidRequest(
                "watermark exceeds the 4096px or 16MP asset limit".to_owned(),
            ));
        }
        let base_width = u32::try_from(base.get_width())
            .map_err(|_| MediaError::InvalidRequest("base width is invalid".to_owned()))?;
        let base_height = u32::try_from(base.get_height())
            .map_err(|_| MediaError::InvalidRequest("base height is invalid".to_owned()))?;
        let placement = plan_watermark(
            base_width,
            base_height,
            watermark_width,
            watermark_height,
            spec,
        )?;
        let watermark = ops::colourspace(&watermark, ops::Interpretation::Srgb)
            .map_err(|error| MediaError::Vips(format!("watermark colourspace: {error:?}")))?;
        let watermark = if placement.width < watermark_width {
            ops::resize(
                &watermark,
                f64::from(placement.width) / f64::from(watermark_width),
            )
            .map_err(|error| MediaError::Vips(format!("watermark resize: {error:?}")))?
        } else {
            watermark
        };
        let watermark = match watermark.get_bands() {
            3 => ops::addalpha(&watermark)
                .map_err(|error| MediaError::Vips(format!("watermark alpha: {error:?}")))?,
            4 => watermark,
            _ => {
                return Err(MediaError::InvalidRequest(
                    "watermark must decode to sRGB or sRGBA".to_owned(),
                ));
            }
        };
        let mut multiply = vec![1.0, 1.0, 1.0, f64::from(spec.opacity_percent) / 100.0];
        let mut add = vec![0.0; 4];
        let watermark = ops::linear_with_opts(
            &watermark,
            &mut multiply,
            &mut add,
            &ops::LinearOptions { uchar: true },
        )
        .map_err(|error| MediaError::Vips(format!("watermark opacity: {error:?}")))?;
        let x = i32::try_from(placement.x)
            .map_err(|_| MediaError::InvalidRequest("watermark x overflow".to_owned()))?;
        let y = i32::try_from(placement.y)
            .map_err(|_| MediaError::InvalidRequest("watermark y overflow".to_owned()))?;
        ops::composite2_with_opts(
            base,
            &watermark,
            ops::BlendMode::Over,
            &ops::Composite2Options {
                x,
                y,
                compositing_space: ops::Interpretation::Srgb,
                premultiplied: false,
            },
        )
        .map_err(|error| MediaError::Vips(format!("watermark composite: {error:?}")))
    }

    #[cfg(test)]
    mod tests {
        use std::path::Path;

        use super::filename_with_options;

        #[test]
        fn filename_options_require_matching_safe_output_paths() -> Result<(), super::MediaError> {
            let encoded = filename_with_options(
                Path::new("/private/job/thumbnail.webp"),
                "webp",
                "Q=80,strip",
            )?;
            assert_eq!(encoded, "/private/job/thumbnail.webp[Q=80,strip]");
            assert!(filename_with_options(Path::new("thumbnail.jpg"), "webp", "strip").is_err());
            assert!(
                filename_with_options(Path::new("job[unsafe]/thumbnail.webp"), "webp", "strip")
                    .is_err()
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        time::Duration,
    };

    use g7mb_application::{WatermarkPosition, WatermarkSpec};
    use g7mb_domain::{MediaKind, VideoProbe};

    use super::{
        FfmpegThumbnailer, FfprobeInspector, MediaError, MediaFormat, RawFfprobeOutput,
        VideoProbeRequest, VideoThumbnailRequest, codec_allowed, detect_file, detect_media_format,
        parse_duration_ms, plan_watermark, video_inspection_from_raw,
    };

    #[test]
    fn watermark_planner_bounds_scale_margin_and_anchor() -> Result<(), MediaError> {
        let spec = WatermarkSpec {
            input: Path::new("registered-watermark.png"),
            position: WatermarkPosition::BottomRight,
            margin_px: 24,
            max_width_percent: 20,
            opacity_percent: 75,
        };
        let placement = plan_watermark(1280, 720, 1000, 200, &spec)?;
        assert_eq!(placement.width, 256);
        assert_eq!(placement.height, 51);
        assert_eq!(placement.x, 1000);
        assert_eq!(placement.y, 645);
        Ok(())
    }

    #[test]
    fn watermark_planner_rejects_unbounded_or_impossible_presets() {
        let mut spec = WatermarkSpec {
            input: Path::new("registered-watermark.png"),
            position: WatermarkPosition::Center,
            margin_px: 0,
            max_width_percent: 51,
            opacity_percent: 100,
        };
        assert!(matches!(
            plan_watermark(100, 100, 20, 20, &spec),
            Err(MediaError::InvalidRequest(_))
        ));
        spec.max_width_percent = 20;
        spec.margin_px = 50;
        assert!(matches!(
            plan_watermark(100, 100, 20, 20, &spec),
            Err(MediaError::InvalidRequest(_))
        ));
    }

    #[test]
    fn rejects_zero_native_thread_budget() {
        let result = FfmpegThumbnailer::new(PathBuf::from("ffmpeg"), Duration::from_secs(30), 0);
        assert!(matches!(result, Err(MediaError::InvalidRequest(_))));
    }

    #[test]
    fn detects_supported_image_and_video_signatures() -> Result<(), MediaError> {
        assert_eq!(
            detect_media_format(b"\xff\xd8\xff\xe0jpeg")?,
            MediaFormat::Jpeg
        );
        assert_eq!(
            detect_media_format(b"\x89PNG\r\n\x1a\nrest")?,
            MediaFormat::Png
        );
        assert_eq!(detect_media_format(b"GIF87arest")?, MediaFormat::Gif);
        assert_eq!(detect_media_format(b"GIF89arest")?, MediaFormat::Gif);
        assert_eq!(
            detect_media_format(b"RIFF\x04\x00\x00\x00WEBP")?,
            MediaFormat::Webp
        );
        assert_eq!(detect_media_format(&ftyp(b"avif"))?, MediaFormat::Avif);
        assert_eq!(detect_media_format(&ftyp(b"heic"))?, MediaFormat::Heif);
        assert_eq!(detect_media_format(&ftyp(b"isom"))?, MediaFormat::Mp4);
        assert_eq!(detect_media_format(&ftyp(b"qt  "))?, MediaFormat::QuickTime);
        assert_eq!(
            detect_media_format(b"\x1a\x45\xdf\xa3xxxxwebm")?,
            MediaFormat::Webm
        );
        assert!(matches!(
            detect_media_format(b"<?php echo 1;"),
            Err(MediaError::UnsupportedSignature)
        ));
        Ok(())
    }

    #[test]
    fn normalized_formats_keep_kind_and_content_type_consistent() {
        let cases = [
            (MediaFormat::Jpeg, MediaKind::Image, "image/jpeg"),
            (MediaFormat::Png, MediaKind::Image, "image/png"),
            (MediaFormat::Gif, MediaKind::Image, "image/gif"),
            (MediaFormat::Webp, MediaKind::Image, "image/webp"),
            (MediaFormat::Avif, MediaKind::Image, "image/avif"),
            (MediaFormat::Heif, MediaKind::Image, "image/heif"),
            (MediaFormat::Mp4, MediaKind::Video, "video/mp4"),
            (MediaFormat::QuickTime, MediaKind::Video, "video/quicktime"),
            (MediaFormat::Webm, MediaKind::Video, "video/webm"),
        ];
        for (format, kind, content_type) in cases {
            assert_eq!(format.kind(), kind);
            assert_eq!(format.content_type(), content_type);
        }
    }

    #[test]
    fn decoder_aware_limits_keep_avif_and_heif_inside_the_worker_memory_budget() {
        let avif_limits = super::image_limits_for_format(MediaFormat::Avif);
        let heif_limits = super::image_limits_for_format(MediaFormat::Heif);
        let jpeg_limits = super::image_limits_for_format(MediaFormat::Jpeg);
        assert_eq!(avif_limits.max_pixels, 64_000_000);
        assert_eq!(heif_limits.max_pixels, 64_000_000);
        assert_eq!(jpeg_limits.max_pixels, 200_000_000);
    }

    #[test]
    fn rejects_malformed_iso_bmff_signatures() {
        assert!(matches!(
            detect_media_format(b"\0\0\0\x08ftyp"),
            Err(MediaError::UnsupportedSignature)
        ));
        assert!(matches!(
            detect_media_format(b"\0\0\0\x0dftypabcde"),
            Err(MediaError::UnsupportedSignature)
        ));
        assert!(matches!(
            detect_media_format(&ftyp(b"zzzz")),
            Err(MediaError::UnsupportedSignature)
        ));
    }

    #[tokio::test]
    async fn rejects_signature_that_disagrees_with_reserved_kind()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), b"\xff\xd8\xff\xe0jpeg")?;
        let result = detect_file(file.path(), MediaKind::Video).await;
        assert!(matches!(result, Err(MediaError::DeclaredKindMismatch)));
        Ok(())
    }

    #[tokio::test]
    async fn detects_file_and_reports_missing_input() -> Result<(), Box<dyn std::error::Error>> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), b"\x89PNG\r\n\x1a\nrest")?;
        assert_eq!(
            detect_file(file.path(), MediaKind::Image).await?,
            MediaFormat::Png
        );
        let missing = file.path().with_extension("missing");
        assert!(matches!(
            detect_file(&missing, MediaKind::Image).await,
            Err(MediaError::Input(_))
        ));
        Ok(())
    }

    #[test]
    fn parses_only_bounded_video_facts() -> Result<(), Box<dyn std::error::Error>> {
        let raw: RawFfprobeOutput = serde_json::from_value(serde_json::json!({
            "streams": [
                {"codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080},
                {"codec_type": "audio", "codec_name": "aac"}
            ],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.345"}
        }))?;
        let inspection = video_inspection_from_raw(
            &VideoProbeRequest {
                input: PathBuf::from("trusted.mp4"),
                byte_len: 1024,
                format: MediaFormat::Mp4,
            },
            raw,
        )?;
        assert_eq!(inspection.codec, "h264");
        assert_eq!(
            inspection.probe,
            VideoProbe {
                byte_len: 1024,
                duration_ms: 12_345,
                width: 1920,
                height: 1080,
                video_streams: 1,
                total_streams: 2,
            }
        );
        Ok(())
    }

    #[test]
    fn rejects_inconsistent_or_incomplete_video_probe_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = VideoProbeRequest {
            input: PathBuf::from("trusted.mp4"),
            byte_len: 1024,
            format: MediaFormat::Mp4,
        };
        for (value, expected) in [
            (
                serde_json::json!({"streams": [], "format": {"format_name": "webm", "duration": "1"}}),
                "container",
            ),
            (
                serde_json::json!({"streams": [], "format": {"format_name": "mp4", "duration": "1"}}),
                "probe",
            ),
            (
                serde_json::json!({"streams": [{"codec_type": "video", "codec_name": "theora", "width": 1, "height": 1}], "format": {"format_name": "mp4", "duration": "1"}}),
                "codec",
            ),
            (
                serde_json::json!({"streams": [{"codec_type": "video", "codec_name": "h264", "width": 1, "height": 1}], "format": {"format_name": "mp4", "duration": "zero"}}),
                "probe",
            ),
        ] {
            let raw: RawFfprobeOutput = serde_json::from_value(value)?;
            let error = match video_inspection_from_raw(&request, raw) {
                Ok(_) => return Err("inconsistent FFprobe facts were accepted".into()),
                Err(error) => error,
            };
            match expected {
                "container" => assert!(matches!(error, MediaError::ContainerMismatch)),
                "codec" => assert!(matches!(error, MediaError::UnsupportedCodec(_))),
                _ => assert!(matches!(error, MediaError::InvalidProbe)),
            }
        }
        let missing_format: RawFfprobeOutput = serde_json::from_value(serde_json::json!({}))?;
        assert!(matches!(
            video_inspection_from_raw(&request, missing_format),
            Err(MediaError::InvalidProbe)
        ));
        Ok(())
    }

    #[test]
    fn duration_parser_rounds_up_without_float_math() -> Result<(), MediaError> {
        assert_eq!(parse_duration_ms("1")?, 1000);
        assert_eq!(parse_duration_ms("1.2")?, 1200);
        assert_eq!(parse_duration_ms("1.0001")?, 1001);
        for invalid in ["", ".1", "0", "-1", "1.x", "18446744073709551616"] {
            assert!(matches!(
                parse_duration_ms(invalid),
                Err(MediaError::InvalidProbe)
            ));
        }
        assert!(codec_allowed(MediaFormat::QuickTime, "prores"));
        assert!(codec_allowed(MediaFormat::Webm, "vp8"));
        assert!(!codec_allowed(MediaFormat::Webm, "h264"));
        assert!(!codec_allowed(MediaFormat::Jpeg, "h264"));
        Ok(())
    }

    #[tokio::test]
    async fn ffprobe_adapter_enforces_process_and_protocol_contracts()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(matches!(
            FfprobeInspector::new(PathBuf::from("ffprobe"), Duration::ZERO),
            Err(MediaError::InvalidRequest(_))
        ));
        let directory = tempfile::tempdir()?;
        let inspector = FfprobeInspector::new(fixture("fake-ffprobe"), Duration::from_secs(5))?;
        let request = VideoProbeRequest {
            input: directory.path().join("trusted.webm"),
            byte_len: 55,
            format: MediaFormat::Webm,
        };
        let inspection = inspector.inspect(&request).await?;
        assert_eq!(inspection.codec, "vp9");
        assert_eq!(inspection.probe.duration_ms, 2500);

        let image_request = VideoProbeRequest {
            format: MediaFormat::Jpeg,
            ..request.clone()
        };
        assert!(matches!(
            inspector.inspect(&image_request).await,
            Err(MediaError::InvalidRequest(_))
        ));

        let failed_request = VideoProbeRequest {
            input: directory.path().join("failed.webm"),
            ..request.clone()
        };
        assert!(matches!(
            inspector.inspect(&failed_request).await,
            Err(MediaError::ExitStatus { .. })
        ));

        let malformed_request = VideoProbeRequest {
            input: directory.path().join("malformed.webm"),
            ..request.clone()
        };
        assert!(matches!(
            inspector.inspect(&malformed_request).await,
            Err(MediaError::ProbeJson(_))
        ));

        let missing = directory.path().join("missing-ffprobe");
        let inspector = FfprobeInspector::new(missing, Duration::from_secs(5))?;
        assert!(matches!(
            inspector.inspect(&request).await,
            Err(MediaError::Spawn { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn ffmpeg_thumbnail_adapter_enforces_output_contract()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let binary = fixture("fake-ffmpeg");
        let output = directory.path().join("thumbnail.jpg");
        let request = VideoThumbnailRequest {
            input: directory.path().join("source.mp4"),
            output: output.clone(),
            timestamp_ms: 1234,
            duration_ms: 10_000,
            max_width: 1280,
        };
        FfmpegThumbnailer::new(binary.clone(), Duration::from_secs(5), 2)?
            .extract(&request)
            .await?;
        assert!(std::fs::metadata(&output)?.len() > 0);

        let fallback_request = VideoThumbnailRequest {
            input: directory.path().join("source-fallback.mp4"),
            ..request.clone()
        };
        FfmpegThumbnailer::new(binary.clone(), Duration::from_secs(6), 1)?
            .extract(&fallback_request)
            .await?;
        assert!(std::fs::metadata(&output)?.len() > 0);

        let failed_request = VideoThumbnailRequest {
            input: directory.path().join("source-failed.mp4"),
            ..request.clone()
        };
        assert!(matches!(
            FfmpegThumbnailer::new(binary.clone(), Duration::from_secs(5), 1)?
                .extract(&failed_request)
                .await,
            Err(MediaError::ExitStatus { .. })
        ));

        let empty_request = VideoThumbnailRequest {
            input: directory.path().join("source-empty.mp4"),
            ..request.clone()
        };
        assert!(matches!(
            FfmpegThumbnailer::new(binary, Duration::from_secs(5), 1)?
                .extract(&empty_request)
                .await,
            Err(MediaError::InvalidOutput)
        ));

        let invalid_width = VideoThumbnailRequest {
            max_width: 0,
            ..request
        };
        let missing = directory.path().join("missing-ffmpeg");
        assert!(matches!(
            FfmpegThumbnailer::new(missing, Duration::from_secs(5), 1)?
                .extract(&invalid_width)
                .await,
            Err(MediaError::InvalidRequest(_))
        ));
        Ok(())
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
        let mut bytes = 16_u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(brand);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }
}
