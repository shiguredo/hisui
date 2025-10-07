use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroUsize,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use hisui::{
    layout::{AggregatedSourceInfo, Layout, Resolution, TrimSpans},
    layout_region::{Grid, Region},
    media::MediaStreamId,
    metadata::{SourceId, SourceInfo},
    mixer_video::{VideoMixer, VideoMixerSpec},
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput},
    types::{CodecName, EvenUsize, PixelPosition},
    video::{FrameRate, VideoFormat, VideoFrame},
};
use orfail::OrFail;

const MIN_OUTPUT_WIDTH: usize = 16;
const MIN_OUTPUT_HEIGHT: usize = 16;

// テストでは 5 FPS に固定する
const FPS: FrameRate = FrameRate {
    numerator: NonZeroUsize::MIN.saturating_add(4),
    denumerator: NonZeroUsize::MIN,
};

// 5 FPS なので、映像フレーム一つの尺は 200 ms
const OUTPUT_FRAME_DURATION: Duration = Duration::from_millis(200);

const OUTPUT_STREAM_ID: MediaStreamId = MediaStreamId::new(100);

#[test]
fn start_noop_video_mixer() {
    let mut mixer = VideoMixer::new(
        layout(&[], &[], size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT), None),
        Vec::new(),
        OUTPUT_STREAM_ID,
    );

    // ミキサーへの入力が空なので、出力も空
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));
}

/// 一番単純な合成処理をテストする
#[test]
fn mix_single_source() {
    let (input_stream_id,) = (MediaStreamId::new(0),);
    let total_duration = ms(1000);

    // 入力をそのまま出力するようなリージョン
    let size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let mut region = region(size, size);
    let source = source(0, ms(0), total_duration); // 1000 ms 分のソース
    region.source_ids.insert(source.id.clone());
    region.grid.rows = 1;
    region.grid.columns = 1;
    region.grid.assign_source(source.id.clone(), 0, 0);

    let mut mixer = VideoMixer::new(
        layout(&[region], &[&source], size, None),
        vec![input_stream_id],
        OUTPUT_STREAM_ID,
    );

    // 入力映像フレームを送信する: 500 ms のフレームを二つ
    let input_frame0 = video_frame(&source, size, ms(0), ms(500), 2);
    let input_frame1 = video_frame(&source, size, ms(500), ms(500), 4);
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id,
            input_frame0.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id,
            input_frame1.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id))
        .unwrap();

    // 合成結果を取得する
    for i in 0..total_duration.as_millis() / OUTPUT_FRAME_DURATION.as_millis() {
        let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
        assert_eq!(frame.width, size.width);
        assert_eq!(frame.height, size.height);
        assert_eq!(frame.timestamp, OUTPUT_FRAME_DURATION * i as u32);
        assert_eq!(frame.duration, OUTPUT_FRAME_DURATION);

        if i < 3 {
            // ここまでは最初の入力フレームのデータが使われる
            assert_eq!(frame.data, input_frame0.data);
        } else {
            // ここからは次の入力フレームのデータが使われる
            assert_eq!(frame.data, input_frame1.data);
        }
    }

    // 全ての出力を取得した
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計情報を確認する
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_video_frame_count.get(), 2);
    assert_eq!(stats.total_output_video_frame_count.get(), 5);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(1000));
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 0);
}

