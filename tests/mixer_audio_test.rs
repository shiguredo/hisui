use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use hisui::{
    audio::{AudioData, AudioFormat, CHANNELS, SAMPLE_RATE},
    layout::{AggregatedSourceInfo, Layout, Resolution, TrimSpans},
    media::{MediaSample, MediaStreamId},
    metadata::{SourceId, SourceInfo},
    mixer_audio::AudioMixer,
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput},
    types::CodecName,
    video::FrameRate,
};
use orfail::OrFail;

const OUTPUT_STREAM_ID: MediaStreamId = MediaStreamId::new(100);

#[test]
fn start_noop_audio_mixer() {
    let mut mixer = AudioMixer::new(layout(&[], None).trim_spans, Vec::new(), OUTPUT_STREAM_ID);

    // ミキサーへの入力が空なので、出力も空
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));
}

#[test]
fn mix_three_sources_without_trim() -> orfail::Result<()> {
    // それぞれ期間が異なる三つのソース
    // trim はしないので、合成後の尺は 300 ms になる
    let (source0, input_stream_id0) = source(0, 0, 100); // 範囲: 0 ms ~ 100 ms
    let (source1, input_stream_id1) = source(1, 60, 160); // 範囲: 60 ms ~ 160 ms
    let (source2, input_stream_id2) = source(2, 200, 300); // 範囲: 200 ms ~ 300 ms

    let mut mixer = AudioMixer::new(
        layout(&[source0.clone(), source1.clone(), source2.clone()], None).trim_spans,
        vec![input_stream_id0, input_stream_id1, input_stream_id2],
        OUTPUT_STREAM_ID,
    );

    // ソースに AudioData を供給する
    let duration = Duration::from_millis(20); // このテストでは尺は固定
    for i in 0..5 {
        let sample = 2; // 音声サンプル（ソースで固定）
        mixer
            .process_input(audio_data(&source0, i, duration, sample))
            .or_fail()?;
        mixer
            .process_input(audio_data(&source1, i, duration, sample * 2))
            .or_fail()?;
        mixer
            .process_input(audio_data(&source2, i, duration, sample * 4))
            .or_fail()?;
    }
    mixer.process_input(eos(0)).or_fail()?;
    mixer.process_input(eos(1)).or_fail()?;
    mixer.process_input(eos(2)).or_fail()?;

    // source0 だけが存在する期間: 0 ms ~ 60 ms
    for _ in 0..3 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202);
        }
    }

    // source0 と sourde1 が混在する期間: 60 ms ~ 100 ms
    for _ in 0..2 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202 + 0x0404);
        }
    }

    // source1 だけが存在する期間: 100 ms ~ 160 ms
    for _ in 0..3 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0404);
        }
    }

    // 空白期間: 160 ms ~ 200 ms
    for _ in 0..2 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0);
        }
    }

    // source2 だけが存在する期間: 200 ms ~ 300 ms
    for _ in 0..5 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0808);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計値をチェックする
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_audio_data_count.get(), 15); // 100 ms * 3
    assert_eq!(stats.total_output_audio_data_count.get(), 15); // 300 ms 分
    assert_eq!(stats.total_output_audio_data_duration.get(), ms(300));
    assert_eq!(
        stats.total_output_sample_count.get(),
        (SAMPLE_RATE as f64 * 0.3) as u64
    );
    assert_eq!(
        stats.total_output_filled_sample_count.get(),
        (SAMPLE_RATE as f64 * 0.04) as u64
    );

    // trim=false なのでトリム期間はなし
    assert_eq!(stats.total_trimmed_sample_count.get(), 0);

    Ok(())
}

#[test]
fn mix_three_sources_with_trim() -> orfail::Result<()> {
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
        vec![input_stream_id0, input_stream_id1, input_stream_id2],
        OUTPUT_STREAM_ID,
    );

    // ソースに AudioData を供給する
    let duration = Duration::from_millis(20); // このテストでは尺は固定
    for i in 0..5 {
        let sample = 2; // 音声サンプル（ソースで固定）
        mixer
            .process_input(audio_data(&source0, i, duration, sample))
            .or_fail()?;
        mixer
            .process_input(audio_data(&source1, i, duration, sample * 2))
            .or_fail()?;
        mixer
            .process_input(audio_data(&source2, i, duration, sample * 4))
            .or_fail()?;
    }
    mixer.process_input(eos(0)).or_fail()?;
    mixer.process_input(eos(1)).or_fail()?;
    mixer.process_input(eos(2)).or_fail()?;

    // source0 だけが存在する期間: 0 ms ~ 60 ms
    for _ in 0..3 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202);
        }
    }

    // source0 と sourde1 が混在する期間: 60 ms ~ 100 ms
    for _ in 0..2 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202 + 0x0404);
        }
    }

    // source1 だけが存在する期間: 100 ms ~ 160 ms
    for _ in 0..3 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0404);
        }
    }

    // source2 だけが存在する期間: 200 ms ~ 300 ms
    for _ in 0..5 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0808);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計値をチェックする
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_audio_data_count.get(), 15); // 100 ms * 3
    assert_eq!(stats.total_output_audio_data_count.get(), 13); // 260 ms 分
    assert_eq!(stats.total_output_audio_data_duration.get(), ms(260));
    assert_eq!(
        stats.total_output_sample_count.get(),
        (SAMPLE_RATE as f64 * 0.26) as u64
    );
    assert_eq!(stats.total_output_filled_sample_count.get(), 0);

    // 40 ms 分がトリムされた
    assert_eq!(
        stats.total_trimmed_sample_count.get(),
        (SAMPLE_RATE as f64 * 0.04) as u64
    );

    Ok(())
}

