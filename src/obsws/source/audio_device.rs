use std::time::Duration;

use crate::{AudioFrame, Error, ProcessorHandle, Result, TrackId};

#[derive(Debug, Clone)]
pub struct AudioDeviceSource {
    pub output_audio_track_id: TrackId,
    pub device_id: Option<String>,
    pub sample_rate: Option<i32>,
    pub channels: Option<i32>,
}

impl AudioDeviceSource {
    pub async fn run(self, handle: ProcessorHandle) -> Result<()> {
        let mut output_audio_sender = handle
            .publish_track(self.output_audio_track_id.clone())
            .await
            .map_err(|e| {
                Error::new(format!(
                    "failed to publish output audio track {}: {e}",
                    self.output_audio_track_id
                ))
            })?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let default_config = shiguredo_audio_device::AudioCaptureConfig::default();
        let config = shiguredo_audio_device::AudioCaptureConfig {
            device_id: self.device_id.clone(),
            sample_rate: self.sample_rate.unwrap_or(default_config.sample_rate),
            channels: self.channels.unwrap_or(default_config.channels),
        };

        let (frame_tx, mut frame_rx) =
            tokio::sync::mpsc::unbounded_channel::<shiguredo_audio_device::AudioFrameOwned>();
        let mut capture = shiguredo_audio_device::AudioCapture::new(config, move |frame| {
            let _ = frame_tx.send(frame.to_owned());
        })
        .map_err(|e| Error::new(format!("failed to create audio capture session: {e}")))?;
        capture
            .start()
            .map_err(|e| Error::new(format!("failed to start audio capture session: {e}")))?;

        while let Some(captured_frame) = frame_rx.recv().await {
            let frame = convert_captured_frame_to_i16be(&captured_frame)?;
            if !output_audio_sender.send_audio(frame) {
                break;
            }
        }

        // NOTE: エラーによる早期リターン時も AudioCapture::drop() が stop() を呼ぶため安全
        capture.stop();
        output_audio_sender.send_eos();

        Ok(())
    }
}

fn convert_captured_frame_to_i16be(
    frame: &shiguredo_audio_device::AudioFrameOwned,
) -> Result<AudioFrame> {
    let timestamp = if frame.timestamp_us < 0 {
        Duration::ZERO
    } else {
        Duration::from_micros(frame.timestamp_us as u64)
    };

    let channels = crate::audio::Channels::from_u8(
        u8::try_from(frame.channels)
            .map_err(|_| Error::new(format!("invalid channel count: {}", frame.channels)))?,
    )?;

    let sample_rate = crate::audio::SampleRate::from_u32(
        u32::try_from(frame.sample_rate)
            .map_err(|_| Error::new(format!("invalid sample rate: {}", frame.sample_rate)))?,
    )?;

    let i16be_data = match frame.format {
        shiguredo_audio_device::AudioFormat::S16 => {
            // S16 LE → I16 BE: エンディアン変換
            if !frame.data.len().is_multiple_of(2) {
                return Err(Error::new(format!(
                    "invalid S16 audio data length: {}",
                    frame.data.len()
                )));
            }
            frame
                .data
                .chunks_exact(2)
                .flat_map(|chunk| {
                    let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                    sample.to_be_bytes()
                })
                .collect()
        }
        shiguredo_audio_device::AudioFormat::F32 => {
            // F32 → I16 BE: スケーリング + クランプ + エンディアン変換
            if !frame.data.len().is_multiple_of(4) {
                return Err(Error::new(format!(
                    "invalid F32 audio data length: {}",
                    frame.data.len()
                )));
            }
            frame
                .data
                .chunks_exact(4)
                .flat_map(|chunk| {
                    let sample_f32 = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    let clamped = (sample_f32 * 32767.0).clamp(-32767.0, 32767.0);
                    (clamped as i16).to_be_bytes()
                })
                .collect()
        }
    };

    Ok(AudioFrame {
        data: i16be_data,
        format: crate::audio::AudioFormat::I16Be,
        channels,
        sample_rate,
        timestamp,
        sample_entry: None,
    })
}

/// source processor を起動できる設定が揃っているかを返す
pub(super) fn is_source_startable(
    settings: &crate::obsws::input_registry::ObswsAudioCaptureDeviceSettings,
) -> bool {
    settings.device_id.is_some()
}

