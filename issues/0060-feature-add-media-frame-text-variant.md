# MediaFrame に Text バリアントを追加する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-media-frame-text-variant
- Polished:

## 目的

文字起こし結果や将来のテキストメタデータを MediaPipeline 上で扱えるよう、`MediaFrame` enum に `Text(Arc<TextFrame>)` バリアントを追加する。これは親 issue 0012 系列の MediaPipeline 拡張層であり、0062 (Whisper 推論基盤) が結果を publish_track で流す際の受け皿となる。

## 優先度根拠

本系列の中核 0062 / 0063 の前提となるが、本 issue 単独では利用者向けの機能を提供しない。後続 issue がマージされて初めて利用者から見える機能が完成するため、Medium。

## 現状

- `src/media.rs` の `MediaFrame` enum は `Audio(Arc<AudioFrame>)` / `Video(Arc<VideoFrame>)` の 2 バリアントのみ
- 既存の使用箇所は src 配下で 97 箇所 (内訳: Video 44, Audio 37, コンストラクタ系 16)
- 多くの match は `Audio / Video` をそれぞれ exhaustive で処理する形 (片方を処理、片方を握りつぶす or Err)、`#[non_exhaustive]` も `_` ワイルドカードもほぼ使われていない
- 0013 (テキストオーバーレイ、closed 済み) でも将来検討事項として `MediaFrame::Text` の導入が議論されていた

## 設計方針

### TextFrame 構造体

新規追加 (場所は `src/media.rs` か `src/text.rs`、実装時に判断):

```rust
pub struct TextFrame {
    pub start: Duration,             // 発話開始時刻 (track 基準)
    pub end: Duration,               // 発話終了時刻
    pub text: String,                // 文字起こしテキスト等
    pub language: Option<String>,    // 言語コード ("ja" 等)
    pub no_speech_prob: Option<f32>, // Whisper の no_speech_prob (幻覚指標)
    pub avg_logprob: Option<f32>,    // Whisper の平均 log probability (信頼度目安)
}
```

### MediaFrame 拡張

- `MediaFrame::Text(Arc<TextFrame>)` バリアントを追加
- `MediaFrame::timestamp()` は `start` を返す (既存と整合)
- `expect_text(self) -> Result<Arc<TextFrame>>` を追加 (既存 `expect_audio` / `expect_video` と同様)
- `new_text(frame: TextFrame) -> Self` / `text(frame: TextFrame) -> Self` コンストラクタを追加
- `input_track_id` は持たない (既存 Audio / Video と同流儀。subscribe した TrackId で判別)

### 既存 match 箇所への対応

ripgrep で `MediaFrame::Audio` / `MediaFrame::Video` を全箇所走査し、`MediaFrame::Text` ブランチを追加する。多くは:

- writer 等 (Text を購読しない側): `MediaFrame::Text(_) => {}` で無視
- Audio / Video 専用処理の入口 (mixer 等): `MediaFrame::Text(_) => Err(Error::new("..."))` で明示的にエラー

### `#[non_exhaustive]` は付けない

既存流儀 (網羅性検査が効く形) を維持する。

## 完了条件

- `MediaFrame::Text` バリアントと `TextFrame` 構造体が追加されている
- 全 match 箇所が網羅されてビルド成功 (`cargo check --workspace` / `cargo clippy --workspace --all-targets -- --deny warnings`)
- `cargo test --workspace` が green (既存テストが壊れていないことの確認)
- MediaFrame::Text 自体の単体テスト (timestamp / expect_text / コンストラクタ) が追加されている

## 解決方法

`src/media.rs` (および必要なら `src/text.rs`) に `Text` バリアントと `TextFrame` 構造体を追加する。続いて ripgrep で既存の MediaFrame match 箇所を走査して、各々に Text ブランチを追加する。本 issue は構造拡張のみで、実際に MediaFrame::Text を publish / subscribe する箇所は別 issue (0062) で実装する。
