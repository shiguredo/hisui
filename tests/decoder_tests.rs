use std::sync::Arc;

use hisui::{
    decoder::{VideoDecoder, VideoDecoderOptions},
    media::MediaStreamId,
    metadata::SourceId,
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput},
    reader_mp4::Mp4VideoReader,
    types::EngineName,
    video::VideoFrame,
};
use orfail::OrFail;
use shiguredo_mp4::boxes::{Avc1Box, AvccBox, SampleEntry};
use shiguredo_openh264::Openh264Library;

const DECODER_INPUT_STREAM_ID: MediaStreamId = MediaStreamId::new(0);
const DECODER_OUTPUT_STREAM_ID: MediaStreamId = MediaStreamId::new(1);

#[test]
fn h264_multi_resolutions() -> orfail::Result<()> {
    let source_id0 = SourceId::new("archive-blue-640x480-h264");
    let source_id1 = SourceId::new("archive-blue-640x480-h264");
    let reader0 = Mp4VideoReader::new(
        source_id0,
        "testdata/archive-blue-640x480-h264.mp4",
        Default::default(),
    )
    .or_fail()?;
    let reader1 = Mp4VideoReader::new(
        source_id1,
        "testdata/archive-red-320x320-h264.mp4",
        Default::default(),
    )
    .or_fail()?;
    multi_resolutions_test(reader0, reader1).or_fail()?;
    Ok(())
}

#[test]
#[cfg(target_os = "macos")]
fn h265_multi_resolutions() -> orfail::Result<()> {
    let source_id0 = SourceId::new("archive-blue-640x480-h265");
    let source_id1 = SourceId::new("archive-red-320x320-h265");
    let reader0 = Mp4VideoReader::new(
        source_id0,
        "testdata/archive-blue-640x480-h265.mp4",
        Default::default(),
    )
    .or_fail()?;
    let reader1 = Mp4VideoReader::new(
        source_id1,
        "testdata/archive-red-320x320-h265.mp4",
        Default::default(),
    )
    .or_fail()?;
    multi_resolutions_test(reader0, reader1).or_fail()?;
    Ok(())
}

#[test]
#[cfg(feature = "libvpx")]
fn vp9_multi_resolutions() -> orfail::Result<()> {
    let source_id0 = SourceId::new("archive-blue-640x480-vp9");
    let source_id1 = SourceId::new("archive-red-320x320-vp9");
    let reader0 = Mp4VideoReader::new(
        source_id0,
        "testdata/archive-blue-640x480-vp9.mp4",
        Default::default(),
    )
    .or_fail()?;
    let reader1 = Mp4VideoReader::new(
        source_id1,
        "testdata/archive-red-320x320-vp9.mp4",
        Default::default(),
    )
    .or_fail()?;
    multi_resolutions_test(reader0, reader1).or_fail()?;
    Ok(())
}

#[test]
fn av1_multi_resolutions() -> orfail::Result<()> {
    let source_id0 = SourceId::new("archive-blue-640x480-av1");
    let source_id1 = SourceId::new("archive-red-320x320-av1");
    let reader0 = Mp4VideoReader::new(
        source_id0,
        "testdata/archive-blue-640x480-av1.mp4",
        Default::default(),
    )
    .or_fail()?;
    let reader1 = Mp4VideoReader::new(
        source_id1,
        "testdata/archive-red-320x320-av1.mp4",
        Default::default(),
    )
    .or_fail()?;
    multi_resolutions_test(reader0, reader1).or_fail()?;
    Ok(())
}

