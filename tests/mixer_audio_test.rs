use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use hisui::{
    TrackId,
    audio::{AudioFormat, AudioFrame, CHANNELS, SAMPLE_RATE},
    media::MediaFrame,
    sora_recording_layout::{AggregatedSourceInfo, Layout, Resolution, TrimSpans},
    sora_recording_metadata::{SourceId, SourceInfo},
    sora_recording_mixer_audio::{AudioMixer, AudioMixerOutput},
    types::CodecName,
    video::FrameRate,
};

const OUTPUT_TRACK_ID: &str = "mixer_audio_output";

#[test]
fn start_noop_audio_mixer() {
    let mut mixer = AudioMixer::new(
        layout(&[], None).trim_spans,
        Vec::new(),
        TrackId::new(OUTPUT_TRACK_ID),
        hisui::stats::Stats::new(),
    );

    // ミキサーへの入力が空なので、出力も空
    assert!(matches!(
        mixer.next_output(),
        Ok(AudioMixerOutput::Finished)
    ));
}

#[test]
fn mix_three_sources_without_trim() -> hisui::Result<()> {
    // それぞれ期間が異なる三つのソース
    // trim はしないので、合成後の尺は 300 ms になる
    let (source0, input_stream_id0) = source(0, 0, 100); // 範囲: 0 ms ~ 100 ms
    let (source1, input_stream_id1) = source(1, 60, 160); // 範囲: 60 ms ~ 160 ms
    let (source2, input_stream_id2) = source(2, 200, 300); // 範囲: 200 ms ~ 300 ms

    let mut mixer = AudioMixer::new(
        layout(&[source0.clone(), source1.clone(), source2.clone()], None).trim_spans,
        vec![
            input_stream_id0.clone(),
            input_stream_id1.clone(),
            input_stream_id2.clone(),
        ],
        TrackId::new(OUTPUT_TRACK_ID),
        hisui::stats::Stats::new(),
    );

    // ソースに AudioFrame を供給する
    let duration = Duration::from_millis(20); // このテストでは尺は固定
    for i in 0..5 {
        let sample = 2; // 音声サンプル（ソースで固定）
        push_input(
            &mut mixer,
            audio_data(&source0, &input_stream_id0, i, duration, sample),
        )?;
        push_input(
            &mut mixer,
            audio_data(&source1, &input_stream_id1, i, duration, sample * 2),
        )?;
        push_input(
            &mut mixer,
            audio_data(&source2, &input_stream_id2, i, duration, sample * 4),
        )?;
    }
    push_input(&mut mixer, eos(&input_stream_id0))?;
    push_input(&mut mixer, eos(&input_stream_id1))?;
    push_input(&mut mixer, eos(&input_stream_id2))?;

    // source0 だけが存在する期間: 0 ms ~ 60 ms
    for _ in 0..3 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202);
        }
    }

    // source0 と sourde1 が混在する期間: 60 ms ~ 100 ms
    for _ in 0..2 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202 + 0x0404);
        }
    }

    // source1 だけが存在する期間: 100 ms ~ 160 ms
    for _ in 0..3 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0404);
        }
    }

    // 空白期間: 160 ms ~ 200 ms
    for _ in 0..2 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0);
        }
    }

    // source2 だけが存在する期間: 200 ms ~ 300 ms
    for _ in 0..5 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0808);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(matches!(
        mixer.next_output(),
        Ok(AudioMixerOutput::Finished)
    ));

    // 統計値をチェックする
    let stats = mixer.stats();
    assert_eq!(stats.total_input_audio_data_count(), 15); // 100 ms * 3
    assert_eq!(stats.total_output_audio_data_count(), 15); // 300 ms 分
    assert_eq!(stats.total_output_audio_data_duration(), ms(300));
    assert_eq!(
        stats.total_output_sample_count(),
        (SAMPLE_RATE as f64 * 0.3) as u64
    );
    assert_eq!(
        stats.total_output_filled_sample_count(),
        (SAMPLE_RATE as f64 * 0.04) as u64
    );

    // trim=false なのでトリム期間はなし
    assert_eq!(stats.total_trimmed_sample_count(), 0);

    Ok(())
}

