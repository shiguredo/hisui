use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use hisui::{
    audio::{
        AudioData, AudioDataReceiver, AudioDataSyncSender, AudioFormat, CHANNELS, SAMPLE_RATE,
    },
    channel::{self, ErrorFlag},
    layout::{AggregatedSourceInfo, Layout, Resolution},
    metadata::{SourceId, SourceInfo},
    mixer_audio::AudioMixerThread,
    stats::{AudioMixerStats, MixerStats, Seconds, SharedStats, Stats},
    video::FrameRate,
};
use orfail::OrFail;

#[test]
fn start_noop_audio_mixer() {
    let error_flag = ErrorFlag::new();
    let mut output_rx = AudioMixerThread::start(
        error_flag.clone(),
        layout(&[], None),
        Vec::new(),
        SharedStats::new(),
    );

    // ミキサーへの入力が空なので、出力も空
    assert!(output_rx.recv().is_none());

    // エラーは発生していない
    assert!(!error_flag.get());
}

#[test]
fn mix_three_sources_without_trim() -> orfail::Result<()> {
    let error_flag = ErrorFlag::new();
    let stats = SharedStats::new();

    // それぞれ期間が異なる三つのソース
    // trim はしないので、合成後の尺は 300 ms になる
    let (source0, input_tx0, input_rx0) = source(0, 0, 100); // 範囲: 0 ms ~ 100 ms
    let (source1, input_tx1, input_rx1) = source(1, 60, 160); // 範囲: 60 ms ~ 160 ms
    let (source2, input_tx2, input_rx2) = source(2, 200, 300); // 範囲: 200 ms ~ 300 ms

    let mut output_rx = AudioMixerThread::start(
        error_flag.clone(),
        layout(&[source0.clone(), source1.clone(), source2.clone()], None),
        vec![input_rx0, input_rx1, input_rx2],
        stats.clone(),
    );

    // ソースに AudioData を供給する
    let duration = Duration::from_millis(20); // このテストでは尺は固定
    for i in 0..5 {
        let sample = 2; // 音声サンプル（ソースで固定）
        input_tx0
            .send(audio_data(&source0, i, duration, sample))
            .or_fail()?;
        input_tx1
            .send(audio_data(&source1, i, duration, sample * 2))
            .or_fail()?;
        input_tx2
            .send(audio_data(&source2, i, duration, sample * 4))
            .or_fail()?;
    }
    std::mem::drop(input_tx0);
    std::mem::drop(input_tx1);
    std::mem::drop(input_tx2);

    // source0 だけが存在する期間: 0 ms ~ 60 ms
    for _ in 0..3 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202);
        }
    }

    // source0 と sourde1 が混在する期間: 60 ms ~ 100 ms
    for _ in 0..2 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202 + 0x0404);
        }
    }

    // source1 だけが存在する期間: 100 ms ~ 160 ms
    for _ in 0..3 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0404);
        }
    }

    // 空白期間: 160 ms ~ 200 ms
    for _ in 0..2 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0);
        }
    }

    // source2 だけが存在する期間: 200 ms ~ 300 ms
    for _ in 0..5 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0808);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(output_rx.recv().is_none());

    // エラーは発生していない
    assert!(!error_flag.get());

    // 統計値をチェックする
    stats.with_lock(|stats| {
        let stats = audio_mixer_stats(stats);
        assert!(!stats.error);
        assert_eq!(stats.total_input_audio_data_count, 15); // 100 ms * 3
        assert_eq!(stats.total_output_audio_data_count, 15); // 300 ms 分
        assert_eq!(stats.total_output_audio_data_seconds, ms(300));
        assert_eq!(
            stats.total_output_sample_count,
            (SAMPLE_RATE as f64 * 0.3) as u64
        );
        assert_eq!(
            stats.total_output_filled_sample_count,
            (SAMPLE_RATE as f64 * 0.04) as u64
        );

        // trim=false なのでトリム期間はなし
        assert_eq!(stats.total_trimmed_sample_count, 0);
    });

    Ok(())
}

