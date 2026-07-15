//! Pure domain policy for uploads and media processing.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Public, unguessable identifier for an upload.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UploadId(Uuid);

impl UploadId {
    /// Creates a time-sortable UUIDv7 upload identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Returns the underlying UUID for persistence adapters.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for UploadId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for UploadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for UploadId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Public identifier grouping up to one bounded multi-upload request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UploadBatchId(Uuid);

impl UploadBatchId {
    /// Creates a time-sortable UUIDv7 batch identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Returns the underlying UUID for transport and persistence adapters.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for UploadBatchId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for UploadBatchId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for UploadBatchId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Server-selected direct upload transfer strategy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadTransfer {
    /// One presigned PUT request.
    SinglePut,
    /// Resumable S3-compatible multipart upload.
    Multipart {
        /// Target part size. The final part may be smaller.
        part_size_bytes: u64,
    },
}

/// Untrusted client facts used only to reserve upload capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadCandidate {
    /// Client-declared media class, verified after upload.
    pub declared_kind: MediaKind,
    /// Exact encoded byte length the direct upload must contain.
    pub byte_len: u64,
}

/// Bounded policy for one multi-upload batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadBatchPolicy {
    /// Maximum files in one batch.
    pub max_files: usize,
    /// Aggregate encoded bytes reserved by one batch.
    pub max_batch_bytes: u64,
    /// Maximum encoded image length.
    pub max_image_bytes: u64,
    /// Maximum encoded video length.
    pub max_video_bytes: u64,
    /// Files at or above this length use multipart.
    pub multipart_threshold_bytes: u64,
    /// Planned multipart part size.
    pub multipart_part_size_bytes: u64,
}

impl Default for UploadBatchPolicy {
    fn default() -> Self {
        Self {
            max_files: 100,
            max_batch_bytes: 20 * 1024 * 1024 * 1024,
            max_image_bytes: 128 * 1024 * 1024,
            max_video_bytes: 5 * 1024 * 1024 * 1024,
            multipart_threshold_bytes: 100 * 1024 * 1024,
            multipart_part_size_bytes: 32 * 1024 * 1024,
        }
    }
}

impl UploadBatchPolicy {
    /// Validates every candidate and returns one transfer strategy per file.
    pub fn plan(
        self,
        candidates: &[UploadCandidate],
    ) -> Result<Vec<UploadTransfer>, UploadBatchPolicyError> {
        if candidates.is_empty() {
            return Err(UploadBatchPolicyError::Empty);
        }
        if candidates.len() > self.max_files {
            return Err(UploadBatchPolicyError::TooManyFiles);
        }
        if self.multipart_part_size_bytes < 5 * 1024 * 1024 {
            return Err(UploadBatchPolicyError::InvalidPartSize);
        }

        let mut total = 0_u64;
        let mut transfers = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if candidate.byte_len == 0 {
                return Err(UploadBatchPolicyError::EmptyFile);
            }
            let max_bytes = match candidate.declared_kind {
                MediaKind::Image => self.max_image_bytes,
                MediaKind::Video => self.max_video_bytes,
            };
            if candidate.byte_len > max_bytes {
                return Err(UploadBatchPolicyError::FileTooLarge);
            }
            total = total
                .checked_add(candidate.byte_len)
                .ok_or(UploadBatchPolicyError::BatchTooLarge)?;
            if total > self.max_batch_bytes {
                return Err(UploadBatchPolicyError::BatchTooLarge);
            }

            let transfer = if candidate.declared_kind == MediaKind::Video
                || candidate.byte_len >= self.multipart_threshold_bytes
            {
                UploadTransfer::Multipart {
                    part_size_bytes: self.multipart_part_size_bytes,
                }
            } else {
                UploadTransfer::SinglePut
            };
            transfers.push(transfer);
        }
        Ok(transfers)
    }
}

/// Invalid multi-upload reservation request.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum UploadBatchPolicyError {
    /// A batch must contain at least one file.
    #[error("upload batch is empty")]
    Empty,
    /// File count exceeds the configured batch cap.
    #[error("upload batch contains too many files")]
    TooManyFiles,
    /// Zero-byte media is not accepted.
    #[error("upload file is empty")]
    EmptyFile,
    /// One file exceeds its media-class byte cap.
    #[error("upload file exceeds byte limit")]
    FileTooLarge,
    /// Aggregate reservation exceeds the batch byte cap.
    #[error("upload batch exceeds aggregate byte limit")]
    BatchTooLarge,
    /// S3-compatible non-final parts must be at least 5 MiB.
    #[error("multipart part size is below the S3-compatible minimum")]
    InvalidPartSize,
}