/// リージョンの位置調整が入った合成のテスト
#[test]
fn mix_single_source_with_offset() {
    let input_stream_id = MediaStreamId::new(0);
    let total_duration = ms(1000);

    // 各種サイズ (region, cell となるにつれて、外側に 1 pixel ずつのマージンや枠線が入る）
    let output_size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let region_size = size(output_size.width - 2, output_size.height - 2);
    let cell_size = size(region_size.width - 2, region_size.height - 2);

    // リージョン設定
    let mut region = region(region_size, cell_size);
    let source = source(0, ms(0), total_duration); // 1000 ms 分のソース
    region.source_ids.insert(source.id.clone());
    region.position.x = EvenUsize::truncating_new(2); // リージョンの描画位置は端から 2 pixel 分ずらす
    region.position.y = EvenUsize::truncating_new(2);
    region.grid.rows = 1;
    region.grid.columns = 1;
    region.grid.assign_source(source.id.clone(), 0, 0);

    let mut mixer = VideoMixer::new(
        layout(&[region], &[&source], output_size, None),
        vec![input_stream_id],
        OUTPUT_STREAM_ID,
    );

    // 入力映像フレームを送信する: 500 ms のフレームを二つ
    // フレームのサイズは cell_size よりも大きいので合成時にリサイズされる
    let input_frame0 = video_frame(&source, output_size, ms(0), ms(500), 2);
    let input_frame1 = video_frame(&source, output_size, ms(500), ms(500), 4);
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id,
            input_frame0.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id,
            input_frame1.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id))
        .unwrap();

    // 合成結果を取得する
    for i in 0..total_duration.as_millis() / OUTPUT_FRAME_DURATION.as_millis() {
        let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
        assert_eq!(frame.width, output_size.width);
        assert_eq!(frame.height, output_size.height);
        assert_eq!(frame.timestamp, OUTPUT_FRAME_DURATION * i as u32);
        assert_eq!(frame.duration, OUTPUT_FRAME_DURATION);

        if i < 3 {
            // ここまでは最初の入力フレームのデータが使われる
            let expected = grayscale_image([
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ]);
            assert_eq!(frame.data, expected);
        } else {
            // ここからは次の入力フレームのデータが使われる
            let expected = grayscale_image([
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ]);
            assert_eq!(frame.data, expected);
        }
    }

    // 全ての出力を取得した
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計情報を確認する
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_video_frame_count.get(), 2);
    assert_eq!(stats.total_output_video_frame_count.get(), 5);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(1000));
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 0);
}

/// 一つのソースを複数のリージョンで使用するテスト
#[test]
fn single_source_multiple_regions() {
    let input_stream_id = MediaStreamId::new(0);
    let total_duration = ms(1000);

    // 各種サイズ
    let output_size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let region_size = size(12, 12);
    let cell_size = size(12, 10);

    // ソースは一つだけ
    let source = source(0, ms(0), total_duration); // 1000 ms 分のソース

    // ソースを共有する二つのリージョン設定
    let mut region0 = region(region_size, cell_size);
    region0.source_ids.insert(source.id.clone());
    region0.position.x = EvenUsize::truncating_new(2); // 一つ目のリージョンの描画位置は端から 2 pixel 分ずらす
    region0.position.y = EvenUsize::truncating_new(2);
    region0.top_border_pixels = EvenUsize::truncating_new(0); // こっちは上限の枠線はなし
    region0.grid.rows = 1;
    region0.grid.columns = 1;
    region0.grid.assign_source(source.id.clone(), 0, 0);

    let mut region1 = region(region_size, cell_size);
    region1.source_ids.insert(source.id.clone());
    region1.position.x = EvenUsize::truncating_new(4); // 二つ目のリージョンの描画位置は端から 4 pixel 分ずらす
    region1.position.y = EvenUsize::truncating_new(4);
    region1.top_border_pixels = EvenUsize::truncating_new(2);
    region1.grid.rows = 1;
    region1.grid.columns = 1;
    region1.grid.assign_source(source.id.clone(), 0, 0);

    let mut mixer = VideoMixer::new(
        layout(&[region0, region1], &[&source], output_size, None),
        vec![input_stream_id],
        OUTPUT_STREAM_ID,
    );

    // 入力映像フレームを送信する: 500 ms のフレームを二つ
    // リサイズを防ぐために cell_size を指定する
    let input_frame0 = video_frame(&source, cell_size, ms(0), ms(500), 2);
    let input_frame1 = video_frame(&source, cell_size, ms(500), ms(500), 4);
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id,
            input_frame0.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id,
            input_frame1.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id))
        .unwrap();

    // 合成結果を取得する
    for i in 0..total_duration.as_millis() / OUTPUT_FRAME_DURATION.as_millis() {
        let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
        assert_eq!(frame.width, output_size.width);
        assert_eq!(frame.height, output_size.height);
        assert_eq!(frame.timestamp, OUTPUT_FRAME_DURATION * i as u32);
        assert_eq!(frame.duration, OUTPUT_FRAME_DURATION);

        if i < 3 {
            // ここまでは最初の入力フレームのデータが使われる
            let expected = grayscale_image([
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0],
                [0, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
                [0, 0, 0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
            ]);
            assert_eq!(frame.data, expected);
        } else {
            // ここからは次の入力フレームのデータが使われる
            let expected = grayscale_image([
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0],
                [0, 0, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
                [0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
            ]);
            assert_eq!(frame.data, expected);
        }
    }

    // 全ての出力を取得した
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計情報を確認する
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_video_frame_count.get(), 2);
    assert_eq!(stats.total_output_video_frame_count.get(), 5);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(1000));
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 0);
}

