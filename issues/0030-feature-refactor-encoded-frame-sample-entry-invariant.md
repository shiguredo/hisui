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

Low。HLS / DASH writer の consumer は obsws coordinator のみで、その上流は常にエンコーダ（全フレーム付与済み）であるため、writer の保持・補完ロジックは事実上デッドであり、mp4 reader 経路の疎な付与が実害を生む経路は現時点で存在しない。

ただし、`H264AnnexB` 形式のネットワーク入力（rtsp 映像・srt 映像）を直接 mp4 出力に繋ぐ構成（デバッグ用録画など）は、現状 sample_entry が `None` のまま writer に届くため mp4 fragment の先頭サンプルで壊れる。この経路は機能バグを含む。

本 issue は (1) 不変条件を全経路で真にして文書化し、(2) デッド化している防御ロジックを削除し、(3) Annex-B 入力経路のバグを解消する、保守性と一貫性のための改修である。

便益はエンコード済みフレームの sample_entry 付与ポリシーが全経路で揃いフレーム構造体のコメントで保証されること、および Annex-B 直接 mp4 出力が動作すること。コストはリーダー 3 ファイル + ネットワーク入力 4 経路の改修、writer 3 ファイルの保持ロジック削除とテスト追従。

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
  - `src/mp4/hybrid_writer.rs`: `last_audio_sample_entry` / `last_video_sample_entry`（`:80-81`）、`.or_else()` フォールバック（`:218` / `:257`）— 同様に削除対象
- ネットワーク入力経路の確認済み現状:
  - `src/rtmp/frame.rs`: 映像（H264/AVCC）・音声（AAC）とも毎フレーム `Some` を付与。**対応不要**
  - `src/rtsp/subscriber.rs`: 映像（H264AnnexB）は `sample_entry: None`。音声（AAC）は初回のみ（`sent_sample_entry` フラグで疎）
  - `src/srt/inbound_endpoint.rs`: 映像（H264AnnexB）は `sample_entry: None`（コメント「Annex-B 入力では sample_entry は付与しない」）。音声（AAC）は config 変化時のみ（疎）
  - `src/webrtc/p2p_session.rs`: 映像は I420 生フレーム。**対象外**
  - Annex-B 映像（rtsp/srt）を直接 mp4 writer（`hybrid_writer.rs`）に繋ぐと、sample_entry が `None` のまま届くため mp4 fragment 先頭サンプルで壊れる

## 設計方針

### 1. 不変条件をフレーム構造体に明記

`VideoFrame.sample_entry`（`src/video.rs`）と `AudioFrame.sample_entry`（`src/audio.rs`）に、「エンコード済みフレーム（圧縮フォーマット）は常に `Some`、生フレーム（I420 / PCM 等の未圧縮）は `None`」という不変条件をコメントで明記する。

`Option` のままにする理由は維持する（生フレームが構造的に `None` を持つため。非 Option 化は issue 0028 で実装不可と判断済み）。コメントだけ先行させて実態が伴わない状態（broken window）を作らないため、設計方針 2 のリーダー対応と同一コミットで入れる。

### 2. リーダー経路・ネットワーク入力経路を全フレーム付与に変更

リーダー内で直近の sample_entry を保持し、エンコード済みフレームを生成するたびに毎フレーム載せる（エンコーダ 3 種・writer が 0017 / 0027 で採った「保持して毎フレーム clone」と同方式。`SharedSampleEntry` は Arc なので clone は安価）。

- `src/mp4/sample_reader.rs`（音声・映像）
- `src/mp4/reader.rs`（4 箇所）
- `src/sora/recording_mp4_reader.rs`（音声・映像）
- `src/rtsp/subscriber.rs`: 音声を毎フレーム付与に変更（`sent_sample_entry` フラグ削除）。映像は下記 Annex-B 対応で解決
- `src/srt/inbound_endpoint.rs`: 音声を毎フレーム付与に変更（config 変化時のみ → 保持して毎フレーム）。映像は下記 Annex-B 対応で解決

**Annex-B 映像（rtsp / srt）の sample_entry 構築**:

キーフレーム（SPS を含む IDR）到達時に `h264_sample_entry_from_annexb(0, 0, &frame.data)` を呼んで sample_entry を生成し保持する。その後は全フレームに保持値を clone して付与する。width / height は RTMP 実装（`src/rtmp/frame.rs`）と同様に 0 で構築する（SPS 内 Exp-Golomb パースによる解像度抽出は別 issue で対応）。

「入力側でパース vs 出力側でパース」の判断: 入力側で完全な `SampleEntry` box を構築する。SPS パース処理はエンコードと比べて極めて軽量なため、責務の明確さを優先して入力側に一本化する。これは RTMP が既に採っている方式と一致する。

### 3. HLS / DASH writer の保持・補完ロジック削除

不変条件成立後はエンコード済みフレームに必ず sample_entry が載るため、writer 側の保持・補完は不要になる。以下を削除する:

- `dash/writer.rs` / `hls/writer.rs` の `last_video_sample_entry` / `last_audio_sample_entry` フィールド
- `.or(state.last_*_sample_entry...)` フォールバック（`frame.sample_entry` を直接使う）
- `fill_missing_sample_entries` 関数とその呼び出し
- HLS MpegTs の SPS/PPS 注入（`:570`）・ADTS ヘッダ生成（`:669`）は、保持フィールドではなく `frame.sample_entry` を直接参照するよう書き換える

削除後は writer の正しさが不変条件に依存する。不変条件違反のフレームが届いた場合は muxer がセグメント先頭サンプルでエラーになる。違反を早期検知するため `debug_assert!` か明示的なエラーを置くかは実装時に判断する。

`src/mp4/hybrid_writer.rs` も同様の保持ロジックを持つため削除対象に含める。

## 完了条件

- `VideoFrame` / `AudioFrame` の `sample_entry` に不変条件のコメントが入ること
- リーダー経路（mp4 readers）がエンコード済みフレームに毎フレーム sample_entry を載せること。ネットワーク入力経路も監査し、疎な箇所が無いこと
- HLS / DASH writer の `last_*_sample_entry`・`.or()` フォールバック・`fill_missing_sample_entries` が削除され、ビルド・テストが通ること
- 録画・HLS / DASH 出力にリグレッションが無いこと
- テスト: リーダーがエンコード済みフレームに毎フレーム sample_entry を載せることを単体 / 統合テストで検証する。Annex-B 入力（rtsp/srt 映像）がキーフレームから sample_entry を構築し毎フレーム付与することを単体テストで検証する。writer のテストは保持ロジック前提のものを削除 / 更新する
- CHANGES.md: 利用者から見た挙動は不変の内部リファクタ。記載要否は実装時に判断する（0017 / 0027 と同方針）

## 関連

- issue 0027（映像エンコーダの全フレーム付与・`VideoFrame.sample_entry` の `SharedSampleEntry` 化。本 issue の前提。writer の同一箇所を触るため、**本 issue は 0027 マージ後に着手する**）
- issue 0017（音声の全フレーム付与と共通型 `SharedSampleEntry` 導入）
- issue 0028（`sample_entry` 非 Option 化。生フレームが `None` を持つため実装せず close。本 issue の不変条件が「常に Some」ではなく「エンコード済み ⟹ Some」である理由）
