use std::time::Duration;

use hisui::{
    decoder_libvpx::LibvpxDecoder,
    decoder_opus::OpusDecoder,
    metadata::SourceId,
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    stats::{Mp4AudioReaderStats, Mp4VideoReaderStats},
    subcommand_legacy::{Args, Runner},
    types::CodecName,
};
use orfail::OrFail;

/// ソースが空の場合
#[test]
fn empty_source() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new().or_fail()?;
    let args = Args::parse(noargs::RawArgs::new(
        [
            "hisui",
            "--show-progress-bar=false",
            "-f",
            "testdata/e2e/empty_source/report.json",
            "--out-file",
            &out_file.path().display().to_string(),
        ]
        .into_iter()
        .map(|s| s.to_string()),
    ))?;
    Runner::new(args).run()?;

    // 結果ファイルを確認（映像・音声トラックが存在しない）
    assert!(out_file.path().exists());
    assert_eq!(
        Mp4AudioReader::new(SourceId::new("dummy"), out_file.path(), audio_stats())
            .or_fail()?
            .count(),
        0
    );
    assert_eq!(
        Mp4VideoReader::new(SourceId::new("dummy"), out_file.path(), video_stats())
            .or_fail()?
            .count(),
        0
    );

    Ok(())
}

// 共通のテスト関数
fn test_simple_single_source_common(
    report_path: &str,
    expected_codec: CodecName,
) -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new().or_fail()?;
    let args = Args::parse(noargs::RawArgs::new(
        [
            "hisui",
            "--show-progress-bar=false",
            "-f",
            report_path,
            "--out-video-codec",
            expected_codec.as_str(),
            "--out-file",
            &out_file.path().display().to_string(),
        ]
        .into_iter()
        .map(|s| s.to_string()),
    ))?;
    Runner::new(args).run()?;

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader =
        Mp4AudioReader::new(SourceId::new("dummy"), out_file.path(), audio_stats()).or_fail()?;
    let mut video_reader =
        Mp4VideoReader::new(SourceId::new("dummy"), out_file.path(), video_stats()).or_fail()?;

    // 後でデコードするために読み込み結果を覚えておく
    let audio_samples = audio_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;
    let video_samples = video_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert_eq!(audio_stats.codec, Some(CodecName::Opus));

    // 一秒分 + 一サンプル (25 ms)
    // => これは入力データのサンプル数と等しい
    assert_eq!(audio_stats.total_sample_count.get(), 51);
    assert_eq!(
        audio_stats.total_track_duration.get(),
        Duration::from_millis(1020)
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec.get(), Some(expected_codec));
    assert_eq!(
        video_stats
            .resolutions
            .get()
            .into_iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(320, 240)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count.get(), 25);
    assert_eq!(
        video_stats.total_track_duration.get(),
        Duration::from_secs(1)
    );

    // 音声をデコードをして中身を確認する
    let mut decoder = OpusDecoder::new().or_fail()?;
    for data in audio_samples {
        let decoded = decoder.decode(&data).or_fail()?;

        // 無音期間があるのは想定外
        assert!(!decoded.data.iter().all(|v| *v == 0));
    }

    // 映像をデコードをして中身を確認する
    let check_decoded_frames = |decoder: &mut LibvpxDecoder| -> orfail::Result<()> {
        while let Some(decoded) = decoder.next_decoded_frame() {
            // 画像が赤一色かどうかの確認する
            let (y_plane, u_plane, v_plane) = decoded.as_yuv_planes().or_fail()?;
            y_plane
                .iter()
                .for_each(|x| assert!(matches!(x, 80..=82), "y={x}"));
            u_plane.iter().for_each(|x| assert_eq!(*x, 90));
            v_plane.iter().for_each(|x| assert_eq!(*x, 240));
        }
        Ok(())
    };

    let mut decoder = LibvpxDecoder::new_vp9().or_fail()?;
    for frame in video_samples {
        decoder.decode(&frame).or_fail()?;
        check_decoded_frames(&mut decoder).or_fail()?;
    }
    decoder.finish().or_fail()?;
    check_decoded_frames(&mut decoder).or_fail()?;

    Ok(())
}

/// 単一のソースをそのまま変換する場合
/// - 入力:
///   - 映像:
///     - VP9
///     - 30 fps
///     - 320x240
///     - 赤一色
///   - 音声:
///     - OPUS
///     - ホワイトノイズ
/// - 出力:
///   - VP9, OPUS, 25 fps, 320x240
#[test]
fn simple_single_source() -> noargs::Result<()> {
    test_simple_single_source_common(
        "testdata/e2e/simple_single_source/report.json",
        CodecName::Vp9,
    )
}

