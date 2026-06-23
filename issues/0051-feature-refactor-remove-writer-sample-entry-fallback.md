# writer 入口の sample_entry fallback 補完経路を削除する

- Priority: Low
- Created: 2026-06-22
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-writer-sample-entry-fallback
- Polished: 2026-06-23

## 目的

issue 0039 の調査結論として、writer 入口の sample_entry fallback 補完経路（issue 0034 で保険として導入）は、入力側全経路で「圧縮フレームは常に sample_entry を持つ」不変条件が確立した今はデッドコード化している。本 issue ではこれを全削除し、削除の根拠となる不変条件を `docs/internals/` 配下に明文化することで責任の所在をコード外に明示する。

## 優先度根拠

Low。デッドコードに近いコード削減のリファクタリングで、機能には影響しない。fallback コードを残しても実害は無い。ただし「念のため」の保険として writer 側で保持し続けるコスト（per-frame の `&mut` 借用と Arc 更新、8 サイトの resolve 呼び出しブロック、16 個の warn 文字列、関連テスト群の維持）と「不変条件の責任の所在を入力側に集約する」価値を比較した結果、削除側に倒すのが妥当と判定された（issue 0039 の調査結論）。

## スコープ

含むもの:

- `src/sample_entry.rs` の `resolve_audio_sample_entry` / `resolve_video_sample_entry` 関数・`SampleEntryResolution<T>` enum・関連単体テスト 8 件（`resolve_audio_*` 4 件 + `resolve_video_*` 4 件）の削除
- 4 writer（`Mp4Writer` / `HybridMp4Writer` / `DashWriter` / `HlsWriter`）の `fallback_audio_sample_entry` / `fallback_video_sample_entry` フィールド・コンストラクタ初期化・入口 match 処理（8 サイト）の削除
- 上記削除に伴い未使用になる `use` 文（各 writer ファイルの `use crate::sample_entry::...` のうち削除対象シンボルへの参照のみのもの）の整理
- `src/mp4/hybrid_writer.rs` の単体テスト 8 件（fallback 関連）の削除
- `pbt/tests/prop_sample_entry.rs` 全件削除（PBT 8 件 + ヘルパ群）
- `HybridMp4Writer::maybe_flush_initial_pending` の `&& let Some(...)` ガード周辺コメントの書き換え
- `SharedSampleEntry::ptr_eq` docstring の書き換え（fallback 文脈 2 行の除去）
- `AudioFrame.sample_entry` / `VideoFrame.sample_entry` の docstring から不変条件ドキュメントへの参照追加
- `docs/internals/sample_entry_invariant.md` の新規作成と `docs/internals/README.md` の目次追記

含まないもの:

- `AudioFrame.sample_entry` / `VideoFrame.sample_entry` の型（`Option<SharedSampleEntry>`）自体の非 Option 化（影響範囲が桁違いに大きく、本 issue のスコープ外）
- `SharedSampleEntry::ptr_eq` メソッド自体の削除（`src/rtsp/subscriber.rs` / `src/webm/reader.rs` の他テストで `changed_since` 短絡経路観測用として利用継続するため残す。`src/mp4/hybrid_writer.rs` での現状の `ptr_eq` 利用は本 issue で削除する fallback テスト 8 件に集中しており、削除後は当該ファイル内では利用箇所が無くなるが、`SharedSampleEntry` の公開 API としての存在意義は他リーダー経路のテストで担保される）
- `changed_since_*` 関連の単体テスト 4 件（fallback と無関係。残す）
- decoder 側の sample_entry 抽出コード（issue 0039 の調査対象外。不変条件適用と削除可否は無関係）
- writer 周辺コードの既存コメント内に残る `issue NNNN` 形式の参照（shiguredo-issues 規約違反として issue 0034 から持ち越された負債。本 issue では清算しない。別 issue 起票が必要）

## 現状

writer 入口の不変条件違反検知 + fallback 補完経路:

- `src/sample_entry.rs` の `resolve_audio_sample_entry` / `resolve_video_sample_entry` 関数と `SampleEntryResolution<T>` enum（`Pass` / `Patched` / `Skip` の 3 バリアント）
- 4 writer の `fallback_audio_sample_entry` / `fallback_video_sample_entry` フィールドと writer 入口での `resolve_*_sample_entry` 呼び出し（4 writer × 2 媒体 = 8 サイト）
- 各サイトの `Patched` / `Skip` アームに `tracing::warn!` "encoded-frame invariant violated"（合計 16 個の warn 呼び出し）