/// A server-owned object storage key.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ObjectKey(String);

impl ObjectKey {
    /// Validates and constructs an object key.
    pub fn new(value: impl Into<String>) -> Result<Self, ObjectKeyError> {
        let value = value.into();
        validate_object_key(&value)?;
        Ok(Self(value))
    }

    /// Returns the validated key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ObjectKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ObjectKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Object key validation failure.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ObjectKeyError {
    /// Empty keys are not valid.
    #[error("object key is empty")]
    Empty,
    /// S3-compatible object keys are bounded for predictable signing and storage.
    #[error("object key exceeds 1024 bytes")]
    TooLong,
    /// Absolute paths are forbidden.
    #[error("object key must be relative")]
    Absolute,
    /// Empty, current, or parent path segments are forbidden.
    #[error("object key contains an unsafe path segment")]
    UnsafeSegment,
    /// Only a narrow portable ASCII alphabet is accepted.
    #[error("object key contains an unsupported character")]
    UnsupportedCharacter,
}

fn validate_object_key(value: &str) -> Result<(), ObjectKeyError> {
    if value.is_empty() {
        return Err(ObjectKeyError::Empty);
    }
    if value.len() > 1024 {
        return Err(ObjectKeyError::TooLong);
    }
    if value.starts_with('/') {
        return Err(ObjectKeyError::Absolute);
    }
    if value
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(ObjectKeyError::UnsafeSegment);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
    {
        return Err(ObjectKeyError::UnsupportedCharacter);
    }
    Ok(())
}

/// Media class selected after trusted probing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    /// A supported raster image.
    Image,
    /// A supported video container.
    Video,
}

/// Upload lifecycle state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadState {
    /// An upload intent exists but bytes are not confirmed.
    Created,
    /// Object storage reports uploaded bytes.
    Uploaded,
    /// The object is private and waiting for processing.
    Quarantined,
    /// A worker owns a processing lease.
    Processing,
    /// All required derivatives are published.
    Ready,
    /// Policy rejected the source permanently.
    Rejected,
    /// Processing failed after the allowed retries.
    Failed,
    /// Deletion has completed or the record is tombstoned.
    Deleted,
}

impl UploadState {
    /// Returns whether the state machine permits the target transition.
    #[must_use]
    pub const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Created, Self::Uploaded)
                | (Self::Uploaded, Self::Quarantined)
                | (
                    Self::Quarantined,
                    Self::Processing | Self::Rejected | Self::Failed
                )
                | (
                    Self::Processing,
                    Self::Ready | Self::Rejected | Self::Failed
                )
                | (
                    Self::Created
                        | Self::Uploaded
                        | Self::Quarantined
                        | Self::Processing
                        | Self::Ready
                        | Self::Rejected
                        | Self::Failed,
                    Self::Deleted
                )
        )
    }

    /// Applies a validated transition.
    pub fn transition_to(&mut self, target: Self) -> Result<(), StateTransitionError> {
        if !self.can_transition_to(target) {
            return Err(StateTransitionError {
                from: *self,
                to: target,
            });
        }
        *self = target;
        Ok(())
    }
}

/// An invalid lifecycle transition.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("invalid upload state transition from {from:?} to {to:?}")]
pub struct StateTransitionError {
    /// Current state.
    pub from: UploadState,
    /// Requested state.
    pub to: UploadState,
}

/// Probe facts used by image policy before full processing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImageProbe {
    /// Encoded source length.
    pub byte_len: u64,
    /// Decoded width.
    pub width: u32,
    /// Decoded height.
    pub height: u32,
    /// Animation frame count, or one for still images.
    pub frames: u32,
}

/// Resource lane selected from trusted image header facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageWorkClass {
    /// Normal image lane; bounded by the worker's general concurrency.
    Standard,
    /// High-cost image lane; additionally bounded by the heavy-image semaphore.
    Heavy,
}