#[test]
fn mix_three_sources_with_trim() -> hisui::Result<()> {
    // それぞれ期間が異なる三つのソース
    // trim をするので、合成後の尺は 260 ms になる
    let (source0, input_stream_id0) = source(0, 0, 100); // 範囲: 0 ms ~ 100 ms
    let (source1, input_stream_id1) = source(1, 60, 160); // 範囲: 60 ms ~ 160 ms
    let (source2, input_stream_id2) = source(2, 200, 300); // 範囲: 200 ms ~ 300 ms

    // 空白期間は除去する
    let trim_span = (Duration::from_millis(160), Duration::from_millis(200));

    let mut mixer = AudioMixer::new(
        layout(
            &[source0.clone(), source1.clone(), source2.clone()],
            Some(trim_span),
        )
        .trim_spans,
        vec![
            input_stream_id0.clone(),
            input_stream_id1.clone(),
            input_stream_id2.clone(),
        ],
        TrackId::new(OUTPUT_TRACK_ID),
        hisui::stats::Stats::new(),
    );

    // ソースに AudioFrame を供給する
    let duration = Duration::from_millis(20); // このテストでは尺は固定
    for i in 0..5 {
        let sample = 2; // 音声サンプル（ソースで固定）
        push_input(
            &mut mixer,
            audio_data(&source0, &input_stream_id0, i, duration, sample),
        )?;
        push_input(
            &mut mixer,
            audio_data(&source1, &input_stream_id1, i, duration, sample * 2),
        )?;
        push_input(
            &mut mixer,
            audio_data(&source2, &input_stream_id2, i, duration, sample * 4),
        )?;
    }
    push_input(&mut mixer, eos(&input_stream_id0))?;
    push_input(&mut mixer, eos(&input_stream_id1))?;
    push_input(&mut mixer, eos(&input_stream_id2))?;

    // source0 だけが存在する期間: 0 ms ~ 60 ms
    for _ in 0..3 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202);
        }
    }

    // source0 と sourde1 が混在する期間: 60 ms ~ 100 ms
    for _ in 0..2 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202 + 0x0404);
        }
    }

    // source1 だけが存在する期間: 100 ms ~ 160 ms
    for _ in 0..3 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0404);
        }
    }

    // source2 だけが存在する期間: 200 ms ~ 300 ms
    for _ in 0..5 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0808);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(matches!(
        mixer.next_output(),
        Ok(AudioMixerOutput::Finished)
    ));

    // 統計値をチェックする
    let stats = mixer.stats();
    assert_eq!(stats.total_input_audio_data_count(), 15); // 100 ms * 3
    assert_eq!(stats.total_output_audio_data_count(), 13); // 260 ms 分
    assert_eq!(stats.total_output_audio_data_duration(), ms(260));
    assert_eq!(
        stats.total_output_sample_count(),
        (SAMPLE_RATE as f64 * 0.26) as u64
    );
    assert_eq!(stats.total_output_filled_sample_count(), 0);

    // 40 ms 分がトリムされた
    assert_eq!(
        stats.total_trimmed_sample_count(),
        (SAMPLE_RATE as f64 * 0.04) as u64
    );

    Ok(())
}

/// AudioFrame.duration がソース毎に異なる場合のテスト
#[test]
fn mix_three_sources_with_mixed_duration() -> hisui::Result<()> {
    // 100 ms のソースを三つ用意する
    let (source0, input_stream_id0) = source(0, 0, 100);
    let (source1, input_stream_id1) = source(1, 0, 100);
    let (source2, input_stream_id2) = source(2, 0, 100);

    let mut mixer = AudioMixer::new(
        layout(&[source0.clone(), source1.clone(), source2.clone()], None).trim_spans,
        vec![
            input_stream_id0.clone(),
            input_stream_id1.clone(),
            input_stream_id2.clone(),
        ],
        TrackId::new(OUTPUT_TRACK_ID),
        hisui::stats::Stats::new(),
    );

    // それぞれのソースに AudioFrame を供給する
    for i in 0..10 {
        let sample = 2;
        let duration = Duration::from_millis(10); // 尺は 10 ms
        push_input(
            &mut mixer,
            audio_data(&source0, &input_stream_id0, i, duration, sample),
        )?;
    }
    for i in 0..4 {
        let sample = 4;
        let duration = Duration::from_millis(25); // 尺は 25 ms
        push_input(
            &mut mixer,
            audio_data(&source1, &input_stream_id1, i, duration, sample),
        )?;
    }
    for i in 0..50 {
        let sample = 8;
        let duration = Duration::from_millis(2); // 尺は 2 ms
        push_input(
            &mut mixer,
            audio_data(&source2, &input_stream_id2, i, duration, sample),
        )?;
    }
    push_input(&mut mixer, eos(&input_stream_id0))?;
    push_input(&mut mixer, eos(&input_stream_id1))?;
    push_input(&mut mixer, eos(&input_stream_id2))?;

    // 合成結果を確認する (合成後の AudioFrame.duraiton は 20 ms に固定）
    for _ in 0..5 {
        let audio_data = next_mixed_data(&mut mixer)?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0E0E);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(matches!(
        mixer.next_output(),
        Ok(AudioMixerOutput::Finished)
    ));

    // 統計値をチェックする
    let stats = mixer.stats();
    assert_eq!(stats.total_input_audio_data_count(), 10 + 4 + 50);
    assert_eq!(stats.total_output_audio_data_count(), 5); // 100 ms = 20 ms * 5
    assert_eq!(stats.total_output_audio_data_duration(), ms(100));
    assert_eq!(
        stats.total_output_sample_count(),
        (SAMPLE_RATE as f64 * 0.1) as u64
    );

    // 空白期間はないので無音補完やトリムは発生しない
    assert_eq!(stats.total_output_filled_sample_count(), 0);
    assert_eq!(stats.total_trimmed_sample_count(), 0);

    Ok(())
}

