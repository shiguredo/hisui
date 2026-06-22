# writer 入口の sample_entry fallback 補完経路を削除する

- Priority: Low
- Created: 2026-06-22
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-writer-sample-entry-fallback
- Polished:

## 目的

issue 0039 の調査結果として、入力側全経路（リーダー: 0030 / 0031 / 0032 / 0033、エンコーダ: 0017 / 0027）で「圧縮（エンコード済み）フレームは常に sample_entry を持つ」不変条件が確立済みであり、issue 0034 で writer 入口に保険として導入された fallback 補完経路は本番経路で発火しないデッドコードとなっている。本 issue ではこれを全削除し、加えて入力側不変条件を `docs/internals/` 配下に明文化することで責任の所在をコード外に明示する。

## 優先度根拠

Low。デッドコードに近いコード削減のリファクタリングで、機能には影響しない。fallback コードを残しても実害は無い。ただし「念のため」の保険として writer 側で保持し続けるコスト（per-frame の `&mut` 借用と Arc 更新、16 サイトの match 重複、warn 文字列の維持）と「不変条件の責任の所在を入力側に集約する」価値を比較した結果、削除側に倒すのが妥当と判定された（issue 0039 の調査結論）。

## 現状

issue 0034 で導入された writer 入口の不変条件違反検知 + fallback 補完経路が以下に存在する。

- `src/sample_entry.rs` の `resolve_audio_sample_entry` / `resolve_video_sample_entry` 関数と `SampleEntryResolution<T>` enum（`Pass` / `Patched` / `Skip` の 3 バリアント）
- 4 writer（`Mp4Writer` / `HybridMp4Writer` / `DashWriter` / `HlsWriter`）の `fallback_audio_sample_entry` / `fallback_video_sample_entry` フィールドと writer 入口での `resolve_*_sample_entry` 呼び出し
- 16 サイト（4 writer × 2（音声 / 映像）× 2（Patched / Skip））の `tracing::warn!` "encoded-frame invariant violated"

issue 0039 の調査により以下が確認された。

- 入力側全経路で「圧縮フレームは常に sample_entry を持つ」不変条件が確立済み（リーダー側: 0030 / 0031 / 0032 / 0033、エンコーダ側: 0017 / 0027 で完了）
- `SampleEntryResolution::Patched` / `Skip` を発火させるのはテストコードのみ（`src/sample_entry.rs` 単体テスト 10 件、`pbt/tests/prop_sample_entry.rs` PBT 8 件、`src/mp4/hybrid_writer.rs` 単体テスト 8 件）。本番経路で発火する経路は存在しない
- 0034 当時「将来の入力経路変更への保険」と判断された設計は、入力側全経路で不変条件が確立した今は writer 側に保険を残す価値より「責任の所在を入力側に集約する」価値の方が大きい

## 設計方針

### 1. fallback 補完経路の削除

`src/sample_entry.rs`:

- `resolve_audio_sample_entry` / `resolve_video_sample_entry` 関数を削除する
- `SampleEntryResolution<T>` enum を削除する
- `mod tests` 配下の単体テスト 10 件（音声 4 + 映像 4 + `ptr_eq` 2）を削除する
- `SharedSampleEntry::ptr_eq` は **残す**（fallback とは独立した `changed_since` 短絡経路観測用）

4 writer（`src/mp4/writer.rs` / `src/mp4/hybrid_writer.rs` / `src/dash/writer.rs` / `src/hls/writer.rs`）:

- 構造体定義から `fallback_audio_sample_entry: Option<SharedSampleEntry>` / `fallback_video_sample_entry: Option<SharedSampleEntry>` フィールドを削除する
- コンストラクタの `fallback_*_sample_entry: None` 初期化を削除する
- writer 入口の `match crate::sample_entry::resolve_*_sample_entry(...)` を削除し、`Pass` パスのみが残る前提のフレーム受け渡しに置き換える（実装時に各 writer のシグネチャに応じて確定）

置き換えの方針:

- `Mp4Writer` / `HybridMp4Writer`: `Arc<AudioFrame>` / `Arc<VideoFrame>` を受け取って `core.handle_input_sample` に流すパスから resolve 呼び出しを削除する
- `DashWriter` / `HlsWriter`: `&AudioFrame` / `&VideoFrame` を受け取って append するパスから resolve 呼び出しを削除する

### 2. テストの削除

`src/mp4/hybrid_writer.rs` の `mod tests` から以下 8 件を削除する。

- `hybrid_writer_falls_back_on_missing_sample_entry_audio`
- `hybrid_writer_falls_back_on_missing_sample_entry_video`
- `hybrid_writer_skips_first_frame_when_missing_sample_entry_audio`
- `hybrid_writer_skips_first_frame_when_missing_sample_entry_video`
- `hybrid_writer_resolves_sample_entry_even_when_audio_track_id_is_disabled`
- `hybrid_writer_resolves_sample_entry_even_when_video_track_id_is_disabled`
- `hybrid_writer_preserves_fallback_across_consecutive_violations_audio`
- `hybrid_writer_preserves_fallback_across_consecutive_violations_video`

