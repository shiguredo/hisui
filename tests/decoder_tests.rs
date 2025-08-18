use hisui::{
    decoder::{VideoDecoder, VideoDecoderOptions},
    media::MediaStreamId,
    metadata::SourceId,
    reader_mp4::Mp4VideoReader,
    stats::VideoDecoderStats,
    video::VideoFrame,
};
use orfail::OrFail;
use shiguredo_mp4::boxes::{Avc1Box, AvccBox, SampleEntry};
use shiguredo_openh264::Openh264Library;

// 実質的には使われないので値はなんでもいい
const DUMMY_STREAM_ID: MediaStreamId = MediaStreamId::new(0);

#[test]
fn h264_multi_resolutions() -> orfail::Result<()> {
    let source_id0 = SourceId::new("archive-blue-640x480-h264");
    let source_id1 = SourceId::new("archive-blue-640x480-h264");
    let reader0 = Mp4VideoReader::new(
        source_id0,
        DUMMY_STREAM_ID,
        "testdata/archive-blue-640x480-h264.mp4",
    )
    .or_fail()?;
    let reader1 = Mp4VideoReader::new(
        source_id1,
        DUMMY_STREAM_ID,
        "tstdata/archive-red-320x320-h264.mp4",
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
        DUMMY_STREAM_ID,
        "tstdata/archive-blue-640x480-h265.mp4",
    )
    .or_fail()?;
    let reader1 = Mp4VideoReader::new(
        source_id1,
        DUMMY_STREAM_ID,
        "tstdata/archive-red-320x320-h265.mp4",
    )
    .or_fail()?;
    multi_resolutions_test(reader0, reader1).or_fail()?;
    Ok(())
}

#[test]
fn vp9_multi_resolutions() -> orfail::Result<()> {
    let source_id0 = SourceId::new("archive-blue-640x480-vp9");
    let source_id1 = SourceId::new("archive-red-320x320-vp9");
    let reader0 = Mp4VideoReader::new(
        source_id0,
        DUMMY_STREAM_ID,
        "tstdata/archive-blue-640x480-vp9.mp4",
    )
    .or_fail()?;
    let reader1 = Mp4VideoReader::new(
        source_id1,
        DUMMY_STREAM_ID,
        "tstdata/archive-red-320x320-vp9.mp4",
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
        DUMMY_STREAM_ID,
        "tstdata/archive-blue-640x480-av1.mp4",
    )
    .or_fail()?;
    let reader1 = Mp4VideoReader::new(
        source_id1,
        DUMMY_STREAM_ID,
        "tstdata/archive-red-320x320-av1.mp4",
    )
    .or_fail()?;
    multi_resolutions_test(reader0, reader1).or_fail()?;
    Ok(())
}

fn multi_resolutions_test<I>(reader0: I, reader1: I) -> orfail::Result<()>
where
    I: Iterator<Item = orfail::Result<VideoFrame>>,
{
    let mut stats = VideoDecoderStats::default();
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
    };

    // デコードする
    let mut decoder = VideoDecoder::new(options);
    let mut output_frames = Vec::new();
    let mut blue_count = 0;
    let mut red_count = 0;

    for input_frame in reader0 {
        let input_frame = prepend_h264_sps_pps(input_frame.or_fail()?);
        decoder.decode(input_frame, &mut stats).or_fail()?;
        blue_count += 1;
        while let Some(output_frame) = decoder.next_decoded_frame() {
            output_frames.push(output_frame);
        }
    }

    // このタイミングで解像度などが切り替わる
    for input_frame in reader1 {
        let input_frame = prepend_h264_sps_pps(input_frame.or_fail()?);
        decoder.decode(input_frame, &mut stats).or_fail()?;
        red_count += 1;
        while let Some(output_frame) = decoder.next_decoded_frame() {
            output_frames.push(output_frame);
        }
    }

    decoder.finish().or_fail()?;
    while let Some(output_frame) = decoder.next_decoded_frame() {
        output_frames.push(output_frame);
    }

    // デコード結果を確認する
    for output_frame in output_frames {
        if blue_count > 0 {
            blue_count -= 1;
            assert_eq!(output_frame.width.get(), 640);
            assert_eq!(output_frame.height.get(), 480);

            // 単色青色かどうかのチェック
            let (y_plane, u_plane, v_plane) = output_frame.as_yuv_planes().or_fail()?;
            y_plane.iter().for_each(|&y| assert_eq!(y, 41));
            u_plane.iter().for_each(|&y| assert_eq!(y, 240));
            v_plane.iter().for_each(|&y| assert_eq!(y, 110));
        } else {
            red_count -= 1;
            assert_eq!(output_frame.width.get(), 320);
            assert_eq!(output_frame.height.get(), 320);

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

fn prepend_h264_sps_pps(mut frame: VideoFrame) -> VideoFrame {
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
    frame
}