impl ImageProbe {
    /// Classifies a valid still or animation before full-pixel transformation.
    #[must_use]
    pub fn work_class(self) -> ImageWorkClass {
        const HEAVY_DIMENSION_THRESHOLD: u32 = 16_384;
        const HEAVY_PIXEL_THRESHOLD: u64 = 100_000_000;

        let pixels = u64::from(self.width) * u64::from(self.height);
        if self.width > HEAVY_DIMENSION_THRESHOLD
            || self.height > HEAVY_DIMENSION_THRESHOLD
            || pixels > HEAVY_PIXEL_THRESHOLD
        {
            ImageWorkClass::Heavy
        } else {
            ImageWorkClass::Standard
        }
    }
}

/// Resource policy for untrusted raster images.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageLimits {
    /// Largest accepted encoded input.
    pub max_bytes: u64,
    /// Largest accepted width or height.
    pub max_dimension: u32,
    /// Largest decoded pixel count for one frame.
    pub max_pixels: u64,
    /// Largest accepted animation frame count.
    pub max_frames: u32,
    /// Largest aggregate decoded pixel budget across frames.
    pub max_animated_pixels: u64,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_dimension: 32_768,
            max_pixels: 200_000_000,
            max_frames: 300,
            max_animated_pixels: 500_000_000,
        }
    }
}

impl ImageLimits {
    /// Validates trusted probe facts against every independent resource budget.
    pub fn validate(self, probe: ImageProbe) -> Result<(), ImagePolicyError> {
        if probe.byte_len > self.max_bytes {
            return Err(ImagePolicyError::EncodedBytes);
        }
        if probe.width == 0 || probe.height == 0 || probe.frames == 0 {
            return Err(ImagePolicyError::ZeroDimensionOrFrame);
        }
        if probe.width > self.max_dimension || probe.height > self.max_dimension {
            return Err(ImagePolicyError::Dimension);
        }
        let pixels = u64::from(probe.width) * u64::from(probe.height);
        if pixels > self.max_pixels {
            return Err(ImagePolicyError::Pixels);
        }
        if probe.frames > self.max_frames {
            return Err(ImagePolicyError::Frames);
        }
        let animated_pixels = pixels
            .checked_mul(u64::from(probe.frames))
            .ok_or(ImagePolicyError::AnimatedPixels)?;
        if animated_pixels > self.max_animated_pixels {
            return Err(ImagePolicyError::AnimatedPixels);
        }
        Ok(())
    }
}

/// Image resource policy failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ImagePolicyError {
    /// Encoded source is too large.
    #[error("encoded image exceeds byte limit")]
    EncodedBytes,
    /// A decoded dimension is invalid or a frame count is zero.
    #[error("image has a zero dimension or frame count")]
    ZeroDimensionOrFrame,
    /// One decoded dimension is too large.
    #[error("image exceeds dimension limit")]
    Dimension,
    /// One decoded frame is too large.
    #[error("image exceeds pixel limit")]
    Pixels,
    /// Animation has too many frames.
    #[error("image exceeds frame limit")]
    Frames,
    /// Aggregate animated decode cost is too large.
    #[error("image exceeds animated pixel budget")]
    AnimatedPixels,
}

/// Trusted container and stream facts used before video frame extraction.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VideoProbe {
    /// Encoded source length.
    pub byte_len: u64,
    /// Duration rounded up to milliseconds.
    pub duration_ms: u64,
    /// Primary video width.
    pub width: u32,
    /// Primary video height.
    pub height: u32,
    /// Number of video streams.
    pub video_streams: u32,
    /// Total streams of every type.
    pub total_streams: u32,
}

/// Resource policy for untrusted video containers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoLimits {
    /// Largest accepted encoded input.
    pub max_bytes: u64,
    /// Longest accepted duration.
    pub max_duration_ms: u64,
    /// Largest accepted width or height.
    pub max_dimension: u32,
    /// Largest decoded pixel count for one frame.
    pub max_pixels: u64,
    /// Largest total stream count.
    pub max_streams: u32,
}

impl Default for VideoLimits {
    fn default() -> Self {
        Self {
            max_bytes: 5 * 1024 * 1024 * 1024,
            max_duration_ms: 6 * 60 * 60 * 1000,
            max_dimension: 8_192,
            max_pixels: 40_000_000,
            max_streams: 8,
        }
    }
}