/// 一つのソースを複数のリージョンで使用するテストのリサイズあり版
#[test]
fn single_source_multiple_regions_with_resize() {
    let input_stream_id = MediaStreamId::new(0);
    let total_duration = ms(1000);

    // 各種サイズ
    let output_size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let region_size = size(12, 12);

    // 複数リージョンでリサイズ結果が変わるようにセルサイズを変える
    let cell_size0 = size(12, 10);
    let cell_size1 = size(8, 8);

    // ソースは一つだけ
    let source = source(0, ms(0), total_duration); // 1000 ms 分のソース

    // ソースを共有する二つのリージョン設定
    let mut region0 = region(region_size, cell_size0);
    region0.source_ids.insert(source.id.clone());
    region0.position.x = EvenUsize::truncating_new(2); // 一つ目のリージョンの描画位置は端から 2 pixel 分ずらす
    region0.position.y = EvenUsize::truncating_new(2);
    region0.top_border_pixels = EvenUsize::truncating_new(0); // こっちは上限の枠線はなし
    region0.grid.rows = 1;
    region0.grid.columns = 1;
    region0.grid.assign_source(source.id.clone(), 0, 0);

    let mut region1 = region(region_size, cell_size1);
    region1.source_ids.insert(source.id.clone());
    region1.position.x = EvenUsize::truncating_new(4); // 二つ目のリージョンの描画位置は端から 4 pixel 分ずらす
    region1.position.y = EvenUsize::truncating_new(4);
    region1.top_border_pixels = EvenUsize::truncating_new(2);
    region1.grid.rows = 1;
    region1.grid.columns = 1;
    region1.grid.assign_source(source.id.clone(), 0, 0);

    let mut mixer = VideoMixer::new(
        layout(&[region0, region1], &[&source], output_size, None),
        vec![input_stream_id],
        OUTPUT_STREAM_ID,
    );

    // 入力映像フレームを送信する
    // サイズは cell_size0 に合わせているので region1 での合成の際にはリサイズが発生する
    let input_frame = video_frame(&source, cell_size0, ms(0), ms(1000), 2);
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id,
            input_frame,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id))
        .unwrap();

    // 比較用に最初の合成フレームを覚えておく
    let first_frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");

    // 残りの合成結果を取得する
    for i in 1..total_duration.as_millis() / OUTPUT_FRAME_DURATION.as_millis() {
        let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
        assert_eq!(frame.width, output_size.width);
        assert_eq!(frame.height, output_size.height);
        assert_eq!(frame.timestamp, OUTPUT_FRAME_DURATION * i as u32);
        assert_eq!(frame.duration, OUTPUT_FRAME_DURATION);

        // 共有ソースのリサイズによって想定外の影響で合成結果が変わっていないかを確認
        assert_eq!(frame.data, first_frame.data);
    }

    // 全ての出力を取得した
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計情報を確認する
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_video_frame_count.get(), 1);
    assert_eq!(stats.total_output_video_frame_count.get(), 5);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(1000));
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 0);
}

