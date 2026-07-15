//! Narrow, bounded MP4/H.264 fallback used only when FFmpeg cannot start.

use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write as _},
    path::Path,
};

use openh264::{decoder::Decoder, formats::YUVSource as _};

use crate::MediaError;

const ANNEX_B_START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Independent hard limits for the optional MP4/H.264 fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenH264FallbackLimits {
    /// Largest MP4 the fallback parser may open.
    pub max_input_bytes: u64,
    /// Maximum number of samples examined from the start of the video track.
    pub max_samples: u32,
    /// Largest individual encoded access unit.
    pub max_sample_bytes: usize,
    /// Aggregate encoded bytes passed to OpenH264.
    pub max_decode_bytes: usize,
    /// Largest number of NAL units accepted in one MP4 sample.
    pub max_nals_per_sample: usize,
    /// Maximum aggregate SPS/PPS bytes.
    pub max_parameter_set_bytes: usize,
    /// Maximum decoded width or height.
    pub max_dimension: usize,
    /// Maximum decoded pixels for the fallback frame.
    pub max_frame_pixels: usize,
    /// Number of decoder errors tolerated while searching for a valid frame.
    pub max_decode_errors: usize,
}

impl Default for OpenH264FallbackLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 512 * 1024 * 1024,
            max_samples: 120,
            max_sample_bytes: 16 * 1024 * 1024,
            max_decode_bytes: 32 * 1024 * 1024,
            max_nals_per_sample: 1_024,
            max_parameter_set_bytes: 256 * 1024,
            max_dimension: 4_096,
            max_frame_pixels: 16_777_216,
            max_decode_errors: 8,
        }
    }
}

impl OpenH264FallbackLimits {
    fn validate(self) -> Result<Self, MediaError> {
        if self.max_input_bytes == 0
            || self.max_samples == 0
            || self.max_sample_bytes == 0
            || self.max_decode_bytes < self.max_sample_bytes
            || self.max_nals_per_sample == 0
            || self.max_parameter_set_bytes == 0
            || self.max_dimension == 0
            || self.max_frame_pixels == 0
            || self.max_decode_errors == 0
        {
            return Err(MediaError::InvalidRequest(
                "OpenH264 fallback limits must be positive and internally consistent".to_owned(),
            ));
        }
        Ok(self)
    }
}

/// Facts about the credential-free frame written by the fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenH264FallbackFrame {
    /// Decoded width before thumbnail resizing.
    pub width: u32,
    /// Decoded height before thumbnail resizing.
    pub height: u32,
    /// Number of MP4 samples read before a frame became available.
    pub samples_read: u32,
    /// Aggregate encoded bytes passed to the decoder.
    pub decoded_bytes: usize,
}

