# RTSP AU header パース用の独自 BitReader を共有 BitReader に統合する

- Priority: Low
- Created: 2026-06-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-unify-rtsp-au-header-bit-reader
- Polished: {YYYY-MM-DD}

## 目的

closed/0048 で `src/video/bit_reader.rs::BitReader` を codec 中立な汎用ビットリーダーとして新設し、H.264 / H.265 SPS パーサで共有するように整理した。一方で `src/rtsp/subscriber.rs` には RTP MPEG4-GENERIC AU header をパースするための独自 `BitReader` 構造体が以前から存在しており、機能的にほぼ同等 (MSB-first / バッファ枯渇で Err) でありながら別実装になっている。`BitReader` という汎用名前空間を 2 箇所が確保している状態を解消する。

## 優先度根拠

Low。

- 機能的なバグや実害はなく、純粋に「Don't live with broken windows」観点の broken window 解消。
- subscriber 側の利用箇所は `extract_audio_data` 内 1 箇所のみで影響範囲が狭い。
- closed/0048 で BitReader の共有化方針を明示しているため、片肺のまま放置するとレビュー時の認知負荷が継続する。

## 現状

- `src/video/bit_reader.rs::BitReader` (pub、closed/0048 で新設)
  - フィールド: `data: &[u8]` / `byte_pos: usize` / `bit_pos: u8`
  - API: `read_u(n: usize) -> Result<u32>` / `read_ue` / `read_se` / `skip_u` / `skip_ue` / `skip_se`
  - `n > 32` で Err、バッファ末尾超過で Err
  - エラーメッセージ: `"bit reader: exhausted before requested read"` 等
- `src/rtsp/subscriber.rs::BitReader` (非 pub、行 1111-1145)
  - フィールド: `bytes: &[u8]` / `bit_offset: usize`
  - API: `read_bits(bit_count: u8) -> Result<u32>` のみ (Exp-Golomb 系は無い)
  - `bit_count == 0` で `Ok(0)`、バッファ末尾超過で Err
  - エラーメッセージ: `"bitstream is truncated"`
- 呼び出し箇所: `src/rtsp/subscriber.rs:1071` で `BitReader::new(au_headers)` から `read_bits` を複数回呼んでいる (MPEG4-GENERIC AU header の `size_length` / `index_length` ビット読み出し)。

## 設計方針

`src/rtsp/subscriber.rs::BitReader` を削除し、`crate::video::bit_reader::BitReader` を import して利用する。

API 差分の解消:

- `read_bits(bit_count: u8)` → `read_u(bit_count as usize)` への置き換え。
- subscriber 側の `bit_count == 0` で `Ok(0)` 経路は、video 側の `read_u(0)` でもループ 0 回で `value = 0` を返すため挙動が一致する (実装着手時に再確認する)。
- エラーメッセージは `"bit reader: exhausted before requested read"` に変わる。RTSP 側で文言を assert しているテストが無いことを実装着手時に確認する。

統合不可と判断した場合の代替: subscriber 側を温存しつつ名前を狭める (`AuHeaderBitReader` 等)。ただしまずは統合可能性を優先する。

## 完了条件

- `src/rtsp/subscriber.rs::BitReader` 構造体と `impl` が削除されている。
- `src/rtsp/subscriber.rs` 内の呼び出しが `crate::video::bit_reader::BitReader` を使う形に置き換わっている。
- 既存テストが全て pass する (`cargo test`、特に RTSP 関連 / AU header パース関連)。
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo fmt --all -- --check` がパスする。

## 解決方法

実装着手後にここに記述する。
