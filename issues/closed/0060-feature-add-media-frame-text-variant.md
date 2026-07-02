# MediaFrame に Text バリアントを追加する

- Priority: Medium
- Created: 2026-06-24
- Completed: 2026-07-02
- Model: Opus 4.7
- Branch: feature/add-media-frame-text-variant
- Polished: 2026-06-30

## 目的

文字起こし結果や将来のテキストメタデータを MediaPipeline 上で扱えるよう、`MediaFrame` enum に `Text(Arc<TextFrame>)` バリアントを追加する。親 issue 0012 で確定済みの「MediaPipeline 流儀準拠で `MediaFrame::Text(Arc<TextFrame>)` を新設し、TranscriptionProcessor は publish_track で結果を流す」方針 (0012 L67-71) の実装担当。

## 優先度根拠

本系列の中核 0062 / 0063 の前提だが、本 issue 単独では利用者から見える機能を提供しないため Medium。

## 現状

- `src/media.rs` の `MediaFrame` enum は `Audio(Arc<AudioFrame>)` / `Video(Arc<VideoFrame>)` の 2 バリアントのみ
- 既存 `AudioFrame` / `VideoFrame` は `#[derive(Debug, Clone)]` のみ (`src/audio.rs` / `src/video.rs` 参照)
- `MediaFrame::Audio` / `MediaFrame::Video` の match 箇所は src + tests を `rg -n 'MediaFrame::(Audio|Video)' src/ tests/` で走査して把握する
- 既存 match は概ね Audio / Video を網羅処理する形だが、以下 3 種が混在する
  - 網羅 match (Audio / Video それぞれ独立処理、`_ =>` なし)
  - `_ => {}` フォールスルーで Audio / Video 以外を握りつぶす match (例: `src/dash/writer.rs`、`src/hls/writer.rs`)
  - `_ => Err("BUG: unexpected input stream")` フォールスルーでバグを検出する match (例: `src/mp4/writer.rs:487-509` の `(InputTrackKind, Option<MediaFrame>)` タプル match)
- `MediaFrame` 以外で類似の網羅 match を持つ局所 enum: `src/rtmp/outbound_endpoint.rs:280-283` の `ClientMediaFrame { Audio, Video }` (実装着手時に `rg -n 'enum.*\{' src/` で他に同種がないか確認する)
- `#[non_exhaustive]` は使われていない

## 設計方針

### text 専用 track 運用

TranscriptionProcessor は **入力 audio track とは別の text 専用 track** を新規に作って publish_track する。subscriber は用途に応じて選択的に subscribe する。

- `transcribe` サブコマンド (0063): text track を subscribe して標準出力に JSON LINE 出力
- obsws: text track を subscribe して Event 変換層を通して WebSocket 配信
- 字幕オーバーレイブリッジ (将来の別 issue): text track + 描画レイアウト設定を組み合わせて VideoRealtimeMixer に描画指示
- audio / video subscriber (`src/mp4/writer.rs`、`src/mixer/audio.rs`、`src/mixer/video.rs`、`src/sora/recording_*` 等): text track を subscribe しないため、TextFrame は流入しない

この track 分離運用が以下を成立させる:

- **subscriber 側の柔軟性**: 用途ごとに subscribe を選択することで、結果データの流れを subscribe トポロジーで制御できる
- **「Text が流入しない」保証**: subscribe_track で text 専用 track を購読しない subscriber には MediaFrame::Text が流れない。後述パターン A / パターン C の「Text 流入想定なし」分類はこの track 分離で担保する
- **MediaFrame の概念汚染を限定**: MediaFrame に Text バリアントを足すが、audio/video subscriber 側からは「subscribe しない track のバリアント」として実質的に隔離されるため、概念上の混在は限定的になる
- **既存 MediaPipeline 機構の流用**: 新規の broadcast 機構や独自整列ロジックを設計せず、`subscribe_track` / `publish_track` / `TrackId` / `register_processor` をそのまま使える

N:1 集約 (複数 audio → 1 text) や用途別 track 分離が必要になった場合は、TranscriptionProcessor の登録単位を増やすか track 設計を見直すことで対応する。本 issue は構造拡張のみで、track 設計の詳細は 0062 で確定する。

### TextFrame 構造体

`src/text.rs` を新設し、以下を定義する (既存 `src/audio.rs` / `src/video.rs` と同じ流儀)。`src/lib.rs` に `pub mod text;` と `pub use text::TextFrame;` を追加する。