fn multi_resolutions_test<I>(reader0: I, reader1: I) -> orfail::Result<()>
where
    I: Iterator<Item = orfail::Result<VideoFrame>>,
{
    let options = VideoDecoderOptions {
        openh264_lib: if let Ok(path) = std::env::var("OPENH264_PATH") {
            Some(Openh264Library::load(path).or_fail()?)
        } else if cfg!(target_os = "macos") {
            None
        } else {
            // 利用可能な H.264 デコーダーは存在しない
            eprintln!("no available H.264 decoder");
            return Ok(());
        },
        decode_params: Default::default(),
        engines: None,
    };

    // デコードする
    let mut decoder = VideoDecoder::new(DECODER_INPUT_STREAM_ID, DECODER_OUTPUT_STREAM_ID, options);
    let mut output_frames = Vec::new();
    let mut blue_count = 0;
    let mut red_count = 0;

    for input_frame in reader0 {
        let input = prepend_h264_sps_pps(input_frame.or_fail()?);
        decoder.process_input(input).or_fail()?;
        blue_count += 1;
    }

    // このタイミングで解像度などが切り替わる
    for input_frame in reader1 {
        let input = prepend_h264_sps_pps(input_frame.or_fail()?);
        decoder.process_input(input).or_fail()?;
        red_count += 1;
    }

    decoder
        .process_input(MediaProcessorInput::eos(DECODER_INPUT_STREAM_ID))
        .or_fail()?;
    while let MediaProcessorOutput::Processed { sample, .. } = decoder.process_output().or_fail()? {
        let output_frame = sample.expect_video_frame().or_fail()?;
        output_frames.push(output_frame);
    }

    // デコード結果を確認する
    for output_frame in output_frames {
        if blue_count > 0 {
            blue_count -= 1;
            assert_eq!(output_frame.width, 640);
            assert_eq!(output_frame.height, 480);

            // 単色青色かどうかのチェック
            let (y_plane, u_plane, v_plane) = output_frame.as_yuv_planes().or_fail()?;
            y_plane.iter().for_each(|&y| assert_eq!(y, 41));
            u_plane.iter().for_each(|&y| assert_eq!(y, 240));
            v_plane.iter().for_each(|&y| assert_eq!(y, 110));
        } else {
            red_count -= 1;
            assert_eq!(output_frame.width, 320);
            assert_eq!(output_frame.height, 320);

            // 単色赤色かどうかのチェック
            let (y_plane, u_plane, v_plane) = output_frame.as_yuv_planes().or_fail()?;
            y_plane.iter().for_each(|&y| assert_eq!(y, 81));
            u_plane.iter().for_each(|&u| assert_eq!(u, 90));
            v_plane.iter().for_each(|&v| assert_eq!(v, 240));
        }
    }
    assert_eq!(blue_count, 0);
    assert_eq!(red_count, 0);

    Ok(())
}

// H.264 1 トラック内でキーフレーム毎に解像度が変わる MP4 を、engine を明示指定してデコードする
//
// 期待する解像度シーケンス (15 fps × 3 秒 = 45 フレーム、キーフレームは frame 0 / 15 / 30):
// - フレーム 0..15  → 320x240
// - フレーム 15..30 → 224x160
// - フレーム 30..45 → 320x240
fn h264_single_track_resolution_change_test(
    engines: Option<Vec<EngineName>>,
    openh264_lib: Option<Openh264Library>,
) -> orfail::Result<()> {
    let source_id = SourceId::new("archive-h264-resolution-change");
    let reader = Mp4VideoReader::new(
        source_id,
        "testdata/archive-h264-resolution-change.mp4",
        Default::default(),
    )
    .or_fail()?;

    let options = VideoDecoderOptions {
        openh264_lib,
        decode_params: Default::default(),
        engines,
    };

    let mut decoder = VideoDecoder::new(DECODER_INPUT_STREAM_ID, DECODER_OUTPUT_STREAM_ID, options);

    let mut input_count = 0;
    for input_frame in reader {
        let input = prepend_h264_sps_pps(input_frame.or_fail()?);
        decoder.process_input(input).or_fail()?;
        input_count += 1;
    }
    decoder
        .process_input(MediaProcessorInput::eos(DECODER_INPUT_STREAM_ID))
        .or_fail()?;

    let mut output_frames = Vec::new();
    while let MediaProcessorOutput::Processed { sample, .. } = decoder.process_output().or_fail()? {
        let output_frame = sample.expect_video_frame().or_fail()?;
        output_frames.push(output_frame);
    }

    assert_expected_resolution_sequence(input_count, &output_frames);
    Ok(())
}

