use shiguredo_mp4::boxes::SampleEntry;

use crate::event::{AudioFrameData, VideoFrameData};
use crate::mp4::{SimpleMp4Writer, opus_sample_entry_value};

/// フレームを VP9 エンコードして MP4 に書き込む
pub fn encode_and_write_frame(
    frame_data: &VideoFrameData,
    vp9_encoder: &mut Option<shiguredo_libvpx::Encoder>,
    vp9_sample_entry: &mut Option<SampleEntry>,
    mp4_writer: &mut SimpleMp4Writer,
    output_video_width: &mut usize,
    output_video_height: &mut usize,
) -> Result<(), String> {
    let width = frame_data.width as usize;
    let height = frame_data.height as usize;

    // エンコーダーの遅延初期化
    if vp9_encoder.is_none() {
        let config = shiguredo_libvpx::EncoderConfig::new(
            width,
            height,
            shiguredo_libvpx::ImageFormat::I420,
            shiguredo_libvpx::CodecConfig::Vp9(Default::default()),
        );
        let encoder = shiguredo_libvpx::Encoder::new(config)
            .map_err(|e| format!("failed to create VP9 encoder: {e}"))?;
        *vp9_encoder = Some(encoder);
        *vp9_sample_entry = Some(crate::mp4::vp9_sample_entry(width, height));
    }

    let encoder = vp9_encoder.as_mut().unwrap();
    let compact_y = compact_i420_plane(&frame_data.y, width, height, frame_data.stride_y)?;
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let compact_u = compact_i420_plane(&frame_data.u, uv_width, uv_height, frame_data.stride_u)?;
    let compact_v = compact_i420_plane(&frame_data.v, uv_width, uv_height, frame_data.stride_v)?;

    let encode_options = shiguredo_libvpx::EncodeOptions {
        force_keyframe: false,
    };
    encoder
        .encode(
            &shiguredo_libvpx::ImageData::I420 {
                y: &compact_y,
                u: &compact_u,
                v: &compact_v,
            },
            &encode_options,
        )
        .map_err(|e| format!("VP9 encode failed: {e}"))?;
    *output_video_width = width;
    *output_video_height = height;

    while let Some(frame) = encoder.next_frame() {
        let se = vp9_sample_entry.take();
        mp4_writer.append_video(
            frame.data(),
            frame.is_keyframe(),
            se,
            frame_data.timestamp_us,
        )?;
    }
    Ok(())
}

pub fn compact_i420_plane(
    plane: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) -> Result<Vec<u8>, String> {
    if stride < width {
        return Err(format!(
            "invalid I420 stride: stride={stride}, width={width}, height={height}"
        ));
    }
    let required_len = stride
        .checked_mul(height)
        .ok_or_else(|| format!("I420 plane size overflow: stride={stride}, height={height}"))?;
    if plane.len() < required_len {
        return Err(format!(
            "insufficient I420 plane data: expected at least {required_len} bytes, got {}",
            plane.len()
        ));
    }
    if stride == width {
        return Ok(plane[..width * height].to_vec());
    }

    let mut compact = Vec::with_capacity(width * height);
    for row in 0..height {
        let offset = row * stride;
        compact.extend_from_slice(&plane[offset..offset + width]);
    }
    Ok(compact)
}

/// 受信した PCM 音声データを Opus エンコードして MP4 に書き込む
pub fn encode_and_write_audio_frame(
    frame_data: &AudioFrameData,
    opus_encoder: &mut Option<shiguredo_opus::Encoder>,
    opus_sample_entry: &mut Option<SampleEntry>,
    audio_pcm_buffer: &mut Vec<i16>,
    mp4_writer: &mut SimpleMp4Writer,
) -> Result<(), String> {
    let sample_rate = frame_data.sample_rate as u32;
    let channels = frame_data.channels as u8;

    // エンコーダーの遅延初期化
    if opus_encoder.is_none() {
        let mut config = shiguredo_opus::EncoderConfig::new(sample_rate, channels);
        config.frame_duration = Some(shiguredo_opus::FrameDuration::Ms10);
        let encoder = shiguredo_opus::Encoder::new(config)
            .map_err(|e| format!("failed to create Opus encoder: {e}"))?;
        let pre_skip = encoder
            .get_lookahead()
            .map_err(|e| format!("failed to get Opus lookahead: {e}"))?;
        *opus_sample_entry = Some(opus_sample_entry_value(channels, pre_skip));
        *opus_encoder = Some(encoder);
    }

    let encoder = opus_encoder.as_mut().unwrap();
    let frame_samples = encoder.frame_samples();
    let total_per_frame = frame_samples * channels as usize;

    audio_pcm_buffer.extend_from_slice(&frame_data.pcm);

    while audio_pcm_buffer.len() >= total_per_frame {
        let pcm: Vec<i16> = audio_pcm_buffer.drain(..total_per_frame).collect();
        let opus_data = encoder
            .encode(&pcm)
            .map_err(|e| format!("Opus encode failed: {e}"))?;
        let duration_us = (frame_samples as u64 * 1_000_000 / sample_rate as u64) as u32;
        let se = opus_sample_entry.take();
        mp4_writer.append_audio(&opus_data, se, duration_us)?;
    }
    Ok(())
}
