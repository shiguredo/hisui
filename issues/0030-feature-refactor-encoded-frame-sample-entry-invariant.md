# エンコード済みフレームは常に sample_entry を持つ不変条件を全経路に適用する

- Priority: Low
- Created: 2026-06-09
- Completed:
- Model: Claude Opus 4.8
- Branch: feature/refactor-encoded-frame-sample-entry-invariant
- Polished:

## 目的

issue 0017（音声）・0027（映像）でエンコーダ出力は全出力フレームに sample_entry を載せるようになった。一方でファイル / MP4 リーダー経路は今も「初回フレームと sample entry 変化時のみ」載せる疎な実装のままで、エンコード済みフレームの sample_entry の扱いがエンコーダ経路とリーダー経路で非対称になっている。

この非対称を解消し、「**エンコード済みフレーム（圧縮フォーマットの `VideoFrame` / `AudioFrame`）は常に sample_entry を持つ**」という不変条件を全経路に適用する。あわせてこの不変条件をフレーム構造体に明記し、不変条件の成立を前提に HLS / DASH writer の sample_entry 保持・補完ロジック（リーダーのような疎な入力に対する防御として置かれていたもの）を削除する。

これはバグ修正ではなく一貫性のためのリファクタである（優先度根拠参照）。

## 優先度根拠

Low。現状で機能的なバグは無い。HLS / DASH writer の consumer は obsws coordinator のみで、その上流は常にエンコーダ（全フレーム付与済み）であるため、writer の保持・補完ロジックは既に事実上デッドであり、リーダー経路の疎な付与が実害を生む経路は現時点で存在しない。本 issue は (1) 不変条件を全経路で真にして文書化し、(2) デッド化している防御ロジックを削除する、保守性のための仕上げである。時間があるときに対応する。

便益はエンコード済みフレームの sample_entry 付与ポリシーが全経路で揃い、フレーム構造体のコメントで保証される一貫性のみで、機能的改善は無い。コストはリーダー 3 ファイル + ネットワーク入力の監査・改修、writer 2 ファイルの保持ロジック削除とテスト追従。

## 現状

確認済みの事実:

- エンコーダ経路は全フレーム付与済み:
  - 映像 5 種: issue 0027（svt_av1 / libvpx / nvcodec / video_toolbox / openh264）
  - 音声 opus: issue 0017
  - 音声 AAC: `src/encoder/fdk_aac.rs:83` が全出力フレームに `Some(self.sample_entry.clone())` を載せる
- リーダー経路は疎（上流 sample が `Some` のとき = 初回・変化時のみ載る）:
  - `src/mp4/sample_reader.rs:109`（音声）/ `:138`（映像）— `sample.sample_entry.map(SharedSampleEntry::new)`
  - `src/mp4/reader.rs:1096` / `:1128` / `:1189` / `:1224` — `context.sample_entry.map(SharedSampleEntry::new)`
  - `src/sora/recording_mp4_reader.rs:151`（映像）/ `:297`（音声）— `sample_entry.map(SharedSampleEntry::new)`（`sample_entry` は `sample.sample_entry.cloned()`）
- HLS / DASH writer の保持・補完ロジックは現状デッド:
  - consumer は obsws coordinator のみ（`src/obsws/coordinator/output_hls.rs:975` / `output_dash.rs:960`）。上流は常に `encoder::create_video_processor*` / `create_audio_processor` → writer で、パススルー経路は無い
  - `src/dash/writer.rs`: `last_video_sample_entry` / `last_audio_sample_entry`（`:256-259`）、`.or()` フォールバック（`:528` / `:593`）、`fill_missing_sample_entries`（定義 `:1012`、呼び出し `:647`）
  - `src/hls/writer.rs`: `MpegTsState`（`:323-326`）と `Fmp4State`（`:340-343`）の `last_*_sample_entry`、`.or()` フォールバック（`:608` / `:700`）、`fill_missing_sample_entries`（定義 `:1171`、呼び出し `:776`）。MpegTs は映像で SPS/PPS 注入（`convert_length_prefixed_to_annexb`、`:570`）、音声で ADTS ヘッダ生成（`wrap_raw_aac_in_adts`、`:669`）に保持値を使う