issue 0039 の調査により以下が確認された:

- 入力側全経路で「圧縮（エンコード済み）フレームは常に sample_entry を持つ」不変条件が確立済み:
  - リーダー側: issue 0030（mp4 / RTSP / SRT AAC 音声）/ 0031（WebM）/ 0032（RTSP Annex-B 映像）/ 0033（SRT Annex-B 映像）
  - エンコーダ側: issue 0017（音声）/ 0027（映像）
- `SampleEntryResolution::Patched` / `Skip` を発火させるのはテストコードのみ（`src/sample_entry.rs` 単体テスト 8 件 + `pbt/tests/prop_sample_entry.rs` PBT 8 件 + `src/mp4/hybrid_writer.rs` 単体テスト 8 件）。本番経路で発火する経路は存在しない

## 設計方針

### 1. fallback 補完経路の削除

`src/sample_entry.rs`:

- `resolve_audio_sample_entry` / `resolve_video_sample_entry` 関数を削除する
- `SampleEntryResolution<T>` enum を削除する
- `mod tests` 配下のテストのうち `resolve_audio_*` 4 件と `resolve_video_*` 4 件（合計 8 件）を削除する
- `SharedSampleEntry::ptr_eq` の docstring（`src/sample_entry.rs:37-42`）から fallback 文脈の言及部分（`src/sample_entry.rs:39-40` の 2 行「fallback 補完値が直前の正常フレームの sample_entry を Arc 共有で保持できているかのテスト用途を想定する。」）のみを削除する。残りの「`changed_since` の `Arc::ptr_eq` 短絡経路が崩れた場合に検知できるよう、Arc の同一性だけを観測できる API として用意する」部分はそのまま維持する

4 writer（`src/mp4/writer.rs` / `src/mp4/hybrid_writer.rs` / `src/dash/writer.rs` / `src/hls/writer.rs`）:

- 構造体定義から `fallback_audio_sample_entry: Option<SharedSampleEntry>` / `fallback_video_sample_entry: Option<SharedSampleEntry>` フィールドを削除する
- コンストラクタの `fallback_*_sample_entry: None` 初期化を削除する
- writer 入口の `match crate::sample_entry::resolve_*_sample_entry(...)` ブロック（合計 8 サイト）を削除し、引数のフレーム（`Arc<AudioFrame>` / `Arc<VideoFrame>` / `&AudioFrame` / `&VideoFrame`）をそのまま下流に渡す形に置き換える
- 16 個の `tracing::warn!` "encoded-frame invariant violated" は match ブロック削除と同時に消える
- 削除対象シンボル（`resolve_*_sample_entry` / `SampleEntryResolution`）への参照が消えることで未使用になる `use` 文（特に `use crate::sample_entry::...` 系）を各 writer ファイルから整理する（`cargo clippy --all-targets -- --deny warnings` で警告にならないこと）

置き換えの方針（writer 種別ごと）:

- `Mp4Writer` / `HybridMp4Writer`: `Arc<AudioFrame>` / `Arc<VideoFrame>` を受け取って `core.handle_input_sample` に流すパスから match ブロックと `Arc::new(patched)` 再 wrap を削除する。`if let Some(sample) = sample && self.core.input_audio_track_id.is_some()` の Option 化（`Patched` / `Skip` 由来）は消え、`if self.core.input_audio_track_id.is_some()` の単純ガードのみが残る
- `DashWriter` / `HlsWriter`: `&AudioFrame` / `&VideoFrame` を受け取って append するパスから、`let patched_holder; let frame = match { Pass => frame, Patched(v) => { patched_holder = v; &patched_holder }, Skip => return Ok(()) };` の delayed-initialized ローカルと shadow ブロックを完全に削除する。`Skip` パスの早期 return も消える

writer 後続処理は `Option<SharedSampleEntry>` 型のままで触らない（`if let Some(ref entry) = frame.sample_entry` パターン等は現状維持）。受信側 `add_received_audio_data` / `add_received_video_data` 計上、`total_input_*_frame_count.inc()` 計上、`handle_*_frame` のシグネチャ、Err 取扱い（上位呼び出し側で `tracing::warn!` 握りつぶし）はすべて変えない。

### 2. テストの削除と残すべきテスト

`src/mp4/hybrid_writer.rs` の `mod tests` から削除する 8 件:

- `hybrid_writer_falls_back_on_missing_sample_entry_audio`
- `hybrid_writer_falls_back_on_missing_sample_entry_video`
- `hybrid_writer_skips_first_frame_when_missing_sample_entry_audio`
- `hybrid_writer_skips_first_frame_when_missing_sample_entry_video`
- `hybrid_writer_resolves_sample_entry_even_when_audio_track_id_is_disabled`
- `hybrid_writer_resolves_sample_entry_even_when_video_track_id_is_disabled`
- `hybrid_writer_preserves_fallback_across_consecutive_violations_audio`
- `hybrid_writer_preserves_fallback_across_consecutive_violations_video`

削除対象 8 件 **以外** はすべて残す（参考として全列挙）:

- `hybrid_writer_finalizes_readable_streams_with_per_frame_sample_entry`
- `hybrid_writer_finalizes_readable_streams_across_fragments`
- `hybrid_writer_consumes_audio_queue_before_waiting_for_video`
- `hybrid_writer_does_not_duplicate_initial_pending_audio_sample`
- `hybrid_writer_recovery_guarantee_stops_at_last_flushed_fragment`
- `hybrid_writer_disables_initial_recovery_path_after_first_flush`
- `hybrid_writer_does_not_double_update_recovery_moov_after_flush`
- `hybrid_writer_fragment_duration_uses_wall_clock_span`

共通ヘルパ `make_audio_frame`（`src/mp4/hybrid_writer.rs:1124`）/ `make_video_frame`（`src/mp4/hybrid_writer.rs:1135`）は上記の残すべきテスト群で利用されているため **残す**。`sample_entry: Option<SampleEntry>` 引数を取る形のままで OK。

`pbt/tests/prop_sample_entry.rs`:

- ファイル全件削除（PBT 8 件 + ヘルパ群）
- 削除根拠: 本 PBT は `resolve_*_sample_entry` の挙動を検証しており、関数自体が削除されるため対応する PBT も削除する。`SharedSampleEntry` の `changed_since` / `ptr_eq` 等の不変条件検証は `src/sample_entry.rs` の単体テスト（`changed_since_*` 4 件 + `ptr_eq_*` 2 件）でカバー継続される

`pbt/Cargo.toml`:

- `shiguredo_mp4` dev-dependency は `pbt/tests/prop_h264_sps.rs:10` で `use shiguredo_mp4::boxes::SampleEntry;` として使用継続のため **削除しない**
- 用途コメント（現状: `# sample_entry PBT で SampleEntry / UnknownBox を直接構築するため`）を `prop_h264_sps` の現状の利用に合わせて書き換える（例: `# prop_h264_sps PBT で SampleEntry / UnknownBox を直接構築するため`）

### 3. `maybe_flush_initial_pending` のガード処理とコメント書き換え

`src/mp4/hybrid_writer.rs::HybridMp4Writer::maybe_flush_initial_pending` の `&& let Some(ref sample_entry) = pending.sample_entry` ガード（`src/mp4/hybrid_writer.rs:403` / `:419`）は **残す**。

理由:

- このガードは「pending → リカバリ用 moov 先行更新」のベストエフォート経路で、fallback 補完経路の有無に依存しない独立した設計動機（未確定 pending を単にスキップする方針）を持つ
- 入力側不変条件で pending.sample_entry は常に Some になるが、`if let Some` パターンを維持することで「fallback 削除と同時に unwrap / panic 化までやると変更点が多くなり回帰リスクが上がる」のを避け、保守的に動作を保つ

コメント書き換え:

`src/mp4/hybrid_writer.rs:398-401` のコメント文言は現在「writer 入口の fallback で sample_entry が補完済み」を前提にしているため、以下のように「入力側不変条件で常に Some」を前提にした文言へ書き換える（参考案）:

```
// この経路はベストエフォートのリカバリで、pending の sample_entry が未確定なら単にスキップする。
// 入力側不変条件で圧縮フレームの sample_entry は常に Some になるためここに来る pending は
// 通常常に Some だが、`if let Some` パターンを残して「未確定 pending は単にスキップ」の
// ベストエフォート方針を独立した動機で保つ。
```

### 4. docs/internals/ に不変条件を明文化

新規ドキュメント `docs/internals/sample_entry_invariant.md` を作成する。これは本 issue の「fallback を writer 側で持たない判断」の根拠を、コード外に文書として明示するもの。shiguredo-doc 規約に従う。

`docs/internals/README.md` の目次にも追記する（既存 8 項目の末尾に `[\`sample_entry\` 不変条件と入力経路の責務](sample_entry_invariant.md)` を 1 行追加）。