impl VideoLimits {
    /// Validates trusted FFprobe facts against independent resource budgets.
    pub fn validate(self, probe: VideoProbe) -> Result<(), VideoPolicyError> {
        if probe.byte_len == 0 || probe.byte_len > self.max_bytes {
            return Err(VideoPolicyError::EncodedBytes);
        }
        if probe.duration_ms == 0 || probe.duration_ms > self.max_duration_ms {
            return Err(VideoPolicyError::Duration);
        }
        if probe.width == 0 || probe.height == 0 {
            return Err(VideoPolicyError::Dimension);
        }
        if probe.width > self.max_dimension || probe.height > self.max_dimension {
            return Err(VideoPolicyError::Dimension);
        }
        let pixels = u64::from(probe.width) * u64::from(probe.height);
        if pixels > self.max_pixels {
            return Err(VideoPolicyError::Pixels);
        }
        if probe.video_streams != 1
            || probe.total_streams == 0
            || probe.total_streams > self.max_streams
        {
            return Err(VideoPolicyError::Streams);
        }
        Ok(())
    }
}

/// Video resource policy failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum VideoPolicyError {
    /// Encoded source is empty or too large.
    #[error("video encoded byte length is outside policy")]
    EncodedBytes,
    /// Duration is missing or exceeds the allowed budget.
    #[error("video duration is outside policy")]
    Duration,
    /// A decoded dimension is zero or too large.
    #[error("video dimensions are outside policy")]
    Dimension,
    /// One decoded frame is too large.
    #[error("video frame exceeds pixel policy")]
    Pixels,
    /// Stream count or primary-video count is unsupported.
    #[error("video stream layout is outside policy")]
    Streams,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        ImageLimits, ImagePolicyError, ImageProbe, ImageWorkClass, MediaKind, ObjectKey,
        UploadBatchId, UploadBatchPolicy, UploadBatchPolicyError, UploadCandidate, UploadId,
        UploadState, UploadTransfer, VideoLimits, VideoPolicyError, VideoProbe,
    };

    #[test]
    fn allows_expected_state_path() -> Result<(), Box<dyn std::error::Error>> {
        let mut state = UploadState::Created;
        for target in [
            UploadState::Uploaded,
            UploadState::Quarantined,
            UploadState::Processing,
            UploadState::Ready,
            UploadState::Deleted,
        ] {
            state.transition_to(target)?;
        }
        Ok(())
    }

    #[test]
    fn rejects_skipped_state() {
        let mut state = UploadState::Created;
        assert!(state.transition_to(UploadState::Ready).is_err());
        assert_eq!(state, UploadState::Created);
    }

    #[test]
    fn rejects_animated_pixel_bomb() {
        let result = ImageLimits::default().validate(ImageProbe {
            byte_len: 1024,
            width: 10_000,
            height: 10_000,
            frames: 10,
        });
        assert_eq!(result, Err(ImagePolicyError::AnimatedPixels));
    }

    #[test]
    fn validates_one_bounded_primary_video_stream() {
        let valid = VideoProbe {
            byte_len: 50 * 1024 * 1024,
            duration_ms: 90_000,
            width: 3_840,
            height: 2_160,
            video_streams: 1,
            total_streams: 2,
        };
        assert_eq!(VideoLimits::default().validate(valid), Ok(()));
        assert_eq!(
            VideoLimits::default().validate(VideoProbe {
                video_streams: 2,
                ..valid
            }),
            Err(VideoPolicyError::Streams)
        );
    }

    #[test]
    fn plans_bounded_multi_upload_without_confusing_server_nodes()
    -> Result<(), UploadBatchPolicyError> {
        let transfers = UploadBatchPolicy::default().plan(&[
            UploadCandidate {
                declared_kind: MediaKind::Image,
                byte_len: 2 * 1024 * 1024,
            },
            UploadCandidate {
                declared_kind: MediaKind::Video,
                byte_len: 50 * 1024 * 1024,
            },
            UploadCandidate {
                declared_kind: MediaKind::Image,
                byte_len: 100 * 1024 * 1024,
            },
        ])?;
        assert_eq!(
            transfers,
            vec![
                UploadTransfer::SinglePut,
                UploadTransfer::Multipart {
                    part_size_bytes: 32 * 1024 * 1024,
                },
                UploadTransfer::Multipart {
                    part_size_bytes: 32 * 1024 * 1024,
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn rejects_more_than_one_hundred_files() {
        let candidates = vec![
            UploadCandidate {
                declared_kind: MediaKind::Image,
                byte_len: 1,
            };
            101
        ];
        assert_eq!(
            UploadBatchPolicy::default().plan(&candidates),
            Err(UploadBatchPolicyError::TooManyFiles)
        );
    }

    #[test]
    fn rejects_every_invalid_batch_capacity_boundary() {
        let default_policy = UploadBatchPolicy::default();
        assert_eq!(default_policy.plan(&[]), Err(UploadBatchPolicyError::Empty));
        assert_eq!(
            default_policy.plan(&[UploadCandidate {
                declared_kind: MediaKind::Image,
                byte_len: 0,
            }]),
            Err(UploadBatchPolicyError::EmptyFile)
        );
        assert_eq!(
            default_policy.plan(&[UploadCandidate {
                declared_kind: MediaKind::Image,
                byte_len: default_policy.max_image_bytes + 1,
            }]),
            Err(UploadBatchPolicyError::FileTooLarge)
        );
        assert_eq!(
            default_policy.plan(&[UploadCandidate {
                declared_kind: MediaKind::Video,
                byte_len: default_policy.max_video_bytes + 1,
            }]),
            Err(UploadBatchPolicyError::FileTooLarge)
        );

        let aggregate_policy = UploadBatchPolicy {
            max_batch_bytes: 10,
            max_image_bytes: 10,
            ..default_policy
        };
        assert_eq!(
            aggregate_policy.plan(&[
                UploadCandidate {
                    declared_kind: MediaKind::Image,
                    byte_len: 6,
                },
                UploadCandidate {
                    declared_kind: MediaKind::Image,
                    byte_len: 6,
                },
            ]),
            Err(UploadBatchPolicyError::BatchTooLarge)
        );

        let invalid_part_policy = UploadBatchPolicy {
            multipart_part_size_bytes: 5 * 1024 * 1024 - 1,
            ..default_policy
        };
        assert_eq!(
            invalid_part_policy.plan(&[UploadCandidate {
                declared_kind: MediaKind::Image,
                byte_len: 1,
            }]),
            Err(UploadBatchPolicyError::InvalidPartSize)
        );
    }

    #[test]
    fn time_sortable_identifiers_round_trip_through_persistence_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let upload_id = UploadId::new();
        assert_eq!(upload_id.to_string().parse::<UploadId>()?, upload_id);
        assert_eq!(upload_id.as_uuid().to_string(), upload_id.to_string());
        assert!(!UploadId::default().to_string().is_empty());

        let batch_id = UploadBatchId::new();
        assert_eq!(batch_id.to_string().parse::<UploadBatchId>()?, batch_id);
        assert!(!UploadBatchId::default().to_string().is_empty());
        Ok(())
    }

    #[test]
    fn default_image_policy_accepts_twenty_five_thousand_pixel_panorama() {
        let probe = ImageProbe {
            byte_len: 16 * 1024 * 1024,
            width: 25_000,
            height: 4_000,
            frames: 1,
        };
        assert_eq!(ImageLimits::default().validate(probe), Ok(()));
        assert_eq!(probe.work_class(), ImageWorkClass::Heavy);
        assert_eq!(
            ImageProbe {
                width: 16_384,
                height: 4_000,
                ..probe
            }
            .work_class(),
            ImageWorkClass::Standard
        );
    }

    #[test]
    fn deserialization_cannot_bypass_object_key_validation()
    -> Result<(), Box<dyn std::error::Error>> {
        let invalid = serde_json::from_str::<ObjectKey>(r#""raw/tenant/../secret""#);
        assert!(invalid.is_err());
        let valid = serde_json::from_str::<ObjectKey>(r#""raw/tenant/id/source.jpg""#)?;
        assert_eq!(valid.as_str(), "raw/tenant/id/source.jpg");
        Ok(())
    }

    proptest! {
        #[test]
        fn object_key_never_accepts_parent_segments(prefix in "[A-Za-z0-9_-]{1,32}") {
            let candidate = format!("{prefix}/../source.jpg");
            prop_assert!(ObjectKey::new(candidate).is_err());
        }

        #[test]
        fn safe_generated_keys_are_accepted(id in "[A-Za-z0-9_-]{1,64}") {
            let candidate = format!("raw/tenant/{id}/source.jpg");
            prop_assert!(ObjectKey::new(candidate).is_ok());
        }
    }
}