/// トリム期間（入力ソースが存在しなくて合成結果から除去される期間）がある場合のテスト
#[test]
fn mix_with_trim() -> orfail::Result<()> {
    let input_stream_id0 = MediaStreamId::new(0);
    let input_stream_id1 = MediaStreamId::new(1);

    // ソースは二つ用意する（途中に空白期間がある）
    let source0 = source(0, ms(0), ms(400)); // 0 ms ~ 400 ms
    let source1 = source(1, ms(800), ms(1000)); // 800 ms ~ 1000 ms
    let trim_span = (ms(400), ms(800));

    // 入力をそのまま出力するようなリージョン
    let size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let mut region = region(size, size);

    region.source_ids = [source0.id.clone(), source1.id.clone()]
        .into_iter()
        .collect();
    region.grid.rows = 1;
    region.grid.columns = 1;
    region.grid.assign_source(source0.id.clone(), 0, 0);
    region.grid.assign_source(source1.id.clone(), 0, 0);

    let mut mixer = VideoMixer::new(
        layout(&[region], &[&source0, &source1], size, Some(trim_span)),
        vec![input_stream_id0, input_stream_id1],
        OUTPUT_STREAM_ID,
    );

    // それぞれのソースで一つずつ入力映像フレームを送信する
    let input_frame0 = video_frame(&source0, size, ms(0), ms(400), 2);
    let input_frame1 = video_frame(&source1, size, ms(800), ms(200), 4);
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id0,
            input_frame0.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id1,
            input_frame1.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id0))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id1))
        .unwrap();

    // 合成結果を取得する
    let mut frames = Vec::new();
    while let MediaProcessorOutput::Processed { sample, .. } = mixer.process_output().or_fail()? {
        let frame = sample.expect_video_frame().or_fail()?;
        frames.push(frame);
    }

    // 最初のソースの期間
    for frame in frames.iter().take_while(|f| f.timestamp < ms(400)) {
        assert_eq!(frame.data, input_frame0.data);
    }

    // 残りは全部次のソースに対応する出力（トリム期間の結果は出力されないので）
    for frame in frames.iter().skip_while(|f| f.timestamp < ms(400)) {
        assert_eq!(frame.data, input_frame1.data);
    }

    // 統計情報を確認する
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_video_frame_count.get(), 2);
    assert_eq!(stats.total_output_video_frame_count.get(), 3);
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 2);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(600));

    Ok(())
}