/// 単一のソースをそのまま変換する場合 (H.265版)
/// - 入力:
///   - 映像:
///     - H.265
///     - 30 fps
///     - 320x240
///     - 赤一色
///   - 音声:
///     - OPUS
///     - ホワイトノイズ
/// - 出力:
///   - VP9, OPUS, 25 fps, 320x240
#[test]
#[cfg(any(feature = "nvcodec", target_os = "macos"))]
fn simple_single_source_h265() -> noargs::Result<()> {
    test_simple_single_source_common(
        "testdata/e2e/simple_single_source_h265/report.json",
        CodecName::H265,
    )
}

/// 単一のソースをそのまま変換する場合（奇数解像度版）
/// - 入力:
///   - 映像:
///     - VP9
///     - 30 fps
///     - 319x239
///     - 赤一色
///   - 音声:
///     - OPUS
///     - ホワイトノイズ
/// - 出力:
///   - VP9, OPUS, 25 fps, 319x239
#[test]
fn odd_resolution_single_source() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new().or_fail()?;
    let args = Args::parse(noargs::RawArgs::new(
        [
            "hisui",
            "--show-progress-bar=false",
            "-f",
            "testdata/e2e/odd_resolution_single_source/report.json",
            "--out-file",
            &out_file.path().display().to_string(),
        ]
        .into_iter()
        .map(|s| s.to_string()),
    ))?;
    Runner::new(args).run()?;

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader =
        Mp4AudioReader::new(SourceId::new("dummy"), out_file.path(), audio_stats()).or_fail()?;
    let mut video_reader =
        Mp4VideoReader::new(SourceId::new("dummy"), out_file.path(), video_stats()).or_fail()?;

    // 後でデコードするために読み込み結果を覚えておく
    let audio_samples = audio_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;
    let video_samples = video_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert_eq!(audio_stats.codec, Some(CodecName::Opus));

    // 一秒分 + 一サンプル (25 ms)
    // => これは入力データのサンプル数と等しい
    assert_eq!(audio_stats.total_sample_count.get(), 51);
    assert_eq!(
        audio_stats.total_track_duration.get(),
        Duration::from_millis(1020)
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec.get(), Some(CodecName::Vp9));
    assert_eq!(
        video_stats
            .resolutions
            .get()
            .into_iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        // 合成後は偶数解像度になる
        //（下と右に枠線が入る）
        [(320, 240)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count.get(), 25);
    assert_eq!(
        video_stats.total_track_duration.get(),
        Duration::from_secs(1)
    );

    // 音声をデコードをして中身を確認する
    let mut decoder = OpusDecoder::new().or_fail()?;
    for data in audio_samples {
        let decoded = decoder.decode(&data).or_fail()?;

        // 無音期間があるのは想定外
        assert!(!decoded.data.iter().all(|v| *v == 0));
    }

    // 映像をデコードをして中身を確認する
    let check_decoded_frames = |decoder: &mut LibvpxDecoder| -> orfail::Result<()> {
        while let Some(decoded) = decoder.next_decoded_frame() {
            // 画像が赤一色かどうかの確認する（ただし、右と下の枠線は黒色になる）
            let (y_plane, u_plane, v_plane) = decoded.as_yuv_planes().or_fail()?;

            y_plane.iter().enumerate().for_each(|(i, &x)| {
                let col = i % 320;
                let row = i / 320;
                if col >= 318 || row >= 238 {
                    assert!(matches!(x, 0..=3), "Expected black Y value, got y={x}",);
                } else {
                    assert!(matches!(x, 79..=83), "Expected red Y value, got y={x}",);
                }
            });

            u_plane.iter().enumerate().for_each(|(i, &x)| {
                let col = (i % 160) * 2;
                let row = (i / 160) * 2;
                if col >= 318 || row >= 238 {
                    assert!(matches!(x, 124..=131), "Expected black U value, got u={x}");
                } else {
                    assert!(matches!(x, 87..=95), "Expected red U value, got u={x}");
                }
            });

            v_plane.iter().enumerate().for_each(|(i, &x)| {
                let col = (i % 160) * 2;
                let row = (i / 160) * 2;
                if col >= 318 || row >= 238 {
                    assert!(matches!(x, 122..=131), "Expected black V value, got v={x}");
                } else {
                    assert!(matches!(x, 238..=244), "Expected red V value, got v={x}");
                }
            });
        }
        Ok(())
    };

    let mut decoder = LibvpxDecoder::new_vp9().or_fail()?;
    for frame in video_samples {
        decoder.decode(&frame).or_fail()?;
        check_decoded_frames(&mut decoder).or_fail()?;
    }
    decoder.finish().or_fail()?;
    check_decoded_frames(&mut decoder).or_fail()?;

    Ok(())
}