/// 不正なフォーマットの音声データを送るテスト
#[test]
fn non_pcm_audio_input_error() -> hisui::Result<()> {
    let (source, input_stream_id) = source(0, 0, 100);
    let mut mixer = AudioMixer::new(
        layout(std::slice::from_ref(&source), None).trim_spans,
        vec![input_stream_id.clone()],
        TrackId::new(OUTPUT_TRACK_ID),
        hisui::stats::Stats::new(),
    );

    // 適当に不正なフォーマットを指定して AudioFrame を送る
    let duration = Duration::from_millis(20);
    let mut input = audio_data(&source, &input_stream_id, 0, duration, 0);
    if let Some(MediaFrame::Audio(audio_data)) = &mut input.sample {
        let audio_data = Arc::make_mut(audio_data);
        audio_data.format = AudioFormat::Opus;
    }

    // 不正なフォーマットのデータを送信
    assert!(push_input(&mut mixer, input).is_err());
    push_input(&mut mixer, eos(&input_stream_id))?;

    // エラーになるので、出力も存在しない
    assert!(matches!(
        mixer.next_output(),
        Ok(AudioMixerOutput::Finished)
    ));

    // 統計値をチェックする
    let stats = mixer.stats();
    assert_eq!(stats.total_input_audio_data_count(), 0);
    assert_eq!(stats.total_output_audio_data_count(), 0);
    assert_eq!(stats.total_output_audio_data_duration(), ms(0));
    assert_eq!(stats.total_output_sample_count(), 0);
    assert_eq!(stats.total_output_filled_sample_count(), 0);
    assert_eq!(stats.total_trimmed_sample_count(), 0);

    Ok(())
}

fn layout(audio_sources: &[SourceInfo], trim_span: Option<(Duration, Duration)>) -> Layout {
    Layout {
        trim_spans: TrimSpans::new(if let Some((start, end)) = trim_span {
            [(start, end)].into_iter().collect()
        } else {
            BTreeMap::new()
        }),
        audio_source_ids: audio_sources.iter().map(|s| s.id.clone()).collect(),
        sources: audio_sources
            .iter()
            .map(|s| {
                (
                    s.id.clone(),
                    AggregatedSourceInfo {
                        id: s.id.clone(),
                        start_timestamp: s.start_timestamp,
                        stop_timestamp: s.stop_timestamp,
                        audio: true,
                        video: true,
                        format: Default::default(),
                        media_paths: Default::default(),
                    },
                )
            })
            .collect(),

        // 以下のフィールドはテストで使われないので、適当な値を設定しておく
        base_path: PathBuf::from(""),
        video_regions: Vec::new(),
        resolution: Resolution::new(16, 16).expect("infallible"),
        frame_rate: FrameRate::FPS_1,
        audio_codec: CodecName::Opus,
        video_codec: CodecName::Vp8,
        audio_bitrate: None,
        video_bitrate: None,
        encode_params: Default::default(),
        decode_params: Default::default(),
        video_encode_engines: None,
        video_decode_engines: None,
    }
}

fn source(id: usize, start_time_ms: u64, end_time_ms: u64) -> (SourceInfo, TrackId) {
    let source = SourceInfo {
        id: SourceId::new(&id.to_string()),
        start_timestamp: Duration::from_millis(start_time_ms),
        stop_timestamp: Duration::from_millis(end_time_ms),

        // 以下はダミー値
        audio: true,
        video: true,
        format: Default::default(),
    };
    (source, TrackId::new(format!("mixer_audio_input_{id}")))
}

#[derive(Debug)]
struct MixerInput {
    track_id: TrackId,
    sample: Option<MediaFrame>,
}

fn audio_data(
    source: &SourceInfo,
    track_id: &TrackId,
    i: usize,
    duration: Duration,
    sample: u8,
) -> MixerInput {
    let sample_bytes = 2; // 一つのサンプルは i16 で表現されるので 2 バイト
    let sample_count =
        (SAMPLE_RATE as f64 * duration.as_secs_f64()) as usize * CHANNELS as usize * sample_bytes;
    let data = AudioFrame {
        data: vec![sample; sample_count],
        format: AudioFormat::I16Be,
        stereo: true,
        sample_rate: SAMPLE_RATE,
        timestamp: source.start_timestamp + duration * i as u32,
        duration,
        sample_entry: None,
    };
    MixerInput {
        track_id: track_id.clone(),
        sample: Some(MediaFrame::audio(data)),
    }
}

fn eos(track_id: &TrackId) -> MixerInput {
    MixerInput {
        track_id: track_id.clone(),
        sample: None,
    }
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn push_input(mixer: &mut AudioMixer, input: MixerInput) -> hisui::Result<()> {
    mixer.push_input(input.track_id, input.sample)
}

fn next_mixed_data(mixer: &mut AudioMixer) -> hisui::Result<Arc<AudioFrame>> {
    match mixer.next_output()? {
        AudioMixerOutput::Processed(sample) => sample.expect_audio(),
        AudioMixerOutput::Pending(track_id) => Err(hisui::Error::new(format!(
            "audio mixer is unexpectedly pending on track {}",
            track_id
        ))),
        AudioMixerOutput::Finished => Err(hisui::Error::new("audio mixer finished unexpectedly")),
    }
}
