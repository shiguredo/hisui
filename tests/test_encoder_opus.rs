//! `src/encoder/opus.rs` の単体テスト。
//!
//! issue 0017 のバグ修正の核心である「音声エンコーダが最初の 1 フレームだけでなく
//! 全出力フレームに sample_entry を載せる」不変条件を検証する。
//! 「最初の 1 フレームだけ載せる」方式だと、録画 writer が最初の entry 付きフレームを
//! 取りこぼした場合に sample_entry が一度も届かず finalize に失敗する。

use std::num::NonZeroUsize;

use hisui::{
    audio::{AudioFormat, AudioFrame, Channels, SampleRate},
    encoder::opus::OpusEncoder,
};

// Opus エンコーダーへ渡す 20ms 分（48kHz ステレオ = 960 サンプル/ch）の無音入力を作る。
// data は I16Be のステレオインターリーブ（1 サンプル = 4 バイト: L 2 バイト + R 2 バイト）。
fn make_silent_input_frame() -> AudioFrame {
    let samples_per_channel = 960;
    AudioFrame {
        data: vec![0u8; samples_per_channel * 4],
        format: AudioFormat::I16Be,
        channels: Channels::STEREO,
        sample_rate: SampleRate::HZ_48000,
        timestamp: std::time::Duration::ZERO,
        sample_entry: None,
    }
}

#[test]
fn opus_encoder_sets_sample_entry_on_every_output_frame() -> hisui::Result<()> {
    let bitrate = NonZeroUsize::new(hisui::audio::DEFAULT_BITRATE).expect("bitrate is non-zero");
    let mut encoder = OpusEncoder::new(bitrate)?;

    // 最初のフレームには当然 sample_entry が載る。
    let first = encoder.encode(&make_silent_input_frame())?;
    let first_entry = first
        .sample_entry
        .as_ref()
        .expect("first frame must carry sample_entry");

    // 2 フレーム目以降にも sample_entry が載ることがバグ修正の核心。
    // 旧実装（self.sample_entry.take()）ではここが None になっていた。
    let second = encoder.encode(&make_silent_input_frame())?;
    let second_entry = second
        .sample_entry
        .as_ref()
        .expect("second frame must also carry sample_entry");

    // 同じエンコーダーが出す sample_entry は内容が同一であること。
    assert_eq!(first_entry.get(), second_entry.get());

    // さらに後続フレームでも継続して載ることを確認する。
    let third = encoder.encode(&make_silent_input_frame())?;
    assert!(
        third.sample_entry.is_some(),
        "third frame must also carry sample_entry"
    );

    Ok(())
}