/// 複数のソースをレイアウト指定なしで変換する場合
#[test]
fn simple_multi_sources() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new().or_fail()?;
    let args = Args::parse(noargs::RawArgs::new(
        [
            "hisui",
            "--show-progress-bar=false",
            "-f",
            "testdata/e2e/simple_multi_sources/report.json",
            "--out-file",
            &out_file.path().display().to_string(),
        ]
        .into_iter()
        .map(|s| s.to_string()),
    ))?;
    Runner::new(args).run()?;

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader =
        Mp4AudioReader::new(SourceId::new("dummy"), out_file.path(), audio_stats()).or_fail()?;
    let mut video_reader =
        Mp4VideoReader::new(SourceId::new("dummy"), out_file.path(), video_stats()).or_fail()?;

    // [NOTE]
    // レイアウトファイル未指定だと映像の解像度が大きめになって
    // テスト内でデコード結果を確認するのが少し面倒なので、このテストでは省略している
    // （統計値を取得するためにイテレーターを最後まで実行する必要はある）
    let _audio_samples = audio_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;
    let _video_samples = video_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert_eq!(audio_stats.codec, Some(CodecName::Opus));

    // 一秒分 + 一サンプル (25 ms)
    // => これは入力データのサンプル数と等しい
    assert_eq!(audio_stats.total_sample_count.get(), 51);
    assert_eq!(
        audio_stats.total_track_duration.get(),
        Duration::from_millis(1020)
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec.get(), Some(CodecName::Vp9));

    // レイアウトファイル未指定の場合には、一つのセルの解像度は 320x240 で、
    // 今回はソースが三つなのでグリッドは 3x1 となり、
    // 以下の解像度になる
    assert_eq!(
        video_stats
            .resolutions
            .get()
            .into_iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(320 * 3, 240 * 1)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count.get(), 25);
    assert_eq!(
        video_stats.total_track_duration.get(),
        Duration::from_secs(1)
    );

    Ok(())
}

/// 分割録画の変換テスト
/// - 同一接続から時系列で分割された複数のソースファイル（R -> G -> B）を一つにまとめる
/// - 各ソースファイルは16x16の解像度
/// - レイアウトファイルで縦に並べて配置
#[test]
fn simple_split_archive() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new().or_fail()?;
    let args = Args::parse(noargs::RawArgs::new(
        [
            "hisui",
            "--show-progress-bar=false",
            "--layout",
            "testdata/e2e/simple_split_archive/layout.jsonc",
            "--out-file",
            &out_file.path().display().to_string(),
        ]
        .into_iter()
        .map(|s| s.to_string()),
    ))?;
    Runner::new(args).run()?;

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader =
        Mp4AudioReader::new(SourceId::new("dummy"), out_file.path(), audio_stats()).or_fail()?;
    let mut video_reader =
        Mp4VideoReader::new(SourceId::new("dummy"), out_file.path(), video_stats()).or_fail()?;

    // 後でデコードするために読み込み結果を覚えておく
    let audio_samples = audio_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;
    let video_samples = video_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert_eq!(audio_stats.codec, Some(CodecName::Opus));

    // 分割ファイルが3つ（各1秒）なので合計3秒分 + 3サンプル (25 ms * 3)
    assert_eq!(audio_stats.total_sample_count.get(), 153); // 51 * 3
    assert_eq!(
        audio_stats.total_track_duration.get(),
        Duration::from_millis(3060) // 1020 * 3
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec.get(), Some(CodecName::Vp9));
    assert_eq!(
        video_stats
            .resolutions
            .get()
            .into_iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(16, 16)] // 単一ソース（分割された部分）なので16x16
    );

    // 3秒分 (25 fps = 40 ms * 75フレーム)
    assert_eq!(video_stats.total_sample_count.get(), 75); // 25 * 3
    assert_eq!(
        video_stats.total_track_duration.get(),
        Duration::from_secs(3)
    );

    // 音声をデコードをして中身を確認する
    let mut decoder = OpusDecoder::new().or_fail()?;
    for data in audio_samples {
        let decoded = decoder.decode(&data).or_fail()?;

        // 無音期間があるのは想定外
        assert!(!decoded.data.iter().all(|v| *v == 0));
    }

    // 映像をデコードをして中身を確認する
    // 時系列順に R -> G -> B の色変化を確認
    let check_decoded_frames =
        |decoder: &mut LibvpxDecoder, frame_index: &mut usize| -> orfail::Result<()> {
            while let Some(decoded) = decoder.next_decoded_frame() {
                // Y成分だけを確認して色の変化を検証
                let (y_plane, _u_plane, v_plane) = decoded.as_yuv_planes().or_fail()?;

                // フレーム番号に基づいて期待される色を判定
                // 0-24: 赤, 25-49: 緑, 50-74: 青
                //
                // なお赤と緑は同じような Y 値でエンコードされているので、 Vの値も考慮している

                if *frame_index < 25 {
                    // 赤色の期間
                    (y_plane.iter().zip(v_plane.iter())).for_each(|(&y, &v)| {
                        assert!(
                            matches!(y, 80..=82) && matches!(v, 240),
                            "Expected red Y / V value, got y={y} / v={v} at frame {}",
                            *frame_index
                        );
                    });
                } else if *frame_index < 50 {
                    // 緑色の期間
                    (y_plane.iter().zip(v_plane.iter())).for_each(|(&y, &v)| {
                        assert!(
                            matches!(y, 80..=82) && matches!(v, 81),
                            "Expected green Y / V value, got y={y} / v={v} at frame {}",
                            *frame_index
                        );
                    });
                } else if *frame_index < 75 {
                    // 青色の期間
                    y_plane.iter().for_each(|&y| {
                        assert!(
                            matches!(y, 40..=42),
                            "Expected blue Y value, got y={y} at frame {}",
                            *frame_index
                        );
                    });
                }
                *frame_index += 1;
            }
            Ok(())
        };

    let mut decoder = LibvpxDecoder::new_vp9().or_fail()?;
    let mut frame_index = 0;
    for frame in video_samples {
        decoder.decode(&frame).or_fail()?;
        check_decoded_frames(&mut decoder, &mut frame_index).or_fail()?;
    }
    decoder.finish().or_fail()?;
    check_decoded_frames(&mut decoder, &mut frame_index).or_fail()?;

    // 全フレームが処理されたことを確認
    assert_eq!(frame_index, 75);

    Ok(())
}