/// Demuxes one H.264 video track and writes its first valid frame as trusted binary PPM.
///
/// The caller must still resize and encode this temporary frame with the normal image policy.
/// This function deliberately ignores the requested presentation timestamp: it is an emergency
/// first-frame fallback, not a general video seek implementation.
pub fn extract_first_frame_ppm(
    input: &Path,
    output: &Path,
    limits: OpenH264FallbackLimits,
) -> Result<OpenH264FallbackFrame, MediaError> {
    let limits = limits.validate()?;
    if input == output {
        return Err(MediaError::InvalidRequest(
            "fallback input and output must differ".to_owned(),
        ));
    }
    let input_file = File::open(input).map_err(MediaError::Input)?;
    let input_metadata = input_file.metadata().map_err(MediaError::Input)?;
    if !input_metadata.is_file()
        || input_metadata.len() == 0
        || input_metadata.len() > limits.max_input_bytes
    {
        return Err(MediaError::FallbackLimit);
    }

    let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(input_file), input_metadata.len())
        .map_err(|_| MediaError::FallbackDecode)?;
    let track = h264_track_config(&mp4, limits)?;
    let mut decoder = Decoder::new().map_err(|_| MediaError::FallbackDecode)?;
    let mut decode_errors = 0_usize;

    for parameter_set in openh264::nal_units(&track.parameter_sets) {
        match decoder.decode(parameter_set) {
            Ok(_) => {}
            Err(_) => {
                decode_errors += 1;
                if decode_errors > limits.max_decode_errors {
                    return Err(MediaError::FallbackDecode);
                }
            }
        }
    }

    let sample_limit = track.sample_count.min(limits.max_samples);
    let mut found_sync = false;
    let mut decoded_bytes = 0_usize;
    for sample_id in 1..=sample_limit {
        let sample = mp4
            .read_sample(track.track_id, sample_id)
            .map_err(|_| MediaError::FallbackDecode)?
            .ok_or(MediaError::FallbackDecode)?;
        if !found_sync {
            if !sample.is_sync {
                continue;
            }
            found_sync = true;
        }
        if sample.bytes.is_empty() || sample.bytes.len() > limits.max_sample_bytes {
            return Err(MediaError::FallbackLimit);
        }
        decoded_bytes = decoded_bytes
            .checked_add(sample.bytes.len())
            .filter(|total| *total <= limits.max_decode_bytes)
            .ok_or(MediaError::FallbackLimit)?;
        let access_unit = avcc_to_annex_b(
            sample.bytes.as_ref(),
            track.nal_length_size,
            limits.max_nals_per_sample,
            limits.max_sample_bytes,
        )?;

        for nal in openh264::nal_units(&access_unit) {
            match decoder.decode(nal) {
                Ok(Some(frame)) => {
                    let (width, height) = frame.dimensions();
                    validate_frame(width, height, limits)?;
                    let rgb_len = width
                        .checked_mul(height)
                        .and_then(|pixels| pixels.checked_mul(3))
                        .ok_or(MediaError::FallbackLimit)?;
                    let mut rgb = vec![0_u8; rgb_len];
                    frame.write_rgb8(&mut rgb);
                    write_ppm(output, width, height, &rgb)?;
                    return Ok(OpenH264FallbackFrame {
                        width: u32::try_from(width).map_err(|_| MediaError::FallbackLimit)?,
                        height: u32::try_from(height).map_err(|_| MediaError::FallbackLimit)?,
                        samples_read: sample_id,
                        decoded_bytes,
                    });
                }
                Ok(None) => {}
                Err(_) => {
                    decode_errors += 1;
                    if decode_errors > limits.max_decode_errors {
                        return Err(MediaError::FallbackDecode);
                    }
                }
            }
        }
    }
    if track.sample_count > sample_limit {
        Err(MediaError::FallbackLimit)
    } else {
        Err(MediaError::FallbackDecode)
    }
}

struct H264TrackConfig {
    track_id: u32,
    sample_count: u32,
    nal_length_size: usize,
    parameter_sets: Vec<u8>,
}

fn h264_track_config<R: std::io::Read + std::io::Seek>(
    mp4: &mp4::Mp4Reader<R>,
    limits: OpenH264FallbackLimits,
) -> Result<H264TrackConfig, MediaError> {
    let mut video_tracks = mp4.tracks().values().filter(|track| {
        track
            .track_type()
            .is_ok_and(|kind| kind == mp4::TrackType::Video)
    });
    let track = video_tracks.next().ok_or(MediaError::FallbackUnsupported)?;
    if video_tracks.next().is_some()
        || track
            .media_type()
            .map_err(|_| MediaError::FallbackUnsupported)?
            != mp4::MediaType::H264
    {
        return Err(MediaError::FallbackUnsupported);
    }
    let avc1 = track
        .trak
        .mdia
        .minf
        .stbl
        .stsd
        .avc1
        .as_ref()
        .ok_or(MediaError::FallbackUnsupported)?;
    let nal_length_size = usize::from(avc1.avcc.length_size_minus_one) + 1;
    if !matches!(nal_length_size, 1 | 2 | 4) {
        return Err(MediaError::FallbackUnsupported);
    }
    let parameter_set_count = avc1
        .avcc
        .sequence_parameter_sets
        .len()
        .checked_add(avc1.avcc.picture_parameter_sets.len())
        .ok_or(MediaError::FallbackLimit)?;
    if parameter_set_count == 0 || parameter_set_count > 32 {
        return Err(MediaError::FallbackLimit);
    }
    let mut parameter_sets = Vec::new();
    for set in avc1
        .avcc
        .sequence_parameter_sets
        .iter()
        .chain(&avc1.avcc.picture_parameter_sets)
    {
        if set.bytes.is_empty() || set.bytes.len() > u16::MAX as usize {
            return Err(MediaError::FallbackLimit);
        }
        append_annex_b_nal(
            &mut parameter_sets,
            &set.bytes,
            limits.max_parameter_set_bytes,
        )?;
    }
    let sample_count = track.sample_count();
    if sample_count == 0 {
        return Err(MediaError::FallbackDecode);
    }
    Ok(H264TrackConfig {
        track_id: track.track_id(),
        sample_count,
        nal_length_size,
        parameter_sets,
    })
}