要監査（実装時に実コードで確認する。現時点で疎 / 全のいずれかは未確認なので推測で書かない）:

- ネットワーク入力がエンコード済みフレームを構築する経路の sample_entry 付与:
  - `src/rtmp/frame.rs`（内部に `video_sample_entry` フィールド `:168` を保持しているため、既に毎フレーム載せている可能性がある。要確認）
  - `src/rtsp/subscriber.rs` / `src/srt/inbound_endpoint.rs` / `src/webrtc/p2p_session.rs`

## 設計方針

### 1. 不変条件をフレーム構造体に明記

`VideoFrame.sample_entry`（`src/video.rs`）と `AudioFrame.sample_entry`（`src/audio.rs`）に、「エンコード済みフレーム（圧縮フォーマット）は常に `Some`、生フレーム（I420 / PCM 等の未圧縮）は `None`」という不変条件をコメントで明記する。

`Option` のままにする理由は維持する（生フレームが構造的に `None` を持つため。非 Option 化は issue 0028 で実装不可と判断済み）。コメントだけ先行させて実態が伴わない状態（broken window）を作らないため、設計方針 2 のリーダー対応と同一コミットで入れる。

### 2. リーダー経路を全フレーム付与に変更

リーダー内で直近の sample_entry を保持し、エンコード済みフレームを生成するたびに毎フレーム載せる（エンコーダ 3 種・writer が 0017 / 0027 で採った「保持して毎フレーム clone」と同方式。`SharedSampleEntry` は Arc なので clone は安価）。

- `src/mp4/sample_reader.rs`（音声・映像）
- `src/mp4/reader.rs`（4 箇所）
- `src/sora/recording_mp4_reader.rs`（音声・映像）
- 要監査のネットワーク入力（rtmp / rtsp / srt / webrtc）で疎な箇所があれば同様に対応する

### 3. HLS / DASH writer の保持・補完ロジック削除

不変条件成立後はエンコード済みフレームに必ず sample_entry が載るため、writer 側の保持・補完は不要になる。以下を削除する:

- `dash/writer.rs` / `hls/writer.rs` の `last_video_sample_entry` / `last_audio_sample_entry` フィールド
- `.or(state.last_*_sample_entry...)` フォールバック（`frame.sample_entry` を直接使う）
- `fill_missing_sample_entries` 関数とその呼び出し
- HLS MpegTs の SPS/PPS 注入（`:570`）・ADTS ヘッダ生成（`:669`）は、保持フィールドではなく `frame.sample_entry` を直接参照するよう書き換える

削除後は writer の正しさが不変条件に依存する。不変条件違反のフレームが届いた場合は muxer がセグメント先頭サンプルでエラーになる。違反を早期検知するため `debug_assert!` か明示的なエラーを置くかは実装時に判断する。

## 完了条件

- `VideoFrame` / `AudioFrame` の `sample_entry` に不変条件のコメントが入ること
- リーダー経路（mp4 readers）がエンコード済みフレームに毎フレーム sample_entry を載せること。ネットワーク入力経路も監査し、疎な箇所が無いこと
- HLS / DASH writer の `last_*_sample_entry`・`.or()` フォールバック・`fill_missing_sample_entries` が削除され、ビルド・テストが通ること
- 録画・HLS / DASH 出力にリグレッションが無いこと
- テスト: リーダーがエンコード済みフレームに毎フレーム sample_entry を載せることを単体 / 統合テストで検証する。writer のテストは保持ロジック前提のものを削除 / 更新する
- CHANGES.md: 利用者から見た挙動は不変の内部リファクタ。記載要否は実装時に判断する（0017 / 0027 と同方針）

## 関連

- issue 0027（映像エンコーダの全フレーム付与・`VideoFrame.sample_entry` の `SharedSampleEntry` 化。本 issue の前提。writer の同一箇所を触るため、**本 issue は 0027 マージ後に着手する**）
- issue 0017（音声の全フレーム付与と共通型 `SharedSampleEntry` 導入）
- issue 0028（`sample_entry` 非 Option 化。生フレームが `None` を持つため実装せず close。本 issue の不変条件が「常に Some」ではなく「エンコード済み ⟹ Some」である理由）