/// `mix_with_trim()` とほぼ同様だけど、トリムは行わないテスト（空白期間は黒塗りになる）
#[test]
fn mix_without_trim() -> orfail::Result<()> {
    let input_stream_id0 = MediaStreamId::new(0);
    let input_stream_id1 = MediaStreamId::new(1);

    // ソースは二つ用意する（途中に空白期間がある）
    let source0 = source(0, ms(0), ms(400)); // 0 ms ~ 400 ms
    let source1 = source(1, ms(800), ms(1000)); // 800 ms ~ 1000 ms

    // 入力をそのまま出力するようなリージョン
    let size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let mut region = region(size, size);

    region.source_ids = [source0.id.clone(), source1.id.clone()]
        .into_iter()
        .collect();
    region.grid.rows = 1;
    region.grid.columns = 1;
    region.grid.assign_source(source0.id.clone(), 0, 0);
    region.grid.assign_source(source1.id.clone(), 0, 0);

    let mut mixer = VideoMixer::new(
        layout(&[region], &[&source0, &source1], size, None),
        vec![input_stream_id0, input_stream_id1],
        OUTPUT_STREAM_ID,
    );

    // それぞれのソースで一つずつ入力映像フレームを送信する
    let input_frame0 = video_frame(&source0, size, ms(0), ms(400), 2);
    let input_frame1 = video_frame(&source1, size, ms(800), ms(200), 4);
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id0,
            input_frame0.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id1,
            input_frame1.clone(),
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id0))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id1))
        .unwrap();

    // 合成結果を取得する
    let mut frames = Vec::new();
    while let MediaProcessorOutput::Processed { sample, .. } = mixer.process_output().or_fail()? {
        let frame = sample.expect_video_frame().or_fail()?;
        frames.push(frame);
    }

    // 最初のソースの期間
    for frame in frames.iter().take_while(|f| f.timestamp < ms(400)) {
        assert_eq!(frame.data, input_frame0.data);
    }

    // 次は入力ソースが存在しない空白期間
    let black = VideoFrame::black(
        EvenUsize::new(size.width).or_fail()?,
        EvenUsize::new(size.height).or_fail()?,
    );
    for frame in frames
        .iter()
        .filter(|f| ms(400) <= f.timestamp && f.timestamp < ms(800))
    {
        assert_eq!(frame.data, black.data);
    }

    // 残りは全部次のソースに対応する出力
    for frame in frames.iter().filter(|f| ms(800) <= f.timestamp) {
        assert_eq!(frame.data, input_frame1.data);
    }

    // 統計情報を確認する
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_video_frame_count.get(), 2);
    assert_eq!(stats.total_output_video_frame_count.get(), 5);
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 0);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(1000));

    Ok(())
}

/// 複数セルがある場合のテスト
///
/// [シナリオ]
/// 2x2 グリッドのセルがあって、その内の最初（左上）のセルには二つのセルが割り当てられている。
/// その左上のセルには、最初から最後までをカバーするソースがあるけど、途中でより優先度の高いソースが
/// 開始されて、一時的にそちらが優先される期間がある。
/// 残りのセルには、開始・終了期間が異なるソースが割り当てられている。
/// ただし、右下のセルは最初から最後まで未割り当てとする。
#[test]
fn mix_multiple_cells() -> orfail::Result<()> {
    let input_stream_id0 = MediaStreamId::new(0);
    let input_stream_id1 = MediaStreamId::new(1);
    let input_stream_id2 = MediaStreamId::new(2);
    let input_stream_id3 = MediaStreamId::new(3);

    // ソースを用意する
    let source0 = source(0, ms(0), ms(1000)); // 0 ms ~ 1000 ms (全期間)
    let source1 = source(1, ms(400), ms(800)); // 400 ms ~ 800 ms (source0 と同じセルに割り当てる）
    let source2 = source(2, ms(200), ms(1000)); // 200 ms ~ 1000 ms
    let source3 = source(3, ms(0), ms(600)); // 0 ms ~ 600 ms

    // セルが四つ(2x2)あるリージョンを用意する
    // セルの枠線は 2 pixel
    let region_size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let cell_size = size(6, 6);
    let mut region = region(region_size, cell_size);

    region.source_ids = [
        source0.id.clone(),
        source1.id.clone(),
        source2.id.clone(),
        source3.id.clone(),
    ]
    .into_iter()
    .collect();
    region.grid.rows = 2;
    region.grid.columns = 2;
    region.grid.assign_source(source0.id.clone(), 0, 1);
    region.grid.assign_source(source1.id.clone(), 0, 0); // 優先順位が source_cell0_0 よりも高い
    region.grid.assign_source(source2.id.clone(), 1, 0);
    region.grid.assign_source(source3.id.clone(), 2, 0);
    region.top_border_pixels = EvenUsize::truncating_new(2);
    region.left_border_pixels = EvenUsize::truncating_new(2);

    let mut spec = layout(
        &[region],
        &[&source0, &source1, &source2, &source3],
        region_size,
        None,
    );

    // リサイズによる入力画像の変化を最小限に抑えるために None を指定する
    spec.resize_filter_mode = shiguredo_libyuv::FilterMode::None;

    let mut mixer = VideoMixer::new(
        spec,
        vec![
            input_stream_id0,
            input_stream_id1,
            input_stream_id2,
            input_stream_id3,
        ],
        OUTPUT_STREAM_ID,
    );

    // それぞれのソースで一つずつ入力映像フレームを送信する
    let input_frame0 = video_frame(&source0, region_size, ms(0), ms(1000), 1);
    let input_frame1 = video_frame(&source1, region_size, ms(400), ms(400), 2);
    let input_frame2 = video_frame(&source2, region_size, ms(200), ms(800), 3);
    let input_frame3 = video_frame(&source3, region_size, ms(0), ms(600), 4);
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id0,
            input_frame0,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id1,
            input_frame1,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id2,
            input_frame2,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id3,
            input_frame3,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id0))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id1))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id2))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id3))
        .unwrap();

    // 合成結果を取得する
    // 0 ms ~ 200 ms の期間は source0 と source3 だけ
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 200 ms ~ 400 ms の期間は source0, source2, source3
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 400 ms ~ 600 msの期間は source1, source2, source3
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 600 ms ~ 800 msの期間は source1, source2
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 800 ms ~ 1000 msの期間は source0, source2
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 3, 3, 3, 3, 3, 3],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 全ての出力を取得した
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計情報を確認する
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_video_frame_count.get(), 4);
    assert_eq!(stats.total_output_video_frame_count.get(), 5);
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 0);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(1000));

    Ok(())
}

