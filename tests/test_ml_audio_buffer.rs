//! `src/ml/audio/buffer.rs` の integration テスト。

#![cfg(feature = "candle")]

use std::num::NonZeroUsize;

use hisui::ml::audio::buffer::AudioChunkBuffer;

/// テスト内で `NonZeroUsize` を組み立てる補助関数 (0 を渡すとテスト失敗)。
fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("chunk_samples must be > 0")
}

/// 空 buffer から `take_chunk` すると None を返す。
#[test]
fn take_chunk_returns_none_when_empty() {
    let mut buf = AudioChunkBuffer::new(nz(4));
    assert!(buf.take_chunk().is_none());
    assert_eq!(buf.remaining(), 0);
}

/// `chunk_samples` 未満の蓄積では `take_chunk` は None を返し、`remaining` に蓄積量が反映される。
#[test]
fn take_chunk_returns_none_when_under_threshold() {
    let mut buf = AudioChunkBuffer::new(nz(4));
    buf.push(&[1.0, 2.0]);
    assert!(buf.take_chunk().is_none(), "4 サンプル未満では取れない想定");
    assert_eq!(buf.remaining(), 2);
}

/// ちょうど `chunk_samples` を push すると 1 チャンクだけ取れ、残余は 0。
#[test]
fn take_chunk_returns_exact_chunk_when_boundary() {
    let mut buf = AudioChunkBuffer::new(nz(4));
    buf.push(&[1.0, 2.0, 3.0, 4.0]);
    let chunk = buf
        .take_chunk()
        .expect("4 サンプルで 1 チャンクが取れる想定");
    assert_eq!(chunk, vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(buf.remaining(), 0);
    assert!(buf.take_chunk().is_none(), "取り出した後は再度 None");
}

/// 複数チャンクぶんを一度に push すると、順に `take_chunk` で取り出せる。
#[test]
fn take_chunk_yields_multiple_chunks_in_order() {
    let mut buf = AudioChunkBuffer::new(nz(3));
    buf.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
    assert_eq!(buf.take_chunk(), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(buf.take_chunk(), Some(vec![4.0, 5.0, 6.0]));
    assert_eq!(
        buf.take_chunk(),
        None,
        "残り 1 サンプルはチャンクにならない"
    );
    assert_eq!(buf.remaining(), 1);
}

/// push → take_chunk → push で蓄積量が加算されることを確認する。
#[test]
fn push_take_push_accumulates_correctly() {
    let mut buf = AudioChunkBuffer::new(nz(4));
    buf.push(&[1.0, 2.0, 3.0]);
    assert!(buf.take_chunk().is_none());
    buf.push(&[4.0, 5.0]);
    assert_eq!(buf.take_chunk(), Some(vec![1.0, 2.0, 3.0, 4.0]));
    assert_eq!(buf.remaining(), 1);
}
