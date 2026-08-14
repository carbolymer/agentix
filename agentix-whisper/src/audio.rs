//! Audio decode pipeline: arbitrary compressed audio → 16 kHz mono f32 PCM.
//!
//! Uses symphonia for format detection and decoding, rubato for resampling.
//! All blocking I/O and CPU work runs inside `spawn_blocking`.

use agentix_infer::InferError;

const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Decode an audio file (WAV, MP3, FLAC, OGG, …) from raw bytes into
/// 16 kHz mono f32 PCM samples suitable for whisper.cpp.
///
/// All blocking work is performed inside `tokio::task::spawn_blocking`.
pub async fn decode_audio_to_pcm(audio: Vec<u8>) -> Result<Vec<f32>, InferError> {
    tokio::task::spawn_blocking(move || decode_blocking(&audio))
        .await
        .map_err(|e| InferError::Backend(format!("spawn_blocking join error: {e}")))?
}

fn decode_blocking(audio: &[u8]) -> Result<Vec<f32>, InferError> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let cursor = std::io::Cursor::new(audio.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let hint = Hint::new();
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| InferError::Backend(format!("audio format detection failed: {e}")))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| InferError::Backend("no decodeable audio track found".to_string()))?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| InferError::Backend("audio track has no sample rate".to_string()))?;

    let channels = codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| InferError::Backend(format!("codec not supported: {e}")))?;

    let mut interleaved: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(symphonia::core::errors::Error::IoError(e)) => {
                return Err(InferError::Backend(format!("audio IO error: {e}")));
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => {
                return Err(InferError::Backend(format!("audio read error: {e}")));
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::IoError(_)) => continue,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => {
                return Err(InferError::Backend(format!("decode error: {e}")));
            }
        };

        let spec = *decoded.spec();
        let duration = decoded.capacity() as u64;

        let buf = sample_buf.get_or_insert_with(|| SampleBuffer::<f32>::new(duration, spec));
        buf.copy_interleaved_ref(decoded);
        interleaved.extend_from_slice(buf.samples());
    }

    // Downmix interleaved multi-channel to mono by averaging channels.
    let mono: Vec<f32> = if channels == 1 {
        interleaved
    } else {
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    if sample_rate == TARGET_SAMPLE_RATE {
        return Ok(mono);
    }

    resample(mono, sample_rate, TARGET_SAMPLE_RATE)
}

fn resample(samples: Vec<f32>, from_rate: u32, to_rate: u32) -> Result<Vec<f32>, InferError> {
    use rubato::{FftFixedIn, Resampler};

    let ratio = to_rate as f64 / from_rate as f64;
    let chunk_size = 4096_usize;

    let mut resampler = FftFixedIn::<f32>::new(
        from_rate as usize,
        to_rate as usize,
        chunk_size,
        2,
        1, // mono
    )
    .map_err(|e| InferError::Backend(format!("resampler creation failed: {e}")))?;

    let total_frames = samples.len();
    let estimated_output = (total_frames as f64 * ratio).ceil() as usize + chunk_size;
    let mut output = Vec::with_capacity(estimated_output);

    let mut pos = 0usize;
    let mut wave_in = vec![vec![0f32; chunk_size]; 1];

    while pos < total_frames || pos == 0 {
        let remaining = total_frames.saturating_sub(pos);
        let frames_this_chunk = remaining.min(chunk_size);

        wave_in[0][..frames_this_chunk].copy_from_slice(&samples[pos..pos + frames_this_chunk]);
        wave_in[0][frames_this_chunk..chunk_size].fill(0.0);

        let waves_out = resampler
            .process(&wave_in, None)
            .map_err(|e| InferError::Backend(format!("resampler process error: {e}")))?;

        output.extend_from_slice(&waves_out[0]);
        pos += frames_this_chunk;

        if frames_this_chunk < chunk_size {
            break;
        }
    }

    Ok(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_sine_wav(sample_rate: u32, duration_secs: f32, freq_hz: f32) -> Vec<u8> {
        let n_samples = (sample_rate as f32 * duration_secs) as u32;
        let data_bytes = n_samples * 2; // 16-bit PCM

        let mut wav = Vec::new();

        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..n_samples {
            let t = i as f32 / sample_rate as f32;
            let s = (2.0 * std::f32::consts::PI * freq_hz * t).sin();
            let sample = (s * i16::MAX as f32) as i16;
            wav.extend_from_slice(&sample.to_le_bytes());
        }

        wav
    }

    #[tokio::test]
    async fn decode_sine_wav_produces_correct_length() {
        let duration_secs = 1.0f32;
        let wav = make_sine_wav(44_100, duration_secs, 440.0);

        let pcm = decode_audio_to_pcm(wav).await.expect("decode failed");

        let expected = (duration_secs * TARGET_SAMPLE_RATE as f32) as usize;
        let tolerance = expected / 20; // 5%
        assert!(
            pcm.len().abs_diff(expected) <= tolerance,
            "expected ≈{expected} samples at 16kHz, got {}",
            pcm.len()
        );
    }

    #[tokio::test]
    async fn decode_native_16khz_wav_no_resample() {
        let duration_secs = 0.5f32;
        let wav = make_sine_wav(16_000, duration_secs, 440.0);

        let pcm = decode_audio_to_pcm(wav).await.expect("decode failed");

        let expected = (duration_secs * 16_000.0) as usize;
        assert!(
            pcm.len().abs_diff(expected) <= 64,
            "expected ≈{expected} samples, got {}",
            pcm.len()
        );
    }
}