/// 複数のソースをレイアウト指定で、縦に並べて変換する場合
#[test]
fn multi_sources_single_column() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new().or_fail()?;
    let args = Args::parse(noargs::RawArgs::new(
        [
            "hisui",
            "--show-progress-bar=false",
            "--layout",
            "testdata/e2e/multi_sources_single_column/layout.json",
            "--out-file",
            &out_file.path().display().to_string(),
        ]
        .into_iter()
        .map(|s| s.to_string()),
    ))?;
    Runner::new(args).run()?;

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader =
        Mp4AudioReader::new(SourceId::new("dummy"), out_file.path(), audio_stats()).or_fail()?;
    let mut video_reader =
        Mp4VideoReader::new(SourceId::new("dummy"), out_file.path(), video_stats()).or_fail()?;

    // 後でデコードするために読み込み結果を覚えておく
    let audio_samples = audio_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;
    let video_samples = video_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert_eq!(audio_stats.codec, Some(CodecName::Opus));

    // 一秒分 + 一サンプル (25 ms)
    // => これは入力データのサンプル数と等しい
    assert_eq!(audio_stats.total_sample_count.get(), 51);
    assert_eq!(
        audio_stats.total_track_duration.get(),
        Duration::from_millis(1020)
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec.get(), Some(CodecName::Vp9));
    assert_eq!(
        video_stats
            .resolutions
            .get()
            .into_iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(16, 52)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count.get(), 25);
    assert_eq!(
        video_stats.total_track_duration.get(),
        Duration::from_secs(1)
    );

    // 音声をデコードをして中身を確認する
    let mut decoder = OpusDecoder::new().or_fail()?;
    for data in audio_samples {
        let decoded = decoder.decode(&data).or_fail()?;

        // 無音期間があるのは想定外
        assert!(!decoded.data.iter().all(|v| *v == 0));
    }

    // 映像をデコードをして中身を確認する
    let check_decoded_frames = |decoder: &mut LibvpxDecoder| -> orfail::Result<()> {
        while let Some(decoded) = decoder.next_decoded_frame() {
            // 完全なチェックは面倒なので Y 成分だけを確認する
            let (y_plane, _u_plane, _v_plane) = decoded.as_yuv_planes().or_fail()?;

            let width = 16;
            for (i, y) in y_plane.iter().copied().enumerate() {
                if i / width < 16 {
                    // 最初の 16 行は青
                    assert!(matches!(y, 40..=42), "y={y}");
                } else if i / width < 16 + 2 {
                    // 次の 2 行は黒色（枠線）
                    assert!(matches!(y, 0..=2), "y={y}");
                } else if i / width < 16 + 2 + 16 {
                    // 次の 16 行は緑
                    assert!(matches!(y, 186 | 187 | 188 | 189), "y={y}");
                } else if i / width < 16 + 2 + 16 + 2 {
                    // 次の 2 行は黒色（枠線）
                    assert!(matches!(y, 0..=2), "y={y}");
                } else if i / width < 16 + 2 + 16 + 2 + 16 {
                    // 最後の 16 行は赤
                    assert!(matches!(y, 80..=82), "y={y}");
                } else {
                    unreachable!()
                }
            }
        }
        Ok(())
    };

    let mut decoder = LibvpxDecoder::new_vp9().or_fail()?;
    for frame in video_samples {
        decoder.decode(&frame).or_fail()?;
        check_decoded_frames(&mut decoder).or_fail()?;
    }
    decoder.finish().or_fail()?;
    check_decoded_frames(&mut decoder).or_fail()?;

    Ok(())
}