pub(super) fn build_record_source_plan(
    settings: &crate::obsws::input_registry::ObswsAudioCaptureDeviceSettings,
    output_kind: super::ObswsOutputKind,
    run_id: u64,
    source_key: &str,
) -> std::result::Result<super::ObswsRecordSourcePlan, super::BuildObswsRecordSourcePlanError> {
    let kind = output_kind.as_str();
    let source_processor_id = crate::ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:audio_device_source"
    ));
    let raw_audio_track_id = crate::TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:raw_audio"
    ));

    let source = AudioDeviceSource {
        output_audio_track_id: raw_audio_track_id.clone(),
        device_id: settings.device_id.clone(),
        sample_rate: settings.sample_rate,
        channels: settings.channels,
    };

    Ok(super::ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: None,
        source_audio_track_id: Some(raw_audio_track_id),
        requests: vec![super::ObswsSourceRequest::CreateAudioDeviceSource {
            source,
            processor_id: Some(source_processor_id),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws::input_registry::ObswsAudioCaptureDeviceSettings;
    use crate::obsws::source::{ObswsOutputKind, ObswsSourceRequest};

    #[test]
    fn build_record_source_plan_with_device_id() {
        let plan = build_record_source_plan(
            &ObswsAudioCaptureDeviceSettings {
                device_id: Some("mic0".to_owned()),
                sample_rate: None,
                channels: None,
            },
            ObswsOutputKind::Program,
            1,
            "0",
        )
        .expect("audio_capture_device source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 1);
        assert_eq!(
            plan.source_processor_ids[0].get(),
            "obsws:program:1:source:0:audio_device_source"
        );

        assert_eq!(plan.requests.len(), 1);

        assert!(plan.source_video_track_id.is_none());
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("obsws:program:1:source:0:raw_audio")
        );

        match &plan.requests[0] {
            ObswsSourceRequest::CreateAudioDeviceSource {
                source,
                processor_id,
            } => {
                assert_eq!(
                    source.output_audio_track_id.get(),
                    "obsws:program:1:source:0:raw_audio"
                );
                assert_eq!(source.device_id.as_deref(), Some("mic0"));
                assert_eq!(
                    processor_id.as_ref().map(|p| p.get()),
                    Some("obsws:program:1:source:0:audio_device_source")
                );
            }
            _ => panic!("expected CreateAudioDeviceSource"),
        }
    }

    #[test]
    fn build_record_source_plan_without_device_id_keeps_input_dormant() {
        let settings = ObswsAudioCaptureDeviceSettings {
            device_id: None,
            sample_rate: None,
            channels: None,
        };
        let plan = build_record_source_plan(&settings, ObswsOutputKind::Program, 2, "1")
            .expect("audio_capture_device source plan without device_id must succeed");

        assert!(
            !is_source_startable(&settings),
            "audio_capture_device without device_id must remain dormant"
        );

        match &plan.requests[0] {
            ObswsSourceRequest::CreateAudioDeviceSource { source, .. } => {
                assert_eq!(source.device_id, None);
            }
            _ => panic!("expected CreateAudioDeviceSource"),
        }
    }

    #[test]
    fn convert_captured_frame_to_i16be_s16_input() {
        // S16 LE: サンプル値 256 = [0x00, 0x01] (LE) → [0x01, 0x00] (BE)
        let captured = shiguredo_audio_device::AudioFrameOwned {
            data: vec![0x00, 0x01, 0x00, 0x02],
            frames: 2,
            channels: 1,
            sample_rate: 48000,
            format: shiguredo_audio_device::AudioFormat::S16,
            timestamp_us: 1_000_000,
        };

        let frame = convert_captured_frame_to_i16be(&captured).expect("convert");

        assert_eq!(frame.format, crate::audio::AudioFormat::I16Be);
        assert_eq!(frame.data, vec![0x01, 0x00, 0x02, 0x00]);
        assert_eq!(frame.channels, crate::audio::Channels::MONO);
        assert_eq!(frame.sample_rate, crate::audio::SampleRate::HZ_48000);
        assert_eq!(frame.timestamp, Duration::from_secs(1));
    }

    #[test]
    fn convert_captured_frame_to_i16be_f32_input() {
        // F32 LE: 1.0 → 32767, -1.0 → -32767
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&(-1.0f32).to_le_bytes());

        let captured = shiguredo_audio_device::AudioFrameOwned {
            data,
            frames: 2,
            channels: 1,
            sample_rate: 48000,
            format: shiguredo_audio_device::AudioFormat::F32,
            timestamp_us: 500_000,
        };

        let frame = convert_captured_frame_to_i16be(&captured).expect("convert");

        assert_eq!(frame.format, crate::audio::AudioFormat::I16Be);
        // 1.0 * 32767.0 = 32767 → BE bytes
        let expected_pos = 32767i16.to_be_bytes();
        // -1.0 * 32767.0 = -32767.0, clamp → -32767 → BE bytes
        let expected_neg = (-32767i16).to_be_bytes();
        assert_eq!(
            frame.data,
            [expected_pos.as_slice(), expected_neg.as_slice()].concat()
        );
        assert_eq!(frame.timestamp, Duration::from_micros(500_000));
    }

    #[test]
    fn convert_captured_frame_to_i16be_rejects_odd_s16_data() {
        let captured = shiguredo_audio_device::AudioFrameOwned {
            data: vec![0x00, 0x01, 0x02],
            frames: 1,
            channels: 1,
            sample_rate: 48000,
            format: shiguredo_audio_device::AudioFormat::S16,
            timestamp_us: 0,
        };

        let error = convert_captured_frame_to_i16be(&captured).expect_err("must fail");
        assert!(error.reason.contains("invalid S16 audio data length"));
    }

    #[test]
    fn convert_captured_frame_to_i16be_rejects_invalid_f32_data() {
        let captured = shiguredo_audio_device::AudioFrameOwned {
            data: vec![0x00, 0x01, 0x02],
            frames: 1,
            channels: 1,
            sample_rate: 48000,
            format: shiguredo_audio_device::AudioFormat::F32,
            timestamp_us: 0,
        };

        let error = convert_captured_frame_to_i16be(&captured).expect_err("must fail");
        assert!(error.reason.contains("invalid F32 audio data length"));
    }
}