/// 枠線なしで複数セルがある場合のテスト
#[test]
fn mix_multiple_cells_with_no_borders() -> orfail::Result<()> {
    let input_stream_id0 = MediaStreamId::new(0);
    let input_stream_id1 = MediaStreamId::new(1);
    let input_stream_id2 = MediaStreamId::new(2);
    let input_stream_id3 = MediaStreamId::new(3);

    // ソースを用意する
    let source0 = source(0, ms(0), ms(1000)); // 0 ms ~ 1000 ms (全期間)
    let source1 = source(1, ms(400), ms(800)); // 400 ms ~ 800 ms (source0 と同じセルに割り当てる)
    let source2 = source(2, ms(200), ms(1000)); // 200 ms ~ 1000 ms
    let source3 = source(3, ms(0), ms(600)); // 0 ms ~ 600 ms

    // セルが四つ(2x2)あるリージョンを用意する
    // セルの枠線は 0 pixel
    let region_size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let cell_size = size(8, 8);
    let mut region = region(region_size, cell_size);

    region.source_ids = [
        source0.id.clone(),
        source1.id.clone(),
        source2.id.clone(),
        source3.id.clone(),
    ]
    .into_iter()
    .collect();
    region.grid.rows = 2;
    region.grid.columns = 2;
    region.grid.assign_source(source0.id.clone(), 0, 1);
    region.grid.assign_source(source1.id.clone(), 0, 0); // 優先順位が source_cell0_0 よりも高い
    region.grid.assign_source(source2.id.clone(), 1, 0);
    region.grid.assign_source(source3.id.clone(), 2, 0);
    region.top_border_pixels = EvenUsize::truncating_new(0); // 枠線なし
    region.left_border_pixels = EvenUsize::truncating_new(0); // 枠線なし
    region.inner_border_pixels = EvenUsize::truncating_new(0); // 枠線なし

    let mut mixer = VideoMixer::new(
        layout(
            &[region],
            &[&source0, &source1, &source2, &source3],
            region_size,
            None,
        ),
        vec![
            input_stream_id0,
            input_stream_id1,
            input_stream_id2,
            input_stream_id3,
        ],
        OUTPUT_STREAM_ID,
    );

    // それぞれのソースで一つずつ入力映像フレームを送信する
    let input_frame0 = video_frame(&source0, region_size, ms(0), ms(1000), 1);
    let input_frame1 = video_frame(&source1, region_size, ms(400), ms(400), 2);
    let input_frame2 = video_frame(&source2, region_size, ms(200), ms(800), 3);
    let input_frame3 = video_frame(&source3, region_size, ms(0), ms(600), 4);
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id0,
            input_frame0,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id1,
            input_frame1,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id2,
            input_frame2,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::video_frame(
            input_stream_id3,
            input_frame3,
        ))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id0))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id1))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id2))
        .unwrap();
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id3))
        .unwrap();

    // 合成結果を取得する
    // 0 ms ~ 200 ms の期間は source0 と source3 だけ
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 200 ms ~ 400 ms の期間は source0, source2, source3
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 400 ms ~ 600 msの期間は source1, source2, source3
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
        [4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 600 ms ~ 800 msの期間は source1, source2
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 800 ms ~ 1000 msの期間は source0, source2
    let frame = next_mixed_frame(&mut mixer).expect("failed to receive output frame");
    let expected = grayscale_image([
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(frame.data, expected);

    // 全ての出力を取得した
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // 統計情報を確認する
    let stats = mixer.stats();
    assert!(!stats.error.get());
    assert_eq!(stats.total_input_video_frame_count.get(), 4);
    assert_eq!(stats.total_output_video_frame_count.get(), 5);
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 0);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(1000));

    Ok(())
}