これらに付随する共通ヘルパ（`make_audio_frame` / `make_video_frame` の `sample_entry: Option<SampleEntry>` 引数を取る形）は他テストで利用されているなら残す。利用されていなければ削除する。

fallback と無関係な既存テスト（`hybrid_writer_finalizes_readable_streams_with_per_frame_sample_entry` 等）は **残す**。

`pbt/tests/prop_sample_entry.rs`:

- ファイル全件を削除する（PBT 8 件 + ヘルパ）
- `pbt/Cargo.toml` から `shiguredo_mp4` dev-dependency を削除する（0034 で本 PBT 用に追加された分。他で使われていなければ完全に削除する）

### 3. `maybe_flush_initial_pending` のガード処理

`src/mp4/hybrid_writer.rs::HybridMp4Writer::maybe_flush_initial_pending` の `&& let Some(ref sample_entry) = pending.sample_entry` ガード（`src/mp4/hybrid_writer.rs:403` / `:419`）は **残す**。

理由:

- fallback 補完経路の有無に依存しない独立した「ベストエフォート設計」（リカバリ用 moov 先行更新は失敗時に panic しない方針）
- 入力側不変条件で pending.sample_entry は常に Some だが、`if let Some` パターンは「ベストエフォート＝失敗時に panic しない」設計動機を独立して保つ

ただしコメント文言（`src/mp4/hybrid_writer.rs:398-401`）は「writer 入口の fallback で sample_entry が補完済み」を前提にしているため、「入力側不変条件で常に Some」を前提にした文言へ書き換える。

### 4. docs/internals/ に不変条件を明文化

入力側で「圧縮（エンコード済み）フレームには常に sample_entry を付与する」不変条件を、`docs/internals/` 配下の新規ドキュメントとして明文化する。

目的:

- リーダー / エンコーダ各経路で段階的に確立された不変条件の所在をコード外に明示する
- 将来の入力経路追加時にこの規約を踏襲することを担保する
- writer 側で fallback を残さない判断（本 issue の主旨）の根拠となる「不変条件は入力側で守る」設計方針を明示する

記載内容:

- 不変条件の定義: 「圧縮フォーマット（`format.codec_name().is_some()`）のフレームの `sample_entry` は必ず `Some` であること」
- 適用範囲: 入力側全経路
  - リーダー: `src/mp4/reader.rs`、`src/webm/reader.rs`、`src/rtsp/subscriber.rs`、`src/srt/inbound_endpoint.rs`、`src/sora/recording_mp4_reader.rs`
  - エンコーダ: `src/encoder/openh264.rs`、`src/encoder/svt_av1.rs`、`src/encoder/libvpx.rs`、`src/encoder/video_toolbox.rs`、`src/encoder/nvcodec.rs`、`src/encoder/fdk_aac.rs`、`src/encoder/audio_toolbox.rs`、`src/encoder/opus.rs`
- 生フォーマット（`format.codec_name().is_none()`）は対象外
- 新規入力経路追加時のチェックリスト（どの段階で sample_entry を確立すべきか、確立できない場合のテスト戦略）

`AudioFrame.sample_entry`（`src/audio.rs:92`）と `VideoFrame.sample_entry`（`src/video.rs:56`）の docstring からこのドキュメントへリンクを張る（相対パス参照）。

ファイル名・配置は実装時に確定する（例: `docs/internals/sample_entry_invariant.md` 等。`docs/internals/README.md` の目次にも追記する）。

### CHANGES.md

本 issue で削除する fallback コードは issue 0034 で develop に追加されたもので未リリース。`shiguredo-changelog` の「派生元ブランチとの最終的な差分のみを記載すること」「開発ブランチ内の中間状態の修正は記載しないこと」に従い、最終 diff として現れないため記載対象外と判断する見込み。

`docs/internals/` の新規ドキュメント追加についても、未リリース状態のリポジトリ内部ドキュメント整備のため CHANGES.md 記載対象外の見込み。

実装時に最終確認する。

## 完了条件

- 設計方針 1〜4 の削除・追加・書き換えが実装されていること
- 以下のシンボル名が `rg` で検索しても結果が空であること:
  - `resolve_audio_sample_entry` / `resolve_video_sample_entry`
  - `SampleEntryResolution`
  - `fallback_audio_sample_entry` / `fallback_video_sample_entry`
- `pbt/tests/prop_sample_entry.rs` がリポジトリから消えていること
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `fdk-aac` / `nvcodec` / `video_toolbox` を含む）
- 既存 e2e テスト（`e2e-tests/obsws/test_output.py` 等の HLS / DASH / MP4 / SRT 関連）が通ること
- `docs/internals/` 配下の新規ドキュメントが追加され、`AudioFrame.sample_entry` / `VideoFrame.sample_entry` の docstring からリンクされていること
- `HybridMp4Writer::maybe_flush_initial_pending` の `&& let Some(...)` ガードが残っており、コメント文言が「入力側不変条件で常に Some」を前提にした内容に書き換えられていること

## 関連

- issue 0039（調査 issue。本 issue の根拠）
- issue 0034（fallback 補完経路を導入。本 issue で削除する）
- issue 0030 / 0031 / 0032 / 0033（入力側リーダー経路の不変条件確立）
- issue 0017 / 0027（エンコーダ経路の不変条件確立）