// H.264 以外のコーデック用: 1 トラック内でキーフレーム毎に解像度が変わる MP4 を、engine を
// 明示指定してデコードする (frame data はそのまま流す、SPS/PPS の prepend は不要)
fn passthrough_single_track_resolution_change_test(
    testdata_path: &str,
    source_id_str: &str,
    engines: Option<Vec<EngineName>>,
) -> orfail::Result<()> {
    let source_id = SourceId::new(source_id_str);
    let reader = Mp4VideoReader::new(source_id, testdata_path, Default::default()).or_fail()?;

    let options = VideoDecoderOptions {
        openh264_lib: None,
        decode_params: Default::default(),
        engines,
    };

    let mut decoder = VideoDecoder::new(DECODER_INPUT_STREAM_ID, DECODER_OUTPUT_STREAM_ID, options);

    let mut input_count = 0;
    for input_frame in reader {
        let frame = input_frame.or_fail()?;
        decoder
            .process_input(MediaProcessorInput::video_frame(
                DECODER_INPUT_STREAM_ID,
                frame,
            ))
            .or_fail()?;
        input_count += 1;
    }
    decoder
        .process_input(MediaProcessorInput::eos(DECODER_INPUT_STREAM_ID))
        .or_fail()?;

    let mut output_frames = Vec::new();
    while let MediaProcessorOutput::Processed { sample, .. } = decoder.process_output().or_fail()? {
        let output_frame = sample.expect_video_frame().or_fail()?;
        output_frames.push(output_frame);
    }

    assert_expected_resolution_sequence(input_count, &output_frames);
    Ok(())
}

fn assert_expected_resolution_sequence(input_count: usize, output_frames: &[Arc<VideoFrame>]) {
    assert_eq!(input_count, 45, "入力フレーム数が想定と異なる");
    assert_eq!(output_frames.len(), 45, "出力フレーム数が想定と異なる");

    // NVDEC のハードウェアデコード最小解像度 (HEVC=144x144 / VP9=128x128 / AV1=128x128) を
    // 全コーデックで上回るように解像度を選んでいる
    let expected: Vec<(usize, usize)> = (0..15)
        .map(|_| (320, 240))
        .chain((0..15).map(|_| (224, 160)))
        .chain((0..15).map(|_| (320, 240)))
        .collect();

    for (i, (frame, (expected_width, expected_height))) in
        output_frames.iter().zip(expected.iter()).enumerate()
    {
        assert_eq!(
            (frame.width, frame.height),
            (*expected_width, *expected_height),
            "フレーム {i} の解像度が期待値と一致しない"
        );
    }
}

#[test]
fn h264_single_track_resolution_change_openh264() -> orfail::Result<()> {
    let Ok(path) = std::env::var("OPENH264_PATH") else {
        eprintln!("skip: OPENH264_PATH が設定されていない");
        return Ok(());
    };
    let openh264_lib = Openh264Library::load(path).or_fail()?;
    h264_single_track_resolution_change_test(
        Some(vec![EngineName::Openh264]),
        Some(openh264_lib),
    )
    .or_fail()
}

#[test]
#[cfg(target_os = "macos")]
fn h264_single_track_resolution_change_video_toolbox() -> orfail::Result<()> {
    h264_single_track_resolution_change_test(Some(vec![EngineName::VideoToolbox]), None).or_fail()
}

#[test]
#[cfg(feature = "nvcodec")]
fn h264_single_track_resolution_change_nvcodec() -> orfail::Result<()> {
    if !shiguredo_nvcodec::is_cuda_library_available() {
        eprintln!("skip: CUDA ライブラリが利用できない");
        return Ok(());
    }
    h264_single_track_resolution_change_test(Some(vec![EngineName::Nvcodec]), None).or_fail()
}

#[test]
#[cfg(target_os = "macos")]
fn h265_single_track_resolution_change_video_toolbox() -> orfail::Result<()> {
    passthrough_single_track_resolution_change_test(
        "testdata/archive-h265-resolution-change.mp4",
        "archive-h265-resolution-change",
        Some(vec![EngineName::VideoToolbox]),
    )
    .or_fail()
}