/// 不正なフォーマットの映像フレームを送るテスト
#[test]
fn non_yuv_video_input_error() -> orfail::Result<()> {
    let input_stream_id = MediaStreamId::new(0);
    let total_duration = ms(1000);

    // 入力をそのまま出力するようなリージョン
    let size = size(MIN_OUTPUT_WIDTH, MIN_OUTPUT_HEIGHT);
    let mut region = region(size, size);
    let source = source(0, ms(0), total_duration); // 1000 ms 分のソース
    region.source_ids.insert(source.id.clone());
    region.grid.rows = 1;
    region.grid.columns = 1;
    region.grid.assign_source(source.id.clone(), 0, 0);

    let mut mixer = VideoMixer::new(
        layout(&[region], &[&source], size, None),
        vec![input_stream_id],
        OUTPUT_STREAM_ID,
    );

    // 適当に不正なフォーマットを指定して VideoFrame を送る
    // 入力映像フレームを送信する: 500 ms のフレームをひとつ
    let mut input_frame = video_frame(&source, size, ms(0), ms(500), 2);
    input_frame.format = VideoFormat::Av1;
    assert!(
        mixer
            .process_input(MediaProcessorInput::video_frame(
                input_stream_id,
                input_frame,
            ))
            .is_err()
    );
    mixer
        .process_input(MediaProcessorInput::eos(input_stream_id))
        .unwrap();

    // エラーになるので、出力も存在しない
    assert!(matches!(
        mixer.process_output(),
        Ok(MediaProcessorOutput::Finished)
    ));

    // エラーは発生した
    let stats = mixer.stats();
    assert!(!stats.error.get()); // このフラグはスケジューラ側で管理しているので、ここでは `true` にならない

    // 統計値をチェックする
    assert_eq!(stats.total_input_video_frame_count.get(), 0);
    assert_eq!(stats.total_output_video_frame_count.get(), 0);
    assert_eq!(stats.total_output_video_frame_duration.get(), ms(0));
    assert_eq!(stats.total_trimmed_video_frame_count.get(), 0);

    Ok(())
}