ドキュメントに必須で記載する内容:

- 不変条件の定義: 「圧縮フォーマット（`format.codec_name().is_some()`）のフレームの `sample_entry` は必ず `Some` であること」
- 適用範囲: 入力側全経路
  - リーダー: `src/mp4/reader.rs`、`src/webm/reader.rs`、`src/rtsp/subscriber.rs`、`src/srt/inbound_endpoint.rs`、`src/sora/recording_mp4_reader.rs`
  - エンコーダ: `src/encoder/openh264.rs`、`src/encoder/svt_av1.rs`、`src/encoder/libvpx.rs`、`src/encoder/video_toolbox.rs`、`src/encoder/nvcodec.rs`、`src/encoder/fdk_aac.rs`、`src/encoder/audio_toolbox.rs`、`src/encoder/opus.rs`
- 生フォーマット（`format.codec_name().is_none()`）は対象外
- 新規入力経路追加時のチェックリスト（どの段階で sample_entry を確立すべきか、確立できない場合のテスト戦略）

`AudioFrame.sample_entry`（`src/audio.rs:88-92`）と `VideoFrame.sample_entry`（`src/video.rs:52-56`）の docstring は、既存の不変条件記述はそのまま残し、末尾にドキュメントへの参照を追記する。rustdoc 上で死にリンクにならないよう、Markdown のリンク記法（`[..](..)`）は使わず、パス文字列のまま参照する形にする:

```
/// 詳細は `docs/internals/sample_entry_invariant.md` を参照する。
```

### CHANGES.md

本 issue では記載しない。本 issue で削除する fallback コードは issue 0034 で develop に追加されたもので未リリースであり、shiguredo-changelog の「派生元ブランチとの最終的な差分のみを記載すること」「開発ブランチ内の中間状態の修正は記載しないこと」に従う（最終 diff として現れない）。

`docs/internals/` の新規ドキュメント追加も shiguredo-changelog の「`.rst` / `.md` ファイルの変更は変更履歴に反映しないこと」に従い記載しない。

## 完了条件

- 設計方針 1〜4 の削除・追加・書き換えが実装されていること
- 以下のシンボル名・文字列が `rg ... src/ pbt/ tests/ e2e-tests/ docs/` で検索しても結果が空であること:
  - `resolve_audio_sample_entry` / `resolve_video_sample_entry`
  - `SampleEntryResolution`
  - `fallback_audio_sample_entry` / `fallback_video_sample_entry`
  - 削除対象のテスト名プレフィックス（`hybrid_writer_falls_back_on_missing_sample_entry` / `hybrid_writer_skips_first_frame_when_missing_sample_entry` / `hybrid_writer_resolves_sample_entry_even_when` / `hybrid_writer_preserves_fallback_across_consecutive_violations` / `resolve_audio_` / `resolve_video_`）
  - `encoded-frame invariant violated`
- `pbt/Cargo.toml` の `shiguredo_mp4` dev-dependency が残り、用途コメントが現状の利用（`prop_h264_sps`）に合わせて書き換えられていること
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `fdk-aac` / `nvcodec` / `video_toolbox` を含む）
- 既存 e2e テスト（`e2e-tests/obsws/test_output.py` 等の HLS / DASH / MP4 / SRT 関連）が通ること
- `docs/internals/sample_entry_invariant.md` が追加され、`docs/internals/README.md` の目次にも追記されていること
- `AudioFrame.sample_entry` / `VideoFrame.sample_entry` の docstring から `docs/internals/sample_entry_invariant.md` への参照（パス文字列。リンクではない）が追記されていること
- `HybridMp4Writer::maybe_flush_initial_pending` の `&& let Some(...)` ガードが残っており、コメント文言が「入力側不変条件で常に Some」を前提にした内容に書き換えられていること
- `SharedSampleEntry::ptr_eq` の docstring から fallback 文脈の言及 2 行のみが除かれ、`changed_since` 短絡経路観測用 API としての記述は維持されていること

## 関連

- closed/0039（調査 issue。本 issue の根拠）
- closed/0034（fallback 補完経路を導入。本 issue で削除する）
- closed/0030 / closed/0031 / closed/0032 / closed/0033（入力側リーダー経路の不変条件確立）
- closed/0017 / closed/0027（エンコーダ経路の不変条件確立）
- closed/0040（`docs/internals/` 配下に規約ドキュメントを追加する直近の先例）

## 解決方法

実装着手後にここに記述する。
