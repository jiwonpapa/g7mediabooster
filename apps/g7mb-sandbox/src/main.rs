//! Credential-free native media sandbox entrypoint.

#[cfg(feature = "native-vips")]
use std::process::Stdio;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use anyhow::{Context as _, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(feature = "native-vips")]
use g7mb_application::ImageThumbnailRequest;
#[cfg(feature = "native-vips")]
use g7mb_application::WatermarkSpec;
use g7mb_application::{ImageOutputFormat, WatermarkPosition};
use g7mb_contracts::CapabilitiesResponse;
use g7mb_domain::{MediaKind, VideoLimits};
#[cfg(feature = "native-vips")]
use g7mb_media::image_limits_for_format;
#[cfg(feature = "native-vips")]
use g7mb_media::openh264_fallback::{OpenH264FallbackLimits, extract_first_frame_ppm};
use g7mb_media::{
    FfmpegThumbnailer, FfprobeInspector, MediaError, MediaInspection, VideoProbeRequest,
    VideoThumbnailRequest, detect_file,
};

#[derive(Debug, Parser)]
#[command(version, about = "Credential-free native media sandbox")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check native executable startup health.
    Doctor,
    /// Run bounded native fixtures and print the verified runtime capability JSON.
    Capabilities,
    /// Detect and validate one untrusted local source without credentials or network access.
    Probe {
        /// Trusted local input path supplied by the worker.
        #[arg(long)]
        input: PathBuf,
        /// Authenticated reservation media class.
        #[arg(long, value_enum)]
        declared_kind: MediaKindArg,
        /// Exact encoded byte length verified during download.
        #[arg(long)]
        byte_len: u64,
        /// Native probe wall-clock limit for video.
        #[arg(long, default_value_t = 15)]
        timeout_seconds: u64,
        /// Native decoder thread budget.
        #[arg(long, default_value_t = 1)]
        threads: i32,
    },
    /// Extract one JPEG frame from a trusted local MP4.
    VideoThumbnail {
        /// Trusted local input path.
        #[arg(long)]
        input: PathBuf,
        /// Trusted local output path.
        #[arg(long)]
        output: PathBuf,
        /// Seek timestamp in milliseconds.
        #[arg(long, default_value_t = 0)]
        timestamp_ms: u64,
        /// Trusted total duration from FFprobe.
        #[arg(long)]
        duration_ms: u64,
        /// Maximum output width.
        #[arg(long, default_value_t = 1280)]
        max_width: u32,
        /// Wall-clock timeout.
        #[arg(long, default_value_t = 45)]
        timeout_seconds: u64,
        /// FFmpeg thread budget.
        #[arg(long, default_value_t = 1)]
        threads: u32,
        /// Trusted FFmpeg executable path. Worker deployments use the fixed default.
        #[arg(long, default_value = "ffmpeg")]
        ffmpeg_bin: PathBuf,
        /// Allow the narrow MP4/H.264 first-frame fallback when FFmpeg cannot start.
        #[arg(long, default_value_t = false)]
        allow_openh264_fallback: bool,
        /// Trusted registered watermark asset path.
        #[arg(long)]
        watermark: Option<PathBuf>,
        /// Allowlisted watermark anchor.
        #[arg(long, value_enum, default_value = "bottom-right")]
        watermark_position: WatermarkPositionArg,
        /// Watermark safe edge margin.
        #[arg(long, default_value_t = 24)]
        watermark_margin_px: u32,
        /// Maximum watermark width as a percentage of the poster.
        #[arg(long, default_value_t = 20)]
        watermark_max_width_percent: u8,
        /// Watermark alpha multiplier percentage.
        #[arg(long, default_value_t = 80)]
        watermark_opacity_percent: u8,
    },
    /// Produce one image derivative with libvips.
    ImageThumbnail {
        /// Trusted local input path.
        #[arg(long)]
        input: PathBuf,
        /// Trusted local output path.
        #[arg(long)]
        output: PathBuf,
        /// Maximum output edge.
        #[arg(long, default_value_t = 1280)]
        max_edge: u32,
        /// Server-controlled output format.
        #[arg(long, value_enum)]
        format: ImageFormatArg,
        /// libvips internal thread budget.
        #[arg(long, default_value_t = 1)]
        threads: i32,
        /// Trusted registered watermark asset path.
        #[arg(long)]
        watermark: Option<PathBuf>,
        /// Allowlisted watermark anchor.
        #[arg(long, value_enum, default_value = "bottom-right")]
        watermark_position: WatermarkPositionArg,
        /// Watermark safe edge margin.
        #[arg(long, default_value_t = 24)]
        watermark_margin_px: u32,
        /// Maximum watermark width as a percentage of the derivative.
        #[arg(long, default_value_t = 20)]
        watermark_max_width_percent: u8,
        /// Watermark alpha multiplier percentage.
        #[arg(long, default_value_t = 80)]
        watermark_opacity_percent: u8,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ImageFormatArg {
    Jpeg,
    Webp,
    Avif,
    Png,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum MediaKindArg {
    Image,
    Video,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum WatermarkPositionArg {
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

struct ImageThumbnailJob {
    input: PathBuf,
    output: PathBuf,
    max_edge: u32,
    format: ImageOutputFormat,
    threads: i32,
    watermark: Option<PathBuf>,
    watermark_position: WatermarkPosition,
    watermark_margin_px: u32,
    watermark_max_width_percent: u8,
    watermark_opacity_percent: u8,
}

impl From<MediaKindArg> for MediaKind {
    fn from(value: MediaKindArg) -> Self {
        match value {
            MediaKindArg::Image => Self::Image,
            MediaKindArg::Video => Self::Video,
        }
    }
}

impl From<ImageFormatArg> for ImageOutputFormat {
    fn from(value: ImageFormatArg) -> Self {
        match value {
            ImageFormatArg::Jpeg => Self::Jpeg,
            ImageFormatArg::Webp => Self::Webp,
            ImageFormatArg::Avif => Self::Avif,
            ImageFormatArg::Png => Self::Png,
        }
    }
}

impl From<WatermarkPositionArg> for WatermarkPosition {
    fn from(value: WatermarkPositionArg) -> Self {
        match value {
            WatermarkPositionArg::Center => Self::Center,
            WatermarkPositionArg::TopLeft => Self::TopLeft,
            WatermarkPositionArg::TopRight => Self::TopRight,
            WatermarkPositionArg::BottomLeft => Self::BottomLeft,
            WatermarkPositionArg::BottomRight => Self::BottomRight,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    install_network_filter().context("failed to install sandbox network seccomp filter")?;
    g7mb_telemetry::init_tracing()?;
    match Cli::parse().command {
        Command::Doctor => doctor().await,
        Command::Capabilities => {
            let response = runtime_capabilities().await?;
            serde_json::to_writer(std::io::stdout().lock(), &response)
                .context("failed to write sandbox capability response")?;
            Ok(())
        }
        Command::Probe {
            input,
            declared_kind,
            byte_len,
            timeout_seconds,
            threads,
        } => {
            let inspection = probe(
                input,
                declared_kind.into(),
                byte_len,
                Duration::from_secs(timeout_seconds),
                threads,
            )
            .await?;
            serde_json::to_writer(std::io::stdout().lock(), &inspection)
                .context("failed to write sandbox probe response")?;
            Ok(())
        }
        Command::VideoThumbnail {
            input,
            output,
            timestamp_ms,
            duration_ms,
            max_width,
            timeout_seconds,
            threads,
            ffmpeg_bin,
            allow_openh264_fallback,
            watermark,
            watermark_position,
            watermark_margin_px,
            watermark_max_width_percent,
            watermark_opacity_percent,
        } => {
            let total_timeout = Duration::from_secs(timeout_seconds);
            let ffmpeg_timeout = if watermark.is_some() {
                if total_timeout < Duration::from_secs(4) {
                    bail!("watermarked video thumbnail timeout must be at least four seconds");
                }
                total_timeout / 2
            } else {
                total_timeout
            };
            let frame_output = watermark
                .as_ref()
                .map_or_else(|| output.clone(), |_| output.with_extension("frame.jpg"));
            if frame_output == input {
                bail!("video thumbnail input and intermediate output must differ");
            }
            let thumbnailer = FfmpegThumbnailer::new(ffmpeg_bin, ffmpeg_timeout, threads)?;
            let ffmpeg_result = thumbnailer
                .extract(&VideoThumbnailRequest {
                    input: input.clone(),
                    output: frame_output.clone(),
                    timestamp_ms,
                    duration_ms,
                    max_width,
                })
                .await;
            match ffmpeg_result {
                Ok(()) => {}
                Err(MediaError::Spawn { .. }) if allow_openh264_fallback => {
                    #[cfg(feature = "native-vips")]
                    {
                        let fallback_output = output.with_extension("fallback.ppm");
                        if fallback_output == input || fallback_output == output {
                            bail!("OpenH264 fallback paths must differ");
                        }
                        let _remove_frame_result = tokio::fs::remove_file(&frame_output).await;
                        let fallback_frame_result = extract_first_frame_ppm(
                            &input,
                            &fallback_output,
                            OpenH264FallbackLimits::default(),
                        );
                        let fallback_frame = match fallback_frame_result {
                            Ok(frame) => frame,
                            Err(error) => {
                                let _remove_result = tokio::fs::remove_file(&fallback_output).await;
                                return Err(error).context("OpenH264 first-frame fallback failed");
                            }
                        };
                        tracing::info!(
                            width = fallback_frame.width,
                            height = fallback_frame.height,
                            samples_read = fallback_frame.samples_read,
                            decoded_bytes = fallback_frame.decoded_bytes,
                            "used bounded OpenH264 first-frame fallback"
                        );
                        let image_threads =
                            i32::try_from(threads).context("invalid libvips thread budget")?;
                        let result = image_thumbnail(ImageThumbnailJob {
                            input: fallback_output.clone(),
                            output: output.clone(),
                            max_edge: max_width,
                            format: ImageOutputFormat::Jpeg,
                            threads: image_threads,
                            watermark,
                            watermark_position: watermark_position.into(),
                            watermark_margin_px,
                            watermark_max_width_percent,
                            watermark_opacity_percent,
                        })
                        .context("OpenH264 fallback poster generation failed");
                        let _remove_result = tokio::fs::remove_file(fallback_output).await;
                        return result;
                    }
                    #[cfg(not(feature = "native-vips"))]
                    bail!("OpenH264 fallback requires the native-vips feature");
                }
                Err(error) => return Err(error).context("video thumbnail extraction failed"),
            }
            let Some(watermark) = watermark else {
                return Ok(());
            };
            let threads = i32::try_from(threads).context("invalid libvips thread budget")?;
            let result = image_thumbnail(ImageThumbnailJob {
                input: frame_output.clone(),
                output,
                max_edge: max_width,
                format: ImageOutputFormat::Jpeg,
                threads,
                watermark: Some(watermark),
                watermark_position: watermark_position.into(),
                watermark_margin_px,
                watermark_max_width_percent,
                watermark_opacity_percent,
            })
            .context("video poster watermark generation failed");
            let _remove_result = tokio::fs::remove_file(frame_output).await;
            result
        }
        Command::ImageThumbnail {
            input,
            output,
            max_edge,
            format,
            threads,
            watermark,
            watermark_position,
            watermark_margin_px,
            watermark_max_width_percent,
            watermark_opacity_percent,
        } => image_thumbnail(ImageThumbnailJob {
            input,
            output,
            max_edge,
            format: format.into(),
            threads,
            watermark,
            watermark_position: watermark_position.into(),
            watermark_margin_px,
            watermark_max_width_percent,
            watermark_opacity_percent,
        }),
    }
}

async fn probe(
    input: PathBuf,
    declared_kind: MediaKind,
    byte_len: u64,
    timeout: Duration,
    threads: i32,
) -> anyhow::Result<MediaInspection> {
    if byte_len == 0 || timeout.is_zero() || threads <= 0 {
        bail!("sandbox probe limits must be positive");
    }
    let format = detect_file(&input, declared_kind)
        .await
        .context("media signature validation failed")?;
    match declared_kind {
        MediaKind::Image => probe_image(input, byte_len, format, threads),
        MediaKind::Video => {
            let inspection = FfprobeInspector::new(PathBuf::from("ffprobe"), timeout)?
                .inspect(&VideoProbeRequest {
                    input,
                    byte_len,
                    format,
                })
                .await
                .context("video stream probe failed")?;
            VideoLimits::default()
                .validate(inspection.probe)
                .context("video resource policy rejected the source")?;
            Ok(MediaInspection::Video {
                format,
                content_type: format.content_type().to_owned(),
                inspection,
            })
        }
    }
}

#[cfg(feature = "native-vips")]
fn probe_image(
    input: PathBuf,
    byte_len: u64,
    format: g7mb_media::MediaFormat,
    threads: i32,
) -> anyhow::Result<MediaInspection> {
    let engine = g7mb_media::vips::VipsEngine::new(threads)?;
    let probe = engine
        .probe(&input, byte_len)
        .context("image decoder header probe failed")?;
    image_limits_for_format(format)
        .validate(probe)
        .context("image resource policy rejected the source")?;
    Ok(MediaInspection::Image {
        format,
        content_type: format.content_type().to_owned(),
        probe,
    })
}

#[cfg(not(feature = "native-vips"))]
fn probe_image(
    _input: PathBuf,
    _byte_len: u64,
    _format: g7mb_media::MediaFormat,
    _threads: i32,
) -> anyhow::Result<MediaInspection> {
    bail!("image probe requires the native-vips feature")
}

async fn doctor() -> anyhow::Result<()> {
    let reports = g7mb_media::native_tool_report().await?;
    for report in &reports {
        tracing::info!(tool = %report.tool, version = %report.version, healthy = report.healthy, warning = ?report.warning, "native tool report");
    }
    if reports.iter().any(|report| !report.healthy) {
        bail!("one or more native tools reported loader errors");
    }
    verify_network_is_denied()?;
    Ok(())
}

async fn runtime_capabilities() -> anyhow::Result<CapabilitiesResponse> {
    let reports = g7mb_media::native_tool_report().await?;
    if reports.iter().any(|report| !report.healthy) {
        bail!("one or more native tools reported loader errors");
    }
    let native_versions = reports
        .into_iter()
        .map(|report| (report.tool, report.version))
        .collect::<BTreeMap<_, _>>();
    let (image_inputs, image_outputs) = image_capabilities().await?;
    let mp4_thumbnail = video_thumbnail_capability().await.is_ok();
    Ok(CapabilitiesResponse {
        image_inputs,
        image_outputs,
        mp4_thumbnail,
        mp4_h264_fallback: cfg!(feature = "native-vips"),
        native_versions,
    })
}

#[cfg(feature = "native-vips")]
async fn image_capabilities() -> anyhow::Result<(Vec<String>, Vec<String>)> {
    const TINY_PPM: &[u8] = b"P6\n2 2\n255\n\xff\x00\x00\x00\xff\x00\x00\x00\xff\xff\xff\xff";

    let temp = tempfile::tempdir().context("failed to create capability fixture directory")?;
    let source = temp.path().join("source.ppm");
    tokio::fs::write(&source, TINY_PPM)
        .await
        .context("failed to write image capability fixture")?;
    let engine = g7mb_media::vips::VipsEngine::new(1)?;
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for (name, extension, format) in [
        ("jpeg", "jpg", ImageOutputFormat::Jpeg),
        ("png", "png", ImageOutputFormat::Png),
        ("webp", "webp", ImageOutputFormat::Webp),
        ("avif", "avif", ImageOutputFormat::Avif),
    ] {
        let output = temp.path().join(format!("roundtrip.{extension}"));
        if engine
            .thumbnail(&ImageThumbnailRequest {
                input: &source,
                output: &output,
                max_edge: 2,
                format,
                watermark: None,
            })
            .is_ok()
        {
            outputs.push(name.to_owned());
            if image_fixture_decodes(&engine, &output).await {
                inputs.push(name.to_owned());
            }
        }
    }
    for (name, extension) in [("gif", "gif"), ("heif", "heic")] {
        let output = temp.path().join(format!("decoder.{extension}"));
        if native_copy(&source, &output).await && image_fixture_decodes(&engine, &output).await {
            inputs.push(name.to_owned());
        }
    }
    inputs.sort_unstable();
    outputs.sort_unstable();
    Ok((inputs, outputs))
}

#[cfg(feature = "native-vips")]
async fn image_fixture_decodes(
    engine: &g7mb_media::vips::VipsEngine,
    path: &std::path::Path,
) -> bool {
    let Ok(metadata) = tokio::fs::metadata(path).await else {
        return false;
    };
    if metadata.len() == 0
        || detect_file(path, MediaKind::Image).await.is_err()
        || engine.probe(path, metadata.len()).is_err()
    {
        return false;
    }
    true
}

#[cfg(feature = "native-vips")]
async fn native_copy(input: &std::path::Path, output: &std::path::Path) -> bool {
    let command = tokio::process::Command::new("vips")
        .arg("copy")
        .arg(input)
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status();
    matches!(
        tokio::time::timeout(Duration::from_secs(5), command).await,
        Ok(Ok(status)) if status.success()
    )
}

#[cfg(not(feature = "native-vips"))]
async fn image_capabilities() -> anyhow::Result<(Vec<String>, Vec<String>)> {
    Ok((Vec::new(), Vec::new()))
}

async fn video_thumbnail_capability() -> anyhow::Result<()> {
    let temp = tempfile::tempdir().context("failed to create video capability directory")?;
    let input = temp.path().join("tiny-h264.mp4");
    let output = temp.path().join("poster.jpg");
    let fixture = STANDARD
        .decode(include_str!("../../../tests/fixtures/tiny-h264.mp4.b64").trim())
        .context("embedded MP4 capability fixture is invalid")?;
    tokio::fs::write(&input, &fixture)
        .await
        .context("failed to write MP4 capability fixture")?;
    let format = detect_file(&input, MediaKind::Video).await?;
    let inspection = FfprobeInspector::new(PathBuf::from("ffprobe"), Duration::from_secs(5))?
        .inspect(&VideoProbeRequest {
            input: input.clone(),
            byte_len: u64::try_from(fixture.len()).context("MP4 fixture length overflow")?,
            format,
        })
        .await?;
    FfmpegThumbnailer::new(PathBuf::from("ffmpeg"), Duration::from_secs(6), 1)?
        .extract(&VideoThumbnailRequest {
            input,
            output: output.clone(),
            timestamp_ms: 1,
            duration_ms: inspection.probe.duration_ms,
            max_width: 64,
        })
        .await?;
    let metadata = tokio::fs::metadata(output)
        .await
        .context("failed to inspect MP4 capability output")?;
    if metadata.len() == 0 {
        bail!("MP4 capability fixture produced an empty poster");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_network_filter() -> anyhow::Result<()> {
    use std::{collections::BTreeMap, convert::TryInto as _};

    use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};

    let network_syscalls = [
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_sendto,
        libc::SYS_recvfrom,
        libc::SYS_sendmsg,
        libc::SYS_recvmsg,
        libc::SYS_sendmmsg,
        libc::SYS_recvmmsg,
        libc::SYS_shutdown,
        libc::SYS_getsockname,
        libc::SYS_getpeername,
    ];
    let rules = network_syscalls
        .into_iter()
        .map(|syscall| (syscall, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let architecture = std::env::consts::ARCH
        .try_into()
        .map_err(|error| anyhow::anyhow!("unsupported seccomp architecture: {error:?}"))?;
    let errno = u32::try_from(libc::EPERM).context("EPERM cannot be represented")?;
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(errno),
        architecture,
    )?;
    let program: BpfProgram = filter.try_into()?;
    seccompiler::apply_filter_all_threads(&program)?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn install_network_filter() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn verify_network_is_denied() -> anyhow::Result<()> {
    let address = "127.0.0.1:9".parse()?;
    match std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
        Err(error) if error.raw_os_error() == Some(libc::EPERM) => Ok(()),
        Err(error) => bail!("sandbox network syscall was not denied with EPERM: {error}"),
        Ok(_) => bail!("sandbox unexpectedly opened a network connection"),
    }
}

#[cfg(not(target_os = "linux"))]
fn verify_network_is_denied() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(feature = "native-vips")]
fn image_thumbnail(job: ImageThumbnailJob) -> anyhow::Result<()> {
    let engine = g7mb_media::vips::VipsEngine::new(job.threads)?;
    tracing::debug!(version = engine.version()?, "initialized libvips");
    let watermark = job.watermark.as_deref().map(|watermark| WatermarkSpec {
        input: watermark,
        position: job.watermark_position,
        margin_px: job.watermark_margin_px,
        max_width_percent: job.watermark_max_width_percent,
        opacity_percent: job.watermark_opacity_percent,
    });
    engine
        .thumbnail(&ImageThumbnailRequest {
            input: &job.input,
            output: &job.output,
            max_edge: job.max_edge,
            format: job.format,
            watermark,
        })
        .context("image thumbnail generation failed")
}

#[cfg(not(feature = "native-vips"))]
fn image_thumbnail(_job: ImageThumbnailJob) -> anyhow::Result<()> {
    let ImageThumbnailJob {
        input,
        output,
        max_edge,
        format,
        threads,
        watermark,
        watermark_position,
        watermark_margin_px,
        watermark_max_width_percent,
        watermark_opacity_percent,
    } = _job;
    drop((
        input,
        output,
        max_edge,
        format,
        threads,
        watermark,
        watermark_position,
        watermark_margin_px,
        watermark_max_width_percent,
        watermark_opacity_percent,
    ));
    bail!("image support requires the native-vips feature")
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn seccomp_filter_denies_new_network_sockets() -> anyhow::Result<()> {
        super::install_network_filter()?;
        super::verify_network_is_denied()
    }
}