fn layout(
    video_regions: &[Region],
    sources: &[&SourceInfo],
    size: Size,
    trim_span: Option<(Duration, Duration)>,
) -> VideoMixerSpec {
    let layout = Layout {
        trim_spans: TrimSpans::new(if let Some((start, end)) = trim_span {
            [(start, end)].into_iter().collect()
        } else {
            BTreeMap::new()
        }),
        video_regions: video_regions.to_vec(),
        sources: sources
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
        resolution: Resolution::new(size.width, size.height).expect("infallible"),

        // 以下のフィールドはテストで使われないので、適当な値を設定しておく
        base_path: PathBuf::from(""),
        audio_source_ids: Default::default(),
        frame_rate: FPS,
        audio_codec: CodecName::Opus,
        video_codec: CodecName::Vp8,
        audio_bitrate: None,
        video_bitrate: None,
        encode_params: Default::default(),
        decode_params: Default::default(),
    };
    VideoMixerSpec::from_layout(&layout)
}

fn region(region_size: Size, cell_size: Size) -> Region {
    Region {
        grid: Grid {
            assigned_sources: HashMap::new(),
            rows: 0,
            columns: 0,
            cell_width: EvenUsize::new(cell_size.width)
                .unwrap_or_else(|| panic!("not even: {cell_size:?}")),
            cell_height: EvenUsize::new(cell_size.height)
                .unwrap_or_else(|| panic!("not even: {cell_size:?}")),
        },
        source_ids: Default::default(),
        width: EvenUsize::new(region_size.width)
            .unwrap_or_else(|| panic!("not even: {region_size:?}")),
        height: EvenUsize::new(region_size.height)
            .unwrap_or_else(|| panic!("not even: {region_size:?}")),
        position: PixelPosition::default(),
        top_border_pixels: EvenUsize::default(),
        left_border_pixels: EvenUsize::default(),
        inner_border_pixels: EvenUsize::truncating_new(2),
        background_color: [0, 0, 0],

        // 以下のフィールドは VideoMixer では使われないので何でもいい
        // (Layout::video_regions の並びが z_pos を反映していることが前提）
        z_pos: 0,
    }
}

fn source(id: usize, start_timestamp: Duration, stop_timestamp: Duration) -> SourceInfo {
    SourceInfo {
        id: SourceId::new(&id.to_string()),
        start_timestamp,
        stop_timestamp,

        // 以下はダミー値
        audio: true,
        video: true,
        format: Default::default(),
    }
}

fn video_frame(
    source: &SourceInfo,
    size: Size,
    timestamp: Duration,
    duration: Duration,
    grayscale: u8,
) -> VideoFrame {
    let y_size = size.width * size.height;
    let uv_size = (size.width * size.height) / 4 * 2;
    VideoFrame {
        source_id: Some(source.id.clone()),
        data: std::iter::repeat_n(grayscale, y_size)
            .chain(std::iter::repeat_n(128, uv_size))
            .collect(),
        format: VideoFormat::I420,
        keyframe: true,
        width: size.width,
        height: size.height,
        timestamp,
        duration,
        sample_entry: None,
    }
}

fn ms(timestamp: u64) -> Duration {
    Duration::from_millis(timestamp)
}

fn size(width: usize, height: usize) -> Size {
    Size { width, height }
}

#[derive(Debug, Clone, Copy)]
struct Size {
    width: usize,
    height: usize,
}

// I420 形式に変換する
fn grayscale_image<const W: usize, const H: usize>(image: [[u8; W]; H]) -> Vec<u8> {
    let mut yuv = Vec::with_capacity(W * H * 3 / 2); // Y + U/4 + V/4 = 3/2
    yuv.extend_from_slice(&image.concat()); // Y
    yuv.extend(vec![128; (W * H) / 4 * 2]); // U と V
    yuv
}

fn next_mixed_frame(mixer: &mut VideoMixer) -> orfail::Result<Arc<VideoFrame>> {
    mixer
        .process_output()
        .or_fail()?
        .expect_processed()
        .or_fail()?
        .1
        .expect_video_frame()
        .or_fail()
}