/// AudioData.duration がソース毎に異なる場合のテスト
#[test]
fn mix_three_sources_with_mixed_duration() -> orfail::Result<()> {
    // 100 ms のソースを三つ用意する
    let (source0, input_stream_id0) = source(0, 0, 100);
    let (source1, input_stream_id1) = source(1, 0, 100);
    let (source2, input_stream_id2) = source(2, 0, 100);

    let mut mixer = AudioMixer::new(
        layout(&[source0.clone(), source1.clone(), source2.clone()], None).trim_spans,
        vec![input_stream_id0, input_stream_id1, input_stream_id2],
        OUTPUT_STREAM_ID,
    );

    // それぞれのソースに AudioData を供給する
    for i in 0..10 {
        let sample = 2;
        let duration = Duration::from_millis(10); // 尺は 10 ms
        mixer
            .process_input(audio_data(&source0, i, duration, sample))
            .or_fail()?;
    }
    for i in 0..4 {
        let sample = 4;
        let duration = Duration::from_millis(25); // 尺は 25 ms
        mixer
            .process_input(audio_data(&source1, i, duration, sample))
            .or_fail()?;
    }
    for i in 0..50 {
        let sample = 8;
        let duration = Duration::from_millis(2); // 尺は 2 ms
        mixer
            .process_input(audio_data(&source2, i, duration, sample))
            .or_fail()?;
    }
    mixer.process_input(eos(0)).or_fail()?;
    mixer.process_input(eos(1)).or_fail()?;
    mixer.process_input(eos(2)).or_fail()?;

    // 合成結果を確認する (合成後の AudioData.duraiton は 20 ms に固定）
    for _ in 0..5 {
        let audio_data = next_mixed_data(&mut mixer).or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0E0E);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計値をチェックする
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_audio_data_count.get(), 10 + 4 + 50);
    assert_eq!(stats.total_output_audio_data_count.get(), 5); // 100 ms = 20 ms * 5
    assert_eq!(stats.total_output_audio_data_duration.get(), ms(100));
    assert_eq!(
        stats.total_output_sample_count.get(),
        (SAMPLE_RATE as f64 * 0.1) as u64
    );

    // 空白期間はないので無音補完やトリムは発生しない
    assert_eq!(stats.total_output_filled_sample_count.get(), 0);
    assert_eq!(stats.total_trimmed_sample_count.get(), 0);

    Ok(())
}

/// 不正なフォーマットの音声データを送るテスト
#[test]
fn non_pcm_audio_input_error() -> orfail::Result<()> {
    let (source, input_stream_id) = source(0, 0, 100);
    let mut mixer = AudioMixer::new(
        layout(&[source.clone()], None).trim_spans,
        vec![input_stream_id],
        OUTPUT_STREAM_ID,
    );

    // 適当に不正なフォーマットを指定して AudioData を送る
    let duration = Duration::from_millis(20);
    let mut input = audio_data(&source, 0, duration, 0);
    // MediaProcessorInput から AudioData を取得して format を変更
    if let Some(MediaSample::Audio(audio_data)) = &mut input.sample {
        let audio_data = Arc::make_mut(audio_data);
        audio_data.format = AudioFormat::Opus;
    }

    // 不正なフォーマットのデータを送信
    assert!(mixer.process_input(input).is_err());
    mixer.process_input(eos(0)).or_fail()?;

    // エラーになるので、出力も存在しない
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計値をチェックする
    let stats = mixer.stats();
    assert!(!stats.error.get()); // このフラグはスケジューラ側で管理しているので、ここでは `true` にならない
    assert_eq!(stats.total_input_audio_data_count.get(), 0);
    assert_eq!(stats.total_output_audio_data_count.get(), 0);
    assert_eq!(stats.total_output_audio_data_duration.get(), ms(0));
    assert_eq!(stats.total_output_sample_count.get(), 0);
    assert_eq!(stats.total_output_filled_sample_count.get(), 0);
    assert_eq!(stats.total_trimmed_sample_count.get(), 0);

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

fn source(id: usize, start_time_ms: u64, end_time_ms: u64) -> (SourceInfo, MediaStreamId) {
    let source = SourceInfo {
        id: SourceId::new(&id.to_string()),
        start_timestamp: Duration::from_millis(start_time_ms),
        stop_timestamp: Duration::from_millis(end_time_ms),

        // 以下はダミー値
        audio: true,
        video: true,
        format: Default::default(),
    };
    (source, MediaStreamId::new(id as u64))
}

fn audio_data(
    source: &SourceInfo,
    i: usize,
    duration: Duration,
    sample: u8,
) -> MediaProcessorInput {
    let sample_bytes = 2; // 一つのサンプルは i16 で表現されるので 2 バイト
    let sample_count =
        (SAMPLE_RATE as f64 * duration.as_secs_f64()) as usize * CHANNELS as usize * sample_bytes;
    let data = AudioData {
        source_id: Some(source.id.clone()),
        data: vec![sample; sample_count],
        format: AudioFormat::I16Be,
        stereo: true,
        sample_rate: SAMPLE_RATE,
        timestamp: source.start_timestamp + duration * i as u32,
        duration,
        sample_entry: None,
    };
    let id = MediaStreamId::new(source.id.get().parse().expect("infallible"));
    MediaProcessorInput::audio_data(id, data)
}

fn eos(i: usize) -> MediaProcessorInput {
    MediaProcessorInput::eos(MediaStreamId::new(i as u64))
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn next_mixed_data(mixer: &mut AudioMixer) -> orfail::Result<Arc<AudioData>> {
    mixer
        .process_output()
        .or_fail()?
        .expect_processed()
        .or_fail()?
        .1
        .expect_audio_data()
        .or_fail()
}
