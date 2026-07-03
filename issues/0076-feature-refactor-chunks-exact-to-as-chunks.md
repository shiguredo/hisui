# [REFACTOR] chunks_exact(N) を as_chunks::<N> に置き換える

- Priority: Low
- Created: 2026-07-03
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-chunks-exact-to-as-chunks
- Polished: {YYYY-MM-DD}

## 目的

`Vec<u8>` / `&[T]` を chunk_size がリテラル定数な `chunks_exact(N)` で分割している既存 15 箇所を、Rust 1.88.0 で安定化された `as_chunks::<N>` に置き換える。

意図:

- **型安全**: `as_chunks::<N>` は `(&[[T; N]], &[T])` を返し、chunk が配列参照 `&[T; N]` になる。呼び出し側で `c[0] / c[1] / ...` のインデックスアクセスに伴う range check を排除できる
- **意図の明示**: `chunk_size` がコンパイル時定数であることを型で表す
- **doc 推奨**: [`chunks_exact` の std doc](https://doc.rust-lang.org/std/primitive.slice.html#method.chunks_exact) 自身が "If your chunk_size is a constant, consider using as_chunks instead, which will give references to arrays of exactly that length, rather than slices." と明示的に推奨している

## 優先度根拠

Low。既存動作に問題はなく、可読性・型安全の改善のみ。CI は現状通っており緊急性はない。手が空いたときに順次対応する候補。

## 現状

`chunk_size` がリテラル定数な `chunks_exact(N)` は現在 hisui 内で 15 箇所ある。

| ファイル:行 | パターン | 用途 |
|---|---|---|
| `src/video.rs:345, 380, 414` | `plane[..size * 2].chunks_exact(2)` | u8 バイト列 → u16 サンプル復号 |
| `src/video.rs:363, 397, 431` | `row_data.chunks_exact(2)` | u8 バイト列 → u16 サンプル復号 |
| `src/audio.rs:117, 137` | `self.data.chunks_exact(4)` | I16Be ステレオ (2 サンプル × 2 バイト) 復号 |
| `src/sora_source.rs:482` | `frame.data.chunks_exact(2)` | u8 バイト列 → i16 復号 |
| `src/obsws/player.rs:160` | `data.chunks_exact(2)` | u8 バイト列 → i16 復号 |
| `src/obsws/source/audio_device.rs:89` | `chunks_exact(2)` | u8 バイト列 → i16 復号 |
| `src/obsws/source/audio_device.rs:106` | `chunks_exact(4)` | I16Be ステレオ復号 |
| `src/audio/converter.rs:229, 245, 362` | `chunks_exact(2)` | u8 バイト列 → i16 復号、interleaved 対処 |
| `src/mixer/audio.rs:943` | `chunks_exact(2)` | audio データ処理 |
| `src/webrtc/audio.rs:36` | `chunks_exact(2)` | audio データ処理 |

各箇所の書き方は微妙に異なる (`map` / `for` / `flat_map`)、事前の長さ検証の有無も異なる。

参考実装: `src/audio/resample.rs` の stereo→mono ダウンミックスは 0061 のブランチで既に `as_chunks::<2>` に置き換え済み。

```rust
// 事前に偶数長を検証しているので `as_chunks::<2>` の余りは常に空。
let (chunks, _) = pcm.as_chunks::<2>();
chunks.iter().map(|&[l, r]| (l + r) * 0.5).collect()
```

## 設計方針

各箇所を個別に確認して書き換える。単純な機械的置換ではなく、以下を意識する。

1. **余り (`&[T]`) の扱い**: 事前に長さの倍数性を検証済みなら `_` で捨てる。検証していない箇所は既存の動作 (余りを暗黙的に無視) を維持するか、明示的にエラー扱いするかを判断する (現状動作維持がデフォルト方針)
2. **パターン**: `chunks_exact(N).map(|c| ...c[0]...c[N-1]...)` → `as_chunks::<N>().0.iter().map(|&[a, b, ...]| ...)` の形にする
3. **for ループ**: `for chunk in ....chunks_exact(N)` → `for [a, b, ...] in ....as_chunks::<N>().0.iter().copied()` 相当

## 完了条件

- 15 箇所すべてで `chunks_exact(N)` (N はリテラル定数) が `as_chunks::<N>` ベースに置き換わっている
- `cargo clippy --workspace --all-targets --all-features -- --deny warnings` が pass
- `cargo test --workspace` が pass
- 既存テストで書き換え箇所の動作が以前と同じであることを担保する (追加テストは基本不要)

## 解決方法

上記の 15 箇所を対象ファイルごとに順次書き換える。参考実装 (`src/audio/resample.rs`) の書き換え差分に倣う。1 コミットにまとめても、ファイル単位で分けてもよい (レビュー単位が扱いやすい粒度を選ぶ)。