#[test]
fn mix_three_sources_with_trim() -> orfail::Result<()> {
    let error_flag = ErrorFlag::new();
    let stats = SharedStats::new();

    // それぞれ期間が異なる三つのソース
    // trim をするので、合成後の尺は 260 ms になる
    let (source0, input_tx0, input_rx0) = source(0, 0, 100); // 範囲: 0 ms ~ 100 ms
    let (source1, input_tx1, input_rx1) = source(1, 60, 160); // 範囲: 60 ms ~ 160 ms
    let (source2, input_tx2, input_rx2) = source(2, 200, 300); // 範囲: 200 ms ~ 300 ms

    // 空白期間は除去する
    let trim_span = (Duration::from_millis(160), Duration::from_millis(200));

    let mut output_rx = AudioMixerThread::start(
        error_flag.clone(),
        layout(
            &[source0.clone(), source1.clone(), source2.clone()],
            Some(trim_span),
        ),
        vec![input_rx0, input_rx1, input_rx2],
        stats.clone(),
    );

    // ソースに AudioData を供給する
    let duration = Duration::from_millis(20); // このテストでは尺は固定
    for i in 0..5 {
        let sample = 2; // 音声サンプル（ソースで固定）
        input_tx0
            .send(audio_data(&source0, i, duration, sample))
            .or_fail()?;
        input_tx1
            .send(audio_data(&source1, i, duration, sample * 2))
            .or_fail()?;
        input_tx2
            .send(audio_data(&source2, i, duration, sample * 4))
            .or_fail()?;
    }
    std::mem::drop(input_tx0);
    std::mem::drop(input_tx1);
    std::mem::drop(input_tx2);

    // source0 だけが存在する期間: 0 ms ~ 60 ms
    for _ in 0..3 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202);
        }
    }

    // source0 と sourde1 が混在する期間: 60 ms ~ 100 ms
    for _ in 0..2 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0202 + 0x0404);
        }
    }

    // source1 だけが存在する期間: 100 ms ~ 160 ms
    for _ in 0..3 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0404);
        }
    }

    // source2 だけが存在する期間: 200 ms ~ 300 ms
    for _ in 0..5 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0808);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(output_rx.recv().is_none());

    // エラーは発生していない
    assert!(!error_flag.get());

    // 統計値をチェックする
    stats.with_lock(|stats| {
        let stats = audio_mixer_stats(stats);
        assert!(!stats.error);
        assert_eq!(stats.total_input_audio_data_count, 15); // 100 ms * 3
        assert_eq!(stats.total_output_audio_data_count, 13); // 260 ms 分
        assert_eq!(stats.total_output_audio_data_seconds, ms(260));
        assert_eq!(
            stats.total_output_sample_count,
            (SAMPLE_RATE as f64 * 0.26) as u64
        );
        assert_eq!(stats.total_output_filled_sample_count, 0);

        // 40 ms 分がトリムされた
        assert_eq!(
            stats.total_trimmed_sample_count,
            (SAMPLE_RATE as f64 * 0.04) as u64
        );
    });

    Ok(())
}

/// AudioData.duration がソース毎に異なる場合のテスト
#[test]
fn mix_three_sources_with_mixed_duration() -> orfail::Result<()> {
    let error_flag = ErrorFlag::new();
    let stats = SharedStats::new();

    // 100 ms のソースを三つ用意する
    let (source0, input_tx0, input_rx0) = source(0, 0, 100);
    let (source1, input_tx1, input_rx1) = source(1, 0, 100);
    let (source2, input_tx2, input_rx2) = source(2, 0, 100);

    let mut output_rx = AudioMixerThread::start(
        error_flag.clone(),
        layout(&[source0.clone(), source1.clone(), source2.clone()], None),
        vec![input_rx0, input_rx1, input_rx2],
        stats.clone(),
    );

    // それぞれのソースに AudioData を供給する
    for i in 0..10 {
        let sample = 2;
        let duration = Duration::from_millis(10); // 尺は 10 ms
        input_tx0
            .send(audio_data(&source0, i, duration, sample))
            .or_fail()?;
    }
    for i in 0..4 {
        let sample = 4;
        let duration = Duration::from_millis(25); // 尺は 25 ms
        input_tx1
            .send(audio_data(&source1, i, duration, sample))
            .or_fail()?;
    }
    for i in 0..50 {
        let sample = 8;
        let duration = Duration::from_millis(2); // 尺は 2 ms
        input_tx2
            .send(audio_data(&source2, i, duration, sample))
            .or_fail()?;
    }
    std::mem::drop(input_tx0);
    std::mem::drop(input_tx1);
    std::mem::drop(input_tx2);

    // 合成結果を確認する (合成後の AudioData.duraiton は 20 ms に固定）
    for _ in 0..5 {
        let audio_data = output_rx.recv().or_fail()?;
        for c in audio_data.data.chunks(2) {
            assert_eq!(i16::from_be_bytes([c[0], c[1]]), 0x0E0E);
        }
    }

    // 全てのソースの音声データの処理が終わったので、これ以上は出力もない
    assert!(output_rx.recv().is_none());

    // エラーは発生していない
    assert!(!error_flag.get());

    // 統計値をチェックする
    stats.with_lock(|stats| {
        let stats = audio_mixer_stats(stats);
        assert!(!stats.error);
        assert_eq!(stats.total_input_audio_data_count, 10 + 4 + 50);
        assert_eq!(stats.total_output_audio_data_count, 5); // 100 ms = 20 ms * 5
        assert_eq!(stats.total_output_audio_data_seconds, ms(100));
        assert_eq!(
            stats.total_output_sample_count,
            (SAMPLE_RATE as f64 * 0.1) as u64
        );

        // 空白期間はないので無音補完やトリムは発生しない
        assert_eq!(stats.total_output_filled_sample_count, 0);
        assert_eq!(stats.total_trimmed_sample_count, 0);
    });

    Ok(())
}