fn avcc_to_annex_b(
    input: &[u8],
    nal_length_size: usize,
    max_nals: usize,
    max_output_bytes: usize,
) -> Result<Vec<u8>, MediaError> {
    if input.is_empty() || !matches!(nal_length_size, 1 | 2 | 4) {
        return Err(MediaError::FallbackDecode);
    }
    let mut cursor = 0_usize;
    let mut nals = 0_usize;
    let mut output = Vec::with_capacity(input.len().min(max_output_bytes));
    while cursor < input.len() {
        let length_end = cursor
            .checked_add(nal_length_size)
            .filter(|end| *end <= input.len())
            .ok_or(MediaError::FallbackDecode)?;
        let mut length = 0_usize;
        for byte in &input[cursor..length_end] {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(usize::from(*byte)))
                .ok_or(MediaError::FallbackLimit)?;
        }
        if length == 0 {
            return Err(MediaError::FallbackDecode);
        }
        let nal_end = length_end
            .checked_add(length)
            .filter(|end| *end <= input.len())
            .ok_or(MediaError::FallbackDecode)?;
        nals += 1;
        if nals > max_nals {
            return Err(MediaError::FallbackLimit);
        }
        append_annex_b_nal(&mut output, &input[length_end..nal_end], max_output_bytes)?;
        cursor = nal_end;
    }
    if output.is_empty() {
        Err(MediaError::FallbackDecode)
    } else {
        Ok(output)
    }
}

fn append_annex_b_nal(
    output: &mut Vec<u8>,
    nal: &[u8],
    max_output_bytes: usize,
) -> Result<(), MediaError> {
    let new_len = output
        .len()
        .checked_add(ANNEX_B_START_CODE.len())
        .and_then(|length| length.checked_add(nal.len()))
        .filter(|length| *length <= max_output_bytes)
        .ok_or(MediaError::FallbackLimit)?;
    output.reserve(new_len - output.len());
    output.extend_from_slice(&ANNEX_B_START_CODE);
    output.extend_from_slice(nal);
    Ok(())
}

fn validate_frame(
    width: usize,
    height: usize,
    limits: OpenH264FallbackLimits,
) -> Result<(), MediaError> {
    let pixels = width.checked_mul(height).ok_or(MediaError::FallbackLimit)?;
    if width == 0
        || height == 0
        || width > limits.max_dimension
        || height > limits.max_dimension
        || pixels > limits.max_frame_pixels
    {
        return Err(MediaError::FallbackLimit);
    }
    Ok(())
}

fn write_ppm(output: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<(), MediaError> {
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)
        .map_err(MediaError::Output)?;
    let mut writer = BufWriter::new(file);
    write!(writer, "P6\n{width} {height}\n255\n").map_err(MediaError::Output)?;
    writer.write_all(rgb).map_err(MediaError::Output)?;
    writer.flush().map_err(MediaError::Output)
}

#[cfg(test)]
mod tests {
    use super::{OpenH264FallbackLimits, avcc_to_annex_b};
    use crate::MediaError;

    #[test]
    fn converts_bounded_avcc_access_units_to_annex_b() -> Result<(), MediaError> {
        let input = [0, 0, 0, 2, 0x65, 0xaa, 0, 0, 0, 1, 0x06];
        let converted = avcc_to_annex_b(&input, 4, 4, 64)?;
        assert_eq!(converted, [0, 0, 0, 1, 0x65, 0xaa, 0, 0, 0, 1, 0x06]);
        Ok(())
    }

    #[test]
    fn rejects_truncated_or_unbounded_avcc_samples() {
        assert!(matches!(
            avcc_to_annex_b(&[0, 0, 0, 8, 0x65], 4, 4, 64),
            Err(MediaError::FallbackDecode)
        ));
        assert!(matches!(
            avcc_to_annex_b(&[1, 0x65, 1, 0x06], 1, 1, 64),
            Err(MediaError::FallbackLimit)
        ));
        assert!(matches!(
            avcc_to_annex_b(&[1, 0x65], 3, 4, 64),
            Err(MediaError::FallbackDecode)
        ));
    }

    #[test]
    fn fallback_limits_are_fail_closed() {
        let invalid = OpenH264FallbackLimits {
            max_samples: 0,
            ..OpenH264FallbackLimits::default()
        };
        assert!(matches!(
            invalid.validate(),
            Err(MediaError::InvalidRequest(_))
        ));
    }
}