/// リージョンが二つあるレイアウトのテスト
/// - 全体の解像度は 16x34
/// - 一つ目のリージョンには縦並びの二つのセルがある（青と緑）
/// - 二つ目のリージョンは中央に一つのセルがある（赤） => 後ろに別のリージョンがあるので外枠がつく
/// - 音声ソースはなし
#[test]
fn two_regions() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new().or_fail()?;
    let args = Args::parse(noargs::RawArgs::new(
        [
            "hisui",
            "--show-progress-bar=false",
            "--layout",
            "testdata/e2e/two_regions/layout.json",
            "--out-file",
            &out_file.path().display().to_string(),
        ]
        .into_iter()
        .map(|s| s.to_string()),
    ))?;
    Runner::new(args).run()?;

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut video_reader =
        Mp4VideoReader::new(SourceId::new("dummy"), out_file.path(), video_stats()).or_fail()?;

    // 音声はなし
    assert_eq!(
        Mp4AudioReader::new(SourceId::new("dummy"), out_file.path(), audio_stats())
            .or_fail()?
            .count(),
        0
    );

    // 後でデコードするために読み込み結果を覚えておく
    let video_samples = video_reader.by_ref().collect::<orfail::Result<Vec<_>>>()?;

    // 統計値を確認
    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec.get(), Some(CodecName::Vp9));
    assert_eq!(
        video_stats
            .resolutions
            .get()
            .into_iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(16, 34)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count.get(), 25);
    assert_eq!(
        video_stats.total_track_duration.get(),
        Duration::from_secs(1)
    );

    // 映像をデコードをして中身を確認する
    let check_decoded_frames = |decoder: &mut LibvpxDecoder| -> orfail::Result<()> {
        while let Some(decoded) = decoder.next_decoded_frame() {
            // 完全なチェックは面倒なので Y 成分だけを確認する
            let (y_plane, _u_plane, _v_plane) = decoded.as_yuv_planes().or_fail()?;

            let width = 16;
            for (i, y) in y_plane.iter().copied().enumerate() {
                if i / width < 8 {
                    // 最初の 8 行は青
                    assert!(matches!(y, 40..=42), "y={y}");
                } else if i / width < 8 + 2 {
                    // 次の 2 行は黒色（枠線）
                    assert!(matches!(y, 0..=2), "y={y}");
                } else if i / width < 8 + 2 + 16 {
                    // 次の 16 行は赤
                    assert!(matches!(y, 80..=82), "y={y}");
                } else if i / width < 8 + 2 + 16 + 2 {
                    // 次の 2 行は黒色（枠線）
                    assert!(matches!(y, 0..=2), "y={y}");
                } else if i / width < 8 + 2 + 16 + 2 + 6 {
                    // 最後の 6 行は緑
                    assert!(matches!(y, 186..=188), "y={y}");
                } else {
                    unreachable!()
                }
            }
        }
        Ok(())
    };

    let mut decoder = LibvpxDecoder::new_vp9().or_fail()?;
    for frame in video_samples {
        decoder.decode(&frame).or_fail()?;
        check_decoded_frames(&mut decoder).or_fail()?;
    }
    decoder.finish().or_fail()?;
    check_decoded_frames(&mut decoder).or_fail()?;

    Ok(())
}

fn audio_stats() -> Mp4AudioReaderStats {
    Mp4AudioReaderStats {
        codec: Some(CodecName::Opus),
        ..Default::default()
    }
}

fn video_stats() -> Mp4VideoReaderStats {
    Mp4VideoReaderStats::default()
}