/// 不正なフォーマットの音声データを送るテスト
#[test]
fn non_pcm_audio_input_error() -> orfail::Result<()> {
    let error_flag = ErrorFlag::new();
    let stats = SharedStats::new();

    let (source, input_tx, input_rx) = source(0, 0, 100);
    let mut output_rx = AudioMixerThread::start(
        error_flag.clone(),
        layout(&[source.clone()], None),
        vec![input_rx],
        stats.clone(),
    );

    // 適当に不正なフォーマットを指定して AudioData を送る
    let duration = Duration::from_millis(20);
    let mut audio_data = audio_data(&source, 0, duration, 0);
    audio_data.format = AudioFormat::Opus;
    input_tx.send(audio_data).or_fail()?;
    std::mem::drop(input_tx);

    // エラーになるので、出力も存在しない
    assert!(output_rx.recv().is_none());

    // エラーは発生した
    assert!(error_flag.get());

    // 統計値をチェックする
    stats.with_lock(|stats| {
        let stats = audio_mixer_stats(stats);
        assert!(stats.error);
        assert_eq!(stats.total_input_audio_data_count, 0);
        assert_eq!(stats.total_output_audio_data_count, 0);
        assert_eq!(stats.total_output_audio_data_seconds, ms(0));
        assert_eq!(stats.total_output_sample_count, 0);
        assert_eq!(stats.total_output_filled_sample_count, 0);
        assert_eq!(stats.total_trimmed_sample_count, 0);
    });

    Ok(())
}

fn layout(audio_sources: &[SourceInfo], trim_span: Option<(Duration, Duration)>) -> Layout {
    Layout {
        trim_spans: if let Some((start, end)) = trim_span {
            [(start, end)].into_iter().collect()
        } else {
            BTreeMap::new()
        },
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
        bitrate_kbps: 0,
        fps: FrameRate::FPS_1,
        encode_params: Default::default(),
    }
}

fn source(
    id: usize,
    start_time_ms: u64,
    end_time_ms: u64,
) -> (SourceInfo, AudioDataSyncSender, AudioDataReceiver) {
    let source = SourceInfo {
        id: SourceId::new(&id.to_string()),
        start_timestamp: Duration::from_millis(start_time_ms),
        stop_timestamp: Duration::from_millis(end_time_ms),

        // 以下はダミー値
        audio: true,
        video: true,
        format: Default::default(),
    };

    // テストでは全てのソースが同じスレッドから送られるので、詰まらないようにチャネルの上限を大きくしておく
    let (tx, rx) = channel::sync_channel_with_bound(1000);
    (source, tx, rx)
}

fn audio_data(source: &SourceInfo, i: usize, duration: Duration, sample: u8) -> AudioData {
    let sample_bytes = 2; // 一つのサンプルは i16 で表現されるので 2 バイト
    let sample_count =
        (SAMPLE_RATE as f64 * duration.as_secs_f64()) as usize * CHANNELS as usize * sample_bytes;
    AudioData {
        source_id: Some(source.id.clone()),
        data: vec![sample; sample_count],
        format: AudioFormat::I16Be,
        stereo: true,
        sample_rate: SAMPLE_RATE,
        timestamp: source.start_timestamp + duration * i as u32,
        duration,
        sample_entry: None,
    }
}

fn ms(value: u64) -> Seconds {
    Seconds::new(Duration::from_millis(value))
}

fn audio_mixer_stats(stats: &Stats) -> &AudioMixerStats {
    stats
        .mixers
        .iter()
        .find_map(|x| {
            if let MixerStats::Audio(x) = x {
                Some(x)
            } else {
                None
            }
        })
        .expect("infallible")
}