```rust
/// 文字起こし結果や将来のテキストメタデータを表すフレーム。
#[derive(Debug, Clone)]
pub struct TextFrame {
    /// 発話開始時刻 (track 基準、AudioFrame.timestamp / VideoFrame.timestamp と同じ意味論)
    pub start: Duration,
    /// 発話終了時刻。`start <= end` を呼び出し側が保証する (validation は持たない)
    pub end: Duration,
    /// 文字起こしテキスト等
    pub text: String,
    /// ISO 639-1 (2 文字小文字) の言語コード ("ja" 等)。検出失敗時や言語推定なしの場合は None
    pub language: Option<String>,
    /// Whisper の no_speech_prob (幻覚指標、0.0 - 1.0)。Whisper 以外の生成元では None
    pub no_speech_prob: Option<f32>,
    /// Whisper の平均 log probability (信頼度目安)。Whisper 以外の生成元では None
    pub avg_logprob: Option<f32>,
}
```

設計判断:

- `#[derive(Debug, Clone)]` は既存 AudioFrame / VideoFrame に揃える
- `MediaFrame::Text` の中身は `Arc<TextFrame>` で包む。既存 Audio / Video と同流儀で broadcast コストを最小化する
- `sample_entry` フィールドは持たない。TextFrame は MP4 等の container に書き出さないため、`docs/internals/sample_entry_invariant.md` の不変条件対象外
- `start` / `end` の二本立て: Whisper 出力は本質的に発話区間で表現され、0063 の JSON LINE 出力でもそのまま使う。Audio / Video の `timestamp` 単一フィールドとは意味論的に区別する
- `MediaFrame::timestamp()` が返す値は `start` (下流の整列順序は start 基準)。`end` は subscriber が `expect_text()` 後に TextFrame からアクセスする
- `no_speech_prob` / `avg_logprob` を `Option<f32>` にする理由: TextFrame は将来 Whisper 以外の生成元 (字幕入力等) からも使い得るため。Whisper の出力経路では常に `Some` が期待される
- `language` を `Option<String>` にする理由: Whisper の言語自動検出が失敗するケース、および TextFrame を Whisper 以外から生成するケースで `None` を許容する
- `input_track_id` は持たない (既存 Audio / Video と同流儀。subscribe した TrackId で判別)。text 専用 track 運用 (上述) と組み合わせることで、subscriber は自分が subscribe した text track の TrackId を知っているため、フレームに埋める必要はない

### MediaFrame 拡張

- `MediaFrame::Text(Arc<TextFrame>)` バリアントを追加
- `MediaFrame::timestamp()` は Text の場合に `frame.start` を返す
- `expect_text(self) -> Result<Arc<TextFrame>>` を追加 (既存 `expect_audio` / `expect_video` と同様)
- `new_text(frame: TextFrame) -> Self` コンストラクタを追加。`text(...)` 重複コンストラクタは作らない
- `MediaFrame::kind_name(&self) -> &'static str` ヘルパーを追加。Audio → `"audio"`、Video → `"video"`、Text → `"text"` を返す。`Display` ではなく専用メソッドにする理由は既存 `#[derive(Debug)]` との混同を避けるため
- `expect_audio` / `expect_video` の既存固定文言「expected an audio sample, but got a video sample」を kind_name() ベースで動的化する。**冠詞の取り違え (`"a audio sample"`) を避けるため、文言テンプレは冠詞無しの `format!("expected {} sample, but got {} sample", expected_kind, self.kind_name())` を採用する**。self は move されるため、`let actual = self.kind_name(); match self { Self::Audio(f) => Ok(f), _ => Err(Error::new(format!("expected audio sample, but got {} sample", actual))) }` のように、先頭で `kind_name()` を取得してから match する
- 同型「expected ... sample, ...」の固定文言が地の文で散在する。実装着手時に `rg -n 'expected.*sample' src/` で網羅し、`src/subcommand_inspect.rs` (4 箇所)、`src/mixer/audio.rs:823`、`src/mixer/video.rs:772`、`src/sora/recording_mixer_audio.rs:271`、`src/sora/recording_video_mixer.rs:603`、`src/rtmp/publisher.rs:243,269`、`src/rtmp/outbound_endpoint.rs:244,270`、`src/sora/recording_subcommand_vmaf.rs:836,878`、`src/yuv.rs:43` 等を同じ冠詞無しテンプレで統一する。track id 埋め込み版 (例: `"expected audio sample on track {tid}, but got {} sample"`) は trackid 補間部分を残しつつ kind 部分のみ動的化する
- `#[non_exhaustive]` は付けない。既存流儀を維持し、網羅性検査による漏れ検知のメリットを優先する
- feature gate しない (`cfg(feature = "candle")` 配下に置かない)

### 既存 match 箇所への対応

Text バリアント追加で網羅性検査がコンパイル時に効くため、対応漏れは `cargo check --workspace --all-features` で機械的に検出できる。各箇所を以下 3 パターンに分類して対応する (判定基準: 実コードに `_ =>` フォールスルーがあるかと、その内容が `{}` か `Err` か)。

