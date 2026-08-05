// H.265 nvcodec デバッグ用テスト
// `cargo test --features nvcodec --test debug_h265_dump -- --nocapture` で実行
// デバッグ完了後は削除する

#[cfg(feature = "nvcodec")]
use hisui::{
    decoder::{VideoDecoder, VideoDecoderOptions},
    media::MediaStreamId,
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput},
    types::EngineName,
};
use hisui::{metadata::SourceId, reader_mp4::Mp4VideoReader};
use orfail::OrFail;

#[cfg(feature = "nvcodec")]
const DECODER_INPUT_STREAM_ID: MediaStreamId = MediaStreamId::new(0);
#[cfg(feature = "nvcodec")]
const DECODER_OUTPUT_STREAM_ID: MediaStreamId = MediaStreamId::new(1);

fn nal_unit_types_h265(data: &[u8]) -> Vec<(usize, usize, u8)> {
    let mut result = Vec::new();
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        if pos + 4 + len > data.len() {
            break;
        }
        if len > 0 {
            let nal_type = (data[pos + 4] >> 1) & 0x3F;
            result.push((pos, len, nal_type));
        }
        pos += 4 + len;
    }
    result
}

#[test]
fn dump_h265_hvc1_nal_layout() -> orfail::Result<()> {
    let source_id = SourceId::new("archive-h265-resolution-change");
    let reader = Mp4VideoReader::new(
        source_id,
        "testdata/archive-h265-resolution-change.mp4",
        Default::default(),
    )
    .or_fail()?;

    for (i, frame) in reader.enumerate().take(2) {
        let frame = frame.or_fail()?;
        eprintln!(
            "hvc1 frame {i}: key={}, len={}, sample_entry={}",
            frame.keyframe,
            frame.data.len(),
            if frame.sample_entry.is_some() {
                "Some"
            } else {
                "None"
            }
        );
        for (offset, len, ty) in nal_unit_types_h265(&frame.data) {
            eprintln!("  offset={offset} len={len} nal_type={ty}");
        }
    }
    Ok(())
}

#[test]
fn dump_h265_hev1_nal_layout() -> orfail::Result<()> {
    // 既存の hev1 形式 (単一解像度) testdata で構造を確認する比較用
    let source_id = SourceId::new("archive-blue-640x480-h265");
    let reader = Mp4VideoReader::new(
        source_id,
        "testdata/archive-blue-640x480-h265.mp4",
        Default::default(),
    )
    .or_fail()?;

    for (i, frame) in reader.enumerate().take(2) {
        let frame = frame.or_fail()?;
        eprintln!(
            "hev1 frame {i}: key={}, len={}, sample_entry={}",
            frame.keyframe,
            frame.data.len(),
            if frame.sample_entry.is_some() {
                "Some"
            } else {
                "None"
            }
        );
        for (offset, len, ty) in nal_unit_types_h265(&frame.data) {
            eprintln!("  offset={offset} len={len} nal_type={ty}");
        }
    }
    Ok(())
}

// 既存の hev1 形式 (単一解像度) を nvcodec でデコードできるかの smoke test
// これが成功すれば、hvc1→hev1 変換由来か、解像度変化データ由来かが切り分けられる
#[test]
#[cfg(feature = "nvcodec")]
fn smoke_h265_hev1_single_resolution_nvcodec() -> orfail::Result<()> {
    if !shiguredo_nvcodec::is_cuda_library_available() {
        eprintln!("skip: CUDA ライブラリが利用できない");
        return Ok(());
    }
    let source_id = SourceId::new("archive-blue-640x480-h265");
    let reader = Mp4VideoReader::new(
        source_id,
        "testdata/archive-blue-640x480-h265.mp4",
        Default::default(),
    )
    .or_fail()?;
    let options = VideoDecoderOptions {
        openh264_lib: None,
        decode_params: Default::default(),
        engines: Some(vec![EngineName::Nvcodec]),
    };
    let mut decoder = VideoDecoder::new(DECODER_INPUT_STREAM_ID, DECODER_OUTPUT_STREAM_ID, options);
    let mut in_count = 0;
    for f in reader {
        decoder
            .process_input(MediaProcessorInput::video_frame(
                DECODER_INPUT_STREAM_ID,
                f.or_fail()?,
            ))
            .or_fail()?;
        in_count += 1;
    }
    decoder
        .process_input(MediaProcessorInput::eos(DECODER_INPUT_STREAM_ID))
        .or_fail()?;
    let mut out_count = 0;
    while let MediaProcessorOutput::Processed { .. } = decoder.process_output().or_fail()? {
        out_count += 1;
    }
    eprintln!("[hev1-single] input={in_count}, output={out_count}");
    Ok(())
}

// hvc1 → hev1 変換した解像度変化データを nvcodec でデコードする (=失敗が既知)
// エラーメッセージがどこで出るか、初回フレームで出るのか複数フレーム目で出るのかを確認
#[test]
#[cfg(feature = "nvcodec")]
fn smoke_h265_hvc1_resolution_change_nvcodec() -> orfail::Result<()> {
    if !shiguredo_nvcodec::is_cuda_library_available() {
        eprintln!("skip: CUDA ライブラリが利用できない");
        return Ok(());
    }
    let source_id = SourceId::new("archive-h265-resolution-change");
    let reader = Mp4VideoReader::new(
        source_id,
        "testdata/archive-h265-resolution-change.mp4",
        Default::default(),
    )
    .or_fail()?;
    let options = VideoDecoderOptions {
        openh264_lib: None,
        decode_params: Default::default(),
        engines: Some(vec![EngineName::Nvcodec]),
    };
    let mut decoder = VideoDecoder::new(DECODER_INPUT_STREAM_ID, DECODER_OUTPUT_STREAM_ID, options);
    for (i, f) in reader.enumerate() {
        let result = decoder.process_input(MediaProcessorInput::video_frame(
            DECODER_INPUT_STREAM_ID,
            f.or_fail()?,
        ));
        if let Err(e) = result {
            eprintln!("[hvc1-res-change] frame {i} で失敗: {e}");
            return Ok(());
        }
    }
    let _ = decoder.process_input(MediaProcessorInput::eos(DECODER_INPUT_STREAM_ID));
    eprintln!("[hvc1-res-change] 全フレームの process_input に成功");
    Ok(())
}