#[test]
#[cfg(feature = "nvcodec")]
fn h265_single_track_resolution_change_nvcodec() -> orfail::Result<()> {
    if !shiguredo_nvcodec::is_cuda_library_available() {
        eprintln!("skip: CUDA ライブラリが利用できない");
        return Ok(());
    }
    passthrough_single_track_resolution_change_test(
        "testdata/archive-h265-resolution-change.mp4",
        "archive-h265-resolution-change",
        Some(vec![EngineName::Nvcodec]),
    )
    .or_fail()
}

#[test]
#[cfg(feature = "libvpx")]
fn vp8_single_track_resolution_change_libvpx() -> orfail::Result<()> {
    passthrough_single_track_resolution_change_test(
        "testdata/archive-vp8-resolution-change.mp4",
        "archive-vp8-resolution-change",
        Some(vec![EngineName::Libvpx]),
    )
    .or_fail()
}

#[test]
#[cfg(feature = "nvcodec")]
fn vp8_single_track_resolution_change_nvcodec() -> orfail::Result<()> {
    if !shiguredo_nvcodec::is_cuda_library_available() {
        eprintln!("skip: CUDA ライブラリが利用できない");
        return Ok(());
    }
    passthrough_single_track_resolution_change_test(
        "testdata/archive-vp8-resolution-change.mp4",
        "archive-vp8-resolution-change",
        Some(vec![EngineName::Nvcodec]),
    )
    .or_fail()
}

#[test]
#[cfg(feature = "libvpx")]
fn vp9_single_track_resolution_change_libvpx() -> orfail::Result<()> {
    passthrough_single_track_resolution_change_test(
        "testdata/archive-vp9-resolution-change.mp4",
        "archive-vp9-resolution-change",
        Some(vec![EngineName::Libvpx]),
    )
    .or_fail()
}

#[test]
#[cfg(feature = "nvcodec")]
fn vp9_single_track_resolution_change_nvcodec() -> orfail::Result<()> {
    if !shiguredo_nvcodec::is_cuda_library_available() {
        eprintln!("skip: CUDA ライブラリが利用できない");
        return Ok(());
    }
    passthrough_single_track_resolution_change_test(
        "testdata/archive-vp9-resolution-change.mp4",
        "archive-vp9-resolution-change",
        Some(vec![EngineName::Nvcodec]),
    )
    .or_fail()
}

#[test]
fn av1_single_track_resolution_change_dav1d() -> orfail::Result<()> {
    passthrough_single_track_resolution_change_test(
        "testdata/archive-av1-resolution-change.mp4",
        "archive-av1-resolution-change",
        Some(vec![EngineName::Dav1d]),
    )
    .or_fail()
}

#[test]
#[cfg(feature = "nvcodec")]
fn av1_single_track_resolution_change_nvcodec() -> orfail::Result<()> {
    if !shiguredo_nvcodec::is_cuda_library_available() {
        eprintln!("skip: CUDA ライブラリが利用できない");
        return Ok(());
    }
    passthrough_single_track_resolution_change_test(
        "testdata/archive-av1-resolution-change.mp4",
        "archive-av1-resolution-change",
        Some(vec![EngineName::Nvcodec]),
    )
    .or_fail()
}

fn prepend_h264_sps_pps(mut frame: VideoFrame) -> MediaProcessorInput {
    if let Some(SampleEntry::Avc1(Avc1Box {
        avcc_box: AvccBox {
            sps_list, pps_list, ..
        },
        ..
    })) = frame.sample_entry.clone()
    {
        // openh264 用に映像データ本体にも SPS / PPS を含める
        let mut data = Vec::new();
        for nalu in sps_list.into_iter().chain(pps_list.into_iter()) {
            data.extend_from_slice(&(nalu.len() as u32).to_be_bytes());
            data.extend_from_slice(&nalu);
        }
        data.extend_from_slice(&frame.data);
        frame.data = data;
    };

    // 対象外のフレームはそのまま返す
    MediaProcessorInput::video_frame(DECODER_INPUT_STREAM_ID, frame)
}
