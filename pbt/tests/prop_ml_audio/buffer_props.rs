//! `src/ml/audio/buffer.rs` の `AudioChunkBuffer` に対する PBT。

use std::num::NonZeroUsize;

use hisui::ml::audio::AudioChunkBuffer;
use proptest::prelude::*;

/// VAD 経路の正規化 PCM 相当の `[-1.0, 1.0]` 範囲の f32 サンプル列を生成する Strategy。
///
/// `any::<f32>()` にすると NaN / Inf が混入し、`Vec<f32>::PartialEq` が要素ごとに `f32::eq` で
/// 比較するため NaN == NaN が false になって順序保存アサーションが seed 依存で失敗する。
/// AudioChunkBuffer 自体は値の意味に関与しないため、VAD 相当の有限範囲で十分。
fn arb_pcm(max_len: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(-1.0f32..1.0f32, 0..max_len)
}

/// PBT 内で `NonZeroUsize` を組み立てる補助関数 (0 を渡すとテスト失敗)。
fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("chunk_samples must be > 0")
}

proptest! {
    /// push した総サンプル数 == take_chunk で取り出した総サンプル数 + remaining() が常に成り立つ。
    #[test]
    fn total_samples_are_preserved(
        chunk_samples in 1usize..64,
        pushes in prop::collection::vec(arb_pcm(128), 0..8),
    ) {
        let mut buf = AudioChunkBuffer::new(nz(chunk_samples));
        let mut total_pushed = 0usize;
        for pcm in &pushes {
            buf.push(pcm);
            total_pushed += pcm.len();
        }
        let mut total_taken = 0usize;
        while let Some(chunk) = buf.take_chunk() {
            prop_assert_eq!(chunk.len(), chunk_samples, "取り出したチャンクは chunk_samples に一致するはず");
            total_taken += chunk.len();
        }
        prop_assert_eq!(total_pushed, total_taken + buf.remaining(),
            "push した総サンプル数は取り出した総数 + 残余に等しいはず");
    }

    /// push → take_chunk → push で追加した順序が保存される。
    #[test]
    fn order_is_preserved_across_interleaved_push_take(
        chunk_samples in 1usize..32,
        left in arb_pcm(64),
        right in arb_pcm(64),
    ) {
        let mut buf = AudioChunkBuffer::new(nz(chunk_samples));
        // まとめ push して取り出した結果。
        buf.push(&left);
        buf.push(&right);
        let mut combined_output = Vec::new();
        while let Some(chunk) = buf.take_chunk() {
            combined_output.extend(chunk);
        }
        let remaining_combined = buf.remaining();

        // 分けて push (途中で 1 チャンクだけ take) しても、最終的な連結順序が変わらない。
        let mut buf = AudioChunkBuffer::new(nz(chunk_samples));
        buf.push(&left);
        let mut split_output = Vec::new();
        if let Some(chunk) = buf.take_chunk() {
            split_output.extend(chunk);
        }
        buf.push(&right);
        while let Some(chunk) = buf.take_chunk() {
            split_output.extend(chunk);
        }
        let remaining_split = buf.remaining();

        prop_assert_eq!(combined_output, split_output, "順序は push した順のはず");
        prop_assert_eq!(remaining_combined, remaining_split, "残余は同じ数のはず");
    }

    /// take_chunk は chunk_samples 未満の残余が残るときは必ず None を返す。
    #[test]
    fn take_chunk_returns_none_when_remaining_is_short(
        chunk_samples in 2usize..32,
        short_pcm in arb_pcm(32),
    ) {
        let mut buf = AudioChunkBuffer::new(nz(chunk_samples));
        // chunk_samples 未満だけを push する。
        let truncated: Vec<f32> = short_pcm.into_iter().take(chunk_samples - 1).collect();
        let pushed_len = truncated.len();
        buf.push(&truncated);
        prop_assert!(buf.take_chunk().is_none(), "chunk_samples 未満では取れないはず");
        prop_assert_eq!(buf.remaining(), pushed_len, "残余は push した数のはず");
    }
}
