// nvcodec の 3 回連続再作成 (キーフレーム 3 個) が hisui scheduler なしで再現するかを確認する
// `cargo test --release --features nvcodec --test debug_nvcodec_standalone -- --nocapture` で実行
// デバッグ完了後は削除する

#[cfg(feature = "nvcodec")]
use hisui::{
    decoder_nvcodec::NvcodecDecoder, layout_decode_params::LayoutDecodeParams,
    metadata::SourceId, reader_mp4::Mp4VideoReader,
};
#[cfg(feature = "nvcodec")]
use orfail::OrFail;

// hisui scheduler / Processor を経由せず NvcodecDecoder::decode を直接ループで呼ぶ
// - 3 回目のキーフレームまで進んで hang or Err なら nvcodec 内の問題確定
// - 3 回目まで完走するなら hisui scheduler 側で 3 回目のフレームが渡っていない
#[cfg(feature = "nvcodec")]
fn run(codec: &str, path: &str, source_id: &str) -> orfail::Result<()> {
    if !shiguredo_nvcodec::is_cuda_library_available() {
        eprintln!("skip: CUDA ライブラリが利用できない");
        return Ok(());
    }

    let reader = Mp4VideoReader::new(SourceId::new(source_id), path, Default::default())
        .or_fail()?;
    let params = LayoutDecodeParams::default();
    let mut decoder = match codec {
        "h264" => NvcodecDecoder::new_h264(&params).or_fail()?,
        "h265" => NvcodecDecoder::new_h265(&params).or_fail()?,
        _ => unreachable!(),
    };
    eprintln!("[standalone-{codec}] NvcodecDecoder 初期化成功");

    for (i, frame) in reader.enumerate() {
        let frame = frame.or_fail()?;
        eprintln!(
            "[standalone-{codec}] frame {i}: decode() BEFORE keyframe={} data.len()={}",
            frame.keyframe,
            frame.data.len()
        );
        let t = std::time::Instant::now();
        decoder.decode(&frame).or_fail()?;
        eprintln!(
            "[standalone-{codec}] frame {i}: decode() {}ms",
            t.elapsed().as_millis()
        );
        while let Some(_dec) = decoder.next_decoded_frame() {
            // 出力は捨てる (今回の目的は再作成できるかの確認のみ)
        }
    }
    eprintln!("[standalone-{codec}] finish() BEFORE");
    let t = std::time::Instant::now();
    decoder.finish().or_fail()?;
    eprintln!(
        "[standalone-{codec}] finish() {}ms",
        t.elapsed().as_millis()
    );
    while let Some(_dec) = decoder.next_decoded_frame() {}
    eprintln!("[standalone-{codec}] 全フレーム完了");
    Ok(())
}

#[cfg(feature = "nvcodec")]
#[test]
fn standalone_h264_resolution_change() -> orfail::Result<()> {
    run(
        "h264",
        "testdata/archive-h264-resolution-change.mp4",
        "archive-h264-resolution-change",
    )
}

#[cfg(feature = "nvcodec")]
#[test]
fn standalone_h265_resolution_change() -> orfail::Result<()> {
    run(
        "h265",
        "testdata/archive-h265-resolution-change.mp4",
        "archive-h265-resolution-change",
    )
}