- **パターン A (網羅 match、Text を購読しない側): `MediaFrame::Text(_) => {}` で握りつぶす**
  - text 専用 track 運用 (前述) によって subscribe 経路上 Text が流れてこない writer / transform 系
  - 握りつぶす根拠を 1 行の日本語コメントで残す
- **パターン B (網羅 match、Audio / Video 専用処理の入口): `MediaFrame::Text(_) => Err(...)` で明示エラー**
  - エラー文言は kind_name() ベースの統一テンプレを使う
- **パターン C (既存 `_ =>` フォールスルー有り): Text ブランチを明示追加することを原則とする**
  - text 専用 track 運用 (前述) によって実運用上は Text が流入しないが、コンパイル時の網羅性検査と将来のバリアント追加時の検出を優先して明示分岐を入れる
  - 単純 match (例: `src/dash/writer.rs` 等): `MediaFrame::Text(_) =>` を `_ =>` の前に挿入し、`_ =>` を撤去
  - タプル match (例: `src/mp4/writer.rs:487-509` の `match (track_kind, sample)`): `MediaFrame::Text(_) =>` 単独構文は書けない。`(_, Some(MediaFrame::Text(_))) => Err(...)` を既存 `_ => Err("BUG: ...")` の前に挿入し、`_ =>` は BUG 用 (track_kind と Audio/Video の不一致検出) として温存する。エラー文言はパターン B と同じ kind_name() ベースの統一テンプレを使う
  - 局所 enum に MediaFrame を変換する箇所 (例: `src/rtmp/outbound_endpoint.rs:397-406` の `MediaFrame → ClientMediaFrame` 変換): 局所 enum 側に `Text` バリアントを追加せず、`MediaFrame::Text(_) => Err(...)` で MediaFrame 段階で弾く

### matches! 等の利用箇所

`matches!(message, Message::Media(MediaFrame::Video(_)))` のような matches! / if let による単一バリアント判定は網羅性検査が効かない。実装着手時に `rg -n 'matches!.*MediaFrame::|if let.*MediaFrame::' src/ tests/` で全箇所を洗い出し、Text 流入時の想定挙動を確認する。テストコードの if let / matches! 箇所は Text 流入想定なしのため、コンパイル成功すれば追加対応は不要。

## 完了条件

- `MediaFrame::Text` バリアントと `TextFrame` 構造体が追加され、`src/lib.rs` から `pub use` されている
- `MediaFrame::kind_name()` ヘルパーが追加され、`expect_audio` / `expect_video` および地の文の同型エラーメッセージが kind_name() ベースの冠詞無しテンプレで統一されている
- 全 match 箇所が網羅されてビルド成功
  - `cargo fmt --all -- --check`
  - `cargo check --workspace --no-default-features`
  - `cargo check --workspace --all-features`
  - `cargo clippy --workspace --all-targets --all-features -- --deny warnings`
  - `cargo test --workspace`
- `tests/test_media.rs` と `tests/test_text.rs` を新設し、次を検証する単体テストが入っている (shiguredo-rust 規約「単体テストのファイル名は `tests/test_<module>.rs` とし、`src/<module>.rs` に対応させる」に準拠。既存 `src/audio.rs` / `src/video.rs` に対応する `tests/test_audio.rs` / `tests/test_video.rs` は未整備だが、本 issue では新規追加分のみ規約準拠で書く)
  - `MediaFrame::timestamp()` が Text 入力で `start` を返す (`start = 1s, end = 5s` 等、明示的に異なる値で `timestamp() == start` を assert する)
  - `expect_text` が Text 入力で Ok、Audio / Video 入力で Err を返す
  - `expect_audio` / `expect_video` が Text 入力で `"text"` を含む Err メッセージを返す
  - `MediaFrame::new_text` が TextFrame を Arc に包んで返す
  - `MediaFrame::kind_name()` が Audio → `"audio"`、Video → `"video"`、Text → `"text"` を返す
- `docs/internals/sample_entry_invariant.md` に「TextFrame は sample_entry を持たず本不変条件の対象外」を 1 段落追記する
- CHANGES.md エントリは追加しない (内部実装のため)

## 解決方法

設計方針に従って `src/text.rs` を新設、`src/media.rs` に Text バリアント・`expect_text` / `new_text` / `kind_name()` を追加、`rg` で既存 match 箇所を走査して 3 パターンに分類対応、`tests/test_media.rs` / `tests/test_text.rs` を新設、`docs/internals/sample_entry_invariant.md` を追記、完了条件の cargo コマンドを順に green にする。

本 issue は構造拡張のみで、実際に `MediaFrame::Text` を publish / subscribe する箇所 (TranscriptionProcessor 等) は別 issue (0062) で実装する。
