# エンコード済みフレームは常に sample_entry を持つ不変条件をリーダー / 音声入力経路に適用し writer 補完を削除する

- Priority: Low
- Created: 2026-06-09
- Completed: 2026-06-12
- Model: Claude Opus 4.8
- Branch: feature/refactor-encoded-frame-sample-entry-invariant
- Polished: 2026-06-10

## 目的

issue 0017（音声）・0027（映像）でエンコーダ出力は全出力フレームに sample_entry を載せるようになった。一方でファイル / MP4 リーダー経路は今も「初回フレームと sample entry 変化時のみ」載せる疎な実装、rtsp / srt の AAC 音声入力経路も同様に疎な実装のままで、エンコード済みフレームの sample_entry の扱いがエンコーダ経路と他経路で非対称になっている。

この非対称をリーダー / AAC 音声入力経路で解消し、「**エンコード済みフレーム（圧縮フォーマットの `VideoFrame` / `AudioFrame`）は常に sample_entry を持つ**」という不変条件をこれら経路に適用する。あわせてこの不変条件をフレーム構造体に明記し、不変条件の成立を前提に HLS / DASH writer・hybrid_writer の sample_entry 補完ロジックを削除する。

本 issue のスコープは「mp4 リーダー経路 + rtsp / srt の AAC 音声入力経路 + writer の補完ロジック削除」に限定する。対象外経路は「現状」節で列挙する。

## 優先度根拠

Low。本 issue は機能バグ修正ではなく、不変条件の明文化と writer のデッドロジック削除という保守性のためのリファクタ。

issue 0028（非 Option 化）は「規模に対して便益が著しく不釣り合い」で close したが、本 issue は (a) 改修範囲が限定的（リーダー 3 ファイル + ネットワーク入力 2 ファイル + writer 4 ファイル）、(b) 不変条件の明文化と writer のデッドロジック削除という具体的便益、(c) 後続別 issue による段階的拡張の足場、の 3 点で実施に値する。

## 現状

エンコーダ経路は全フレーム付与済み:

- 映像 5 種: issue 0027（svt_av1 / libvpx / nvcodec / video_toolbox / openh264）
- 音声 opus: issue 0017（`src/encoder/opus.rs:60`）
- 音声 AAC: `src/encoder/fdk_aac.rs:83` と `src/encoder/audio_toolbox.rs:177`（macOS）が全出力フレームに `Some(self.sample_entry.clone())` を載せる

本 issue で対応するリーダー / ネットワーク入力経路は疎:

- `src/mp4/sample_reader.rs:109`（音声）/ `:138`（映像）— `sample.sample_entry.map(SharedSampleEntry::new)`
- `src/mp4/reader.rs:1096` / `:1128` / `:1189` / `:1224` — `context.sample_entry.map(SharedSampleEntry::new)`
- `src/sora/recording_mp4_reader.rs:151`（映像）/ `:297`（音声）— `sample_entry.map(SharedSampleEntry::new)`
- `src/rtsp/subscriber.rs`: 音声（AAC）は `sent_sample_entry` フラグ（`:316`）で初回のみ送信（`:677-682`）
- `src/srt/inbound_endpoint.rs`: 音声（AAC）は `:984-995` で `last_aac_config_key`（フィールド宣言 `:728`）の変化時のみ生成

本 issue の対象外（後続別 issue で対応）:

- `src/webm/reader.rs:399`（音声 Opus）/ `:573`（映像）: `sample_entry: None` 固定 → issue 0031
- `src/rtsp/subscriber.rs:639` の Annex-B 映像: SDP `sprop-parameter-sets` パース新設が必要 → issue 0032
- `src/srt/inbound_endpoint.rs:935` の Annex-B 映像: IDR 内 SPS / PPS 抽出新設が必要 → issue 0033
- 観測 API 廃止（`pub fn total_*_sample_entry_count`、`/metrics` Prometheus メトリクス）と hybrid_writer の保持フィールド・changed_since 判定の最終削除 → issue 0034

writer の保持・補完ロジックの現状:

- consumer は obsws coordinator のみ（`src/obsws/coordinator/output_hls.rs:975` / `output_dash.rs:960`）。上流は常にエンコーダ（全フレーム付与済み）で、補完ロジックは事実上デッド
- 該当ファイル: `src/dash/writer.rs` / `src/hls/writer.rs` / `src/mp4/hybrid_writer.rs`
- `src/mp4/writer.rs` の `Mp4Writer` は補完を持たず、`frame.sample_entry.as_ref().map(|e| e.get().clone())` で muxer に直接渡している（`:756` / `:788`）。本 issue での削除対象外（不変条件下でそのまま動く）

その他のネットワーク入力経路:

- `src/rtmp/frame.rs`: 全フレーム `Some` 付与済み。対応不要
- `src/webrtc/p2p_session.rs`: 映像は I420 生フレーム（対象外）、音声は別経路で構築（対象外）

`VideoFrame.sample_entry`（`src/video.rs:51`）と `AudioFrame.sample_entry`（`src/audio.rs:87`）は `Option<SharedSampleEntry>` で、不変条件のコメントは無い。

## 設計方針

### 1. 不変条件をフレーム構造体に明記

`VideoFrame.sample_entry`（`src/video.rs:51`）と `AudioFrame.sample_entry`（`src/audio.rs:87`）に、次の不変条件をコメントで明記する。

「目指す不変条件: subscriber / reader / encoder が下流に出力する圧縮フォーマット（音声: Opus / Aac、映像: H264 / H264AnnexB / H265 / Vp8 / Vp9 / Av1）の `VideoFrame` / `AudioFrame` は常に `Some` を持つ。生フォーマット（音声: I16Be、映像: I420 / I420A）と外部に流れないフレームは `None` を許容する。」

本 issue 完了時点での適用範囲は「mp4 リーダー / rtsp / srt の AAC 音声入力 / エンコーダ出力」。コメントに「現時点で未適用の経路: WebM リーダー、rtsp / srt の Annex-B 映像」と例外を明示する。後続 issue（0031 / 0032 / 0033）の完了条件に「`VideoFrame.sample_entry` / `AudioFrame.sample_entry` のコメントから該当経路の例外記述を削除する」を含めることで、不変条件の境界記述が拡張に追従する。

`Option` のままにする理由は維持する（生フレームが構造的に `None` を持つため。非 Option 化は issue 0028 で実装不可と判断済み）。

### 2. リーダー経路・AAC 音声入力経路を全フレーム付与に変更

リーダー内で直近の sample_entry を保持し、エンコード済みフレームを生成するたびに保持値を clone して載せる。`SharedSampleEntry` は内部 Arc なので clone は安価（0017 / 0027 の実装と同方式）。

#### mp4 リーダー（3 ファイル + 8 サイト）

- `src/mp4/sample_reader.rs`（音声 :109・映像 :138）: `Mp4SampleReader::run` のローカル変数として 2 フィールド構成の保持（`last_audio_sample_entry: Option<SharedSampleEntry>` / `last_video_sample_entry: Option<SharedSampleEntry>`）を導入
- `src/mp4/reader.rs`（音声 :1096 / :1128・映像 :1189 / :1224）: `ReaderState` に 2 フィールド構成の保持を追加。warm-up 経路（`suppress_publish`）と publish 経路の両方で保持値を `sample_entry` に載せる（warm-up 経路でも decoder が sample_entry を要求するため）
- `src/sora/recording_mp4_reader.rs`（映像 :151・音声 :297）: `Mp4VideoReader` / `Mp4AudioReader` はそれぞれ単一トラック専用構造体のため、両者にそれぞれ `last_sample_entry: Option<SharedSampleEntry>` の 1 フィールドのみ追加（構造体名で映像/音声が既に表現されているため、フィールド名には冗長な修飾を付けない）

保持の更新位置（mp4 リーダー 3 ファイル全てに適用）:

- `composition_time_offset.is_some()` の早期 Err return 経路では保持更新不要（フレームは下流に流れない）
- `is_audio_enabled` / `is_video_enabled` で false のフレームは保持を更新しない（無効化された track の sample_entry を有効 track と混在させないため）
- 上記以外では保持を更新し、`VideoFrame` / `AudioFrame` 構築時に保持値を clone して `sample_entry` フィールドに載せる

#### rtsp / srt の AAC 音声経路

- `src/rtsp/subscriber.rs`:
  - `AudioRtpReceiver.sample_entry`（`:315`）を `SampleEntry` から `SharedSampleEntry` に変更
  - `AudioTrackConfig.sample_entry`（`:258`）は `SampleEntry` のまま据え置き（`select_audio_track`（`:1290-1349`）で 1 回構築する DTO 的役割）
  - `setup_session` 内 `AudioRtpReceiver` 構築（`:482-500`）の `sample_entry` フィールド代入箇所（`:498`）で `SharedSampleEntry::new(audio.sample_entry)` で構築（`audio` は `:461` の `if let Some(audio)` で move されているため clone 不要）
  - `sent_sample_entry` フラグ（`:316`）を削除
  - 全フレームで `Some(audio_receiver.sample_entry.clone())` を載せる
- `src/srt/inbound_endpoint.rs`:
  - `last_aac_config_key`（`:728`）は config 変化検知のために残す（AAC AudioSpecificConfig の変化判定に必要）
  - `last_aac_sample_entry: Option<SharedSampleEntry>` を `SrtTsDemuxer` に新規追加（`:720-` 構造体内）
  - `:984-995` の挙動を「config 変化時のみ `create_mp4a_sample_entry` で新規生成して `last_aac_config_key` / `last_aac_sample_entry` を更新、それ以外のフレームは `last_aac_sample_entry` を clone して付与」に変更

### 3. HLS / DASH writer・hybrid_writer の補完ロジック削除

不変条件成立後はエンコード済みフレームに必ず sample_entry が載るため、writer 側の補完は不要になる。以下を削除する。行番号は本 issue 起票時点。実装時は最新コードから grep で特定する。

#### `src/dash/writer.rs`

- `DashWriter` の `last_video_sample_entry` / `last_audio_sample_entry` フィールド（`:257` / `:259`）と初期化（`:355-356`）
- `handle_video_frame` 内（`:484-496` の `if let Some(ref entry) = frame.sample_entry { ... }` ブロック）と `handle_audio_frame` 内（`:553-565`）の保持代入（`:485 self.last_video_sample_entry = Some(entry.clone());` と `:554 self.last_audio_sample_entry = Some(entry.clone());`）。**同一ブロック内の codec_string 解決ロジックは残す**（不変条件下で `frame.sample_entry` を直接参照する形にリファクタする）
- 保持代入の直前にある背景説明コメント（`:482-483`、`:551-552` 相当の「入力経路によっては sample_entry が初回フレームにしか載らないため...」）も併せて削除する
- 映像サンプル構築の `.or(self.last_video_sample_entry.as_ref())` フォールバック（`:526`）と音声側（`:591`）。`frame.sample_entry` を直接使う
- `fill_missing_sample_entries` 関数（定義 `:1010-` / 引数行 `:1012-1013`、呼び出し `:645`）

#### `src/hls/writer.rs`

dash/writer.rs と構造が異なる。映像経路は `match &mut self.format_state { FormatState::MpegTs(state) => { ... }, FormatState::Fmp4(state) => { ... } }` の各 arm 内に独立した `if let Some(entry) = &frame.sample_entry { state.last_video_sample_entry = Some(entry.clone()); ... }` がある（MpegTs 映像 `:562-566`、Fmp4 映像 `:584-588`）。音声経路は外側の `if let Some(entry) = &frame.sample_entry { match &mut self.format_state { FormatState::MpegTs(state) => { state.last_audio_sample_entry = Some(entry.clone()); }, FormatState::Fmp4(state) => { state.last_audio_sample_entry = Some(entry.clone()); } } }`（`:649-658`）の単一 if 文で match 分岐する形式。codec_string 解決ブロック（映像 `:549-559`・音声 `:638-647`）はそれぞれ別の if 文で分離している。

以下を削除する。

- `MpegTsState.last_video_sample_entry` / `last_audio_sample_entry`（`:324` / `:326`）と `Fmp4State.last_video_sample_entry` / `last_audio_sample_entry`（`:341` / `:343`）と初期化（`:368-369`、`:382-383`）
- 映像経路の各 match arm 内の `if let Some(entry) = &frame.sample_entry` 保持代入（MpegTs `:562-566`、Fmp4 `:584-588`）と直前の背景説明コメント
- 音声経路の外側の `if let Some(entry) = &frame.sample_entry { match ... }` 全体（`:649-658`、保持代入の二目的のみのため）
- Fmp4 経路の `.or()` フォールバック（映像 `:606`、push `:608`、音声 `:698`、push `:700`）
- `fill_missing_sample_entries` 関数（定義 `:1169-`、呼び出し `:774`）
- MpegTs の SPS / PPS 注入（`convert_length_prefixed_to_annexb` 呼び出し `:568`、保持値参照引数 `:570`）と ADTS ヘッダ生成（`wrap_raw_aac_in_adts` 呼び出し `:667`、保持値参照引数 `:669`）を `frame.sample_entry` を直接参照する形に書き換える

codec_string 解決ブロック（映像 `:549-559`・音声 `:638-647`）は保持削除後もそのまま残す（不変条件下で `frame.sample_entry` を直接参照する形にリファクタする）。

#### `src/mp4/hybrid_writer.rs`

- `append_video_to_fragment`（`:202-`）内の `.or_else()` フォールバック（`:218`）と音声側（`:243-` の `:257`）を削除し、`frame.sample_entry` を直接使う
- `maybe_flush_initial_pending`（`:409-`）内の `.or_else()` フォールバック（映像 `:422`・音声 `:441`）を削除し、`pending.sample_entry` を直接参照する形にする。書き換え後の最終形は `if let Some(pending) = ... && let Some(ref sample_entry) = pending.sample_entry { samples.push(...) }` のようにネスト if を残す（防御的）。本 issue では writer の上流が常に `Some` を保証する想定だが、`HybridMp4Writer` の入力経路が将来変わる可能性に備えて recovery moov 先行更新の best-effort 設計を保つ
- `missing_*_sample_entry` カウンタ計上（映像 `:220`・音声 `:259`）を削除（フォールバック削除に伴い `sample_entry.is_none()` 判定が不要になるため）
- `last_audio_sample_entry` / `last_video_sample_entry` フィールド（`:80-81`）・初期化（`:175-176`）・`handle_audio_message` / `handle_video_message`（`:938-` / `:976-`）内の保持取り込み（音声 `:951-956`・映像 `:986-991`）・`add_received_*_sample_entry` 呼び出し（音声 `:953`・映像 `:988`）・`SharedSampleEntry::changed_since` 関連は **本 issue では残す**。これらは received カウンタ計上に使われており、観測 API（`pub fn` / Prometheus メトリクス）と一体で issue 0034 で破壊的変更として削除する

#### `add_missing_*_sample_entry` 削除に伴う dead_code 対応

`add_missing_*_sample_entry` 呼び出しを 0030 で削除すると、唯一の呼び出し元が消えるため `pub(crate) fn add_missing_audio_sample_entry` / `add_missing_video_sample_entry`（`src/mp4/writer.rs:229-235`）が crate 内で未使用になり、`cargo clippy --deny warnings` で dead_code 警告が出る。完了条件「clippy 警告ゼロ」を満たすため、本 issue では当該 2 メソッドに `#[allow(dead_code)]` 属性を付与する（コメントに「issue 0034 で `Mp4WriterStats` の missing 系一式と合わせて削除予定」を併記）。`total_missing_*_sample_entry_count` フィールド・`stats.counter` 初期化・struct 初期化・`pub fn` getter は `Mp4WriterStats::new` 内で参照され続けるため dead_code にならず追加対応不要。Prometheus メトリクス `hisui_total_missing_*_sample_entry_count` は 0 固定値として `/metrics` に出力され続け、利用者から見た出力形式は不変（0034 で正式廃止予定）。

#### codec_string 解決は残す

`src/dash/writer.rs`（映像 `:484-496`・音声 `:553-565`）と `src/hls/writer.rs`（映像 `:549-559`・音声 `:638-647`）の codec_string 解決ロジックは保持削除後もそのまま残す。dash 側は同一 `if let Some(ref entry) = frame.sample_entry { ... }` ブロック内で保持代入だけ消す。hls 側は codec_string ブロックと保持ブロックが別の if 文で分離しているため、codec_string ブロックは触らない。

#### 削除順序（コミット粒度の目安）

設計方針 1 / 2 / 3 を 1 PR にまとめてアトミックマージする前提で、コミットを以下の粒度に並べる:

1. 設計方針 1 + 設計方針 2（不変条件コメント追加・mp4 リーダー 3 ファイル + rtsp / srt AAC 修正 + 関連単体テスト追加）
2. 設計方針 3 の dash/writer.rs 削除
3. 設計方針 3 の hls/writer.rs 削除
4. 設計方針 3 の hybrid_writer.rs の writer 補完削除（`.or_else()` / `missing_*` カウンタ呼び出し / 関連テスト削除）

各コミット時点で `cargo check && cargo test` が通る状態を保つ。

## 完了条件

- `VideoFrame` / `AudioFrame` の `sample_entry` フィールドに不変条件のコメントが入ること（適用範囲と現時点で未適用の経路も明記）
- mp4 リーダー 3 ファイルそれぞれが直近の sample_entry を保持し、エンコード済みフレームを生成するすべてのサイト（合計 8 サイト）で毎フレーム `Some` が載ること
- rtsp / srt の AAC 音声経路が毎フレーム sample_entry を載せること
- HLS / DASH writer の補完用 `last_*_sample_entry`・`.or()` フォールバック・`fill_missing_sample_entries` が削除されていること
- hybrid_writer の `.or_else()` フォールバック（`append_*_to_fragment` と `maybe_flush_initial_pending` の双方）と `add_missing_*_sample_entry` カウンタ呼び出しが削除されていること
- hybrid_writer の `last_*_sample_entry` フィールド・`handle_*_message` 内の保持取り込み・`add_received_*_sample_entry` カウンタ呼び出しは残っていること（0034 で破壊的変更として削除予定）
- 録画・HLS / DASH 出力にリグレッションが無いこと: 既存 e2e テスト（`e2e-tests/obsws/test_output.py`）の HLS / DASH / SRT 録画関連テストが通ること
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（dead_code 警告が出ない構造に保つ）
- feature gate（`fdk-aac` / `nvcodec` / `video_toolbox`）でも上記が通ること

### テスト

- 単体テスト（リーダー）:
  - sora 録画 MP4 リーダー（`Mp4VideoReader` / `Mp4AudioReader`）: 同一 sample_entry を持つ複数 sample を読んだ際に、全 `VideoFrame` / `AudioFrame` が `Some(SharedSampleEntry)` を持ち、後続フレームが初回と等価（`SharedSampleEntry::changed_since` が false）であることを検証
  - `src/mp4/reader.rs` の `Mp4FileReader`（OBSWS 経路）と `src/mp4/sample_reader.rs` の `Mp4SampleReader`（inspect 経路）は `Mp4VideoReader` / `Mp4AudioReader` と同一の「直近 sample_entry を保持して全フレームに付与」パターンを採るため、本 issue では単体テスト追加せず既存 e2e で間接カバーとする
- 単体テスト（ネットワーク入力）:
  - srt 音声: `SrtTsDemuxer::build_audio_samples` を直接呼び出し、`last_aac_config_key` 変化時も連続時も全 AU が `Some` を持つことを検証（config 連続シナリオと stereo / mono を切り替える config 変化シナリオの 2 ケース）
  - rtsp 音声: `AudioRtpReceiver.sample_entry: SharedSampleEntry` への型変更と `Some(audio_receiver.sample_entry.clone())` の毎フレーム付与で「全 AU に Some が載る」ことが型システム・コードレベルで保証されるため、本 issue では単体テスト追加せず既存 e2e（`run_rtsp_session_*` 系）で間接カバーとする
- writer テスト削除（`src/mp4/hybrid_writer.rs` の `#[cfg(test)] mod tests`）:
  - missing カウンタ関連（`add_missing_*_sample_entry` 呼び出し削除でテスト対象が消えるため）:
    - `hybrid_writer_counts_missing_sample_entry_for_fragment_first_sample`（`:1420`）
    - `hybrid_writer_counts_finalize_failure_on_missing_sample_entry`（`:1451`）
    - `hybrid_writer_counts_missing_video_sample_entry_for_first_sample`（`:1483`）
  - `.or_else()` フォールバック関連（`append_*_to_fragment` 内のフォールバック削除でテスト対象が消えるため）:
    - `hybrid_writer_keeps_audio_sample_entry_across_fragments`（`:1137`）
    - `hybrid_writer_keeps_video_sample_entry_across_fragments`（`:1550`）
    - `hybrid_writer_captures_audio_sample_entry_at_ingress`（`:1166`）
    - `hybrid_writer_captures_video_sample_entry_at_ingress`（`:1514`）
  - `captures_*_at_ingress` は `last_*_sample_entry` フィールド代入・received カウンタ計上・finalize 成功・フォールバック補完の 4 要素を 1 テストで検証していた。フォールバック補完以外の 3 要素は以下で間接的にカバーされる:
    - フィールド代入と received カウンタ計上は `hybrid_writer_received_*_sample_entry_counts_only_changes` の `changed_since` 判定で検証される
    - finalize 成功は writer テスト更新で扱う `hybrid_writer_finalizes_readable_streams_with_per_frame_sample_entry` で検証される
- writer テスト残置（`last_*_sample_entry` フィールドと received カウンタ計上を 0030 で残すため、以下は変更不要）:
  - `hybrid_writer_received_audio_sample_entry_counts_only_changes`（`:1221`）
  - `hybrid_writer_received_video_sample_entry_counts_only_changes`（`:1287`）
- writer テスト更新:
  - `hybrid_writer_finalizes_readable_audio_with_per_frame_sample_entry`（`:1364`）は不変条件下の正常 finalize パスを守る回帰防止テストとして残す。`total_missing_audio_sample_entry_count` のアサート（`:1398-1402` 付近）と直前の解説コメントを削除（カウンタ呼び出しが消えるため）。本 issue で映像トラックの読み戻しと初回フレームとの sample_entry 等価性検証（`SharedSampleEntry::changed_since`）を追加し、テスト名を `hybrid_writer_finalizes_readable_streams_with_per_frame_sample_entry` に改名する
- 統合テスト: `HybridMp4Writer` で fMP4 セグメントを生成 → finalize → `Mp4AudioReader::new` / `Mp4VideoReader::new` で読み戻し、全フレームに `Some(SampleEntry)` が載っていることを assert する新規テストを追加する。既存 `tests/writer_mp4_tests.rs` は `Mp4Writer`（標準 MP4）専用なので、`HybridMp4Writer` 用テストファイルは新規追加か `src/mp4/hybrid_writer.rs` の `#[cfg(test)] mod tests` 内追加とする

### CHANGES.md

記載しない（内部リファクタ・公開 API 変化なし・利用者挙動変化なし）。0017 / 0027 と同方針。`Mp4WriterStats` の公開 API・`/metrics` Prometheus メトリクスは本 issue では残すため利用者から見た挙動は不変（0034 で破壊的変更として CHANGE 記載予定）。

## 関連

- issue 0027（映像エンコーダの全フレーム付与・`VideoFrame.sample_entry` の `SharedSampleEntry` 化。本 issue の直接の前提。マージ済み・closed）
- issue 0017（音声の全フレーム付与と共通型 `SharedSampleEntry` 導入。間接的な前提。closed）
- issue 0028（`sample_entry` 非 Option 化。実装不可で close。closed）
- issue 0011（録画 finalize 失敗の真因調査。`received_*_sample_entry_count` / `missing_*_sample_entry_count` カウンタの起源。closed）
- issue 0031（WebM リーダーへの sample_entry 構築追加。本 issue の後続。不変条件を WebM 経路にも拡張する）
- issue 0032（RTSP の Annex-B 映像 sample_entry 構築。本 issue の後続。不変条件を RTSP Annex-B 経路にも拡張する）
- issue 0033（SRT の Annex-B 映像 sample_entry 構築。本 issue の後続。不変条件を SRT Annex-B 経路にも拡張する）
- issue 0034（hybrid_writer の `last_*_sample_entry` フィールド・`add_received_*_sample_entry` カウンタ呼び出し・`Mp4WriterStats` 公開 API / `/metrics` メトリクスの破壊的廃止と writer 不変条件違反検知ロギングの追加。本 issue の後続）

## 解決方法

### 不変条件の明文化

- `AudioFrame.sample_entry` (`src/audio.rs`) と `VideoFrame.sample_entry` (`src/video.rs`) に不変条件コメントを追加した。圧縮フォーマット（Opus / Aac、H264 / H264AnnexB / H265 / Vp8 / Vp9 / Av1）の出力フレームは常に `Some` を持つこと、生フォーマット・decoder 内部の中間表現・外部に流れないフレームは `None` を許容することを明示した。本 issue 完了時点で未適用の経路（WebM リーダー、rtsp / srt の Annex-B 映像）は例外として明記し、後続 issue 0031 / 0032 / 0033 で順次解消する。

### リーダー / 音声入力経路の全フレーム付与

- `src/mp4/reader.rs` の `Mp4FileReader` の `ReaderState` に `last_audio_sample_entry` / `last_video_sample_entry` を追加し、warm-up（`suppress_publish`）経路と publish 経路の両方で全フレームに付与するように変更した。
- `src/mp4/sample_reader.rs` の `Mp4SampleReader::run` でローカル変数として `last_audio_sample_entry` / `last_video_sample_entry` を保持し、全フレームに付与するように変更した。
- `src/sora/recording_mp4_reader.rs` の `Mp4VideoReader` / `Mp4AudioReader` に `last_sample_entry` フィールドを追加し、全フレームに付与するように変更した（単一トラック構造体のため映像/音声を冠さない命名）。
- `src/rtsp/subscriber.rs` の `AudioRtpReceiver.sample_entry` を `SampleEntry` から `SharedSampleEntry` に変更し、`sent_sample_entry` フラグを削除して全 AAC AU に `Some(audio_receiver.sample_entry.clone())` を付与する形に変更した。
- `src/srt/inbound_endpoint.rs` の `SrtTsDemuxer` に `last_aac_sample_entry` を追加し、`last_aac_config_key` と同期更新（config 変化時に両方を Some に切り替え）することで全 AAC AU に `Some` を付与する形に変更した。

### HLS / DASH writer・hybrid_writer の補完ロジック削除

- `src/dash/writer.rs` / `src/hls/writer.rs` の `last_*_sample_entry` フィールド・`.or()` フォールバック・`fill_missing_sample_entries` を削除した。codec_string 解決ロジックは `frame.sample_entry` を直接参照する形にリファクタした。
- `src/mp4/hybrid_writer.rs` の `append_*_to_fragment` の `.or_else()` フォールバックと `add_missing_*_sample_entry` カウンタ計上を削除した。`maybe_flush_initial_pending` の `.or_else()` も削除し、`pending.sample_entry` を直接参照する形に変更（ベストエフォート設計は維持）。
- `last_*_sample_entry` フィールド・`handle_*_message` 内の保持取り込み・`add_received_*_sample_entry` カウンタ計上は残置（issue 0034 で観測 API 廃止と合わせて削除予定）。
- `src/mp4/writer.rs` の `add_missing_audio_sample_entry` / `add_missing_video_sample_entry` は呼び出し元が消えるため `#[expect(dead_code)]` で抑制した（issue 0034 で削除予定）。

### テスト

- 単体テスト（リーダー）: `src/sora/recording_mp4_reader.rs` に `mp4_video_reader_emits_sample_entry_on_every_frame` / `mp4_audio_reader_emits_sample_entry_on_every_frame` を追加。後続フレームが初回と等価（`SharedSampleEntry::changed_since` が false）であることまで検証する。`Mp4FileReader` / `Mp4SampleReader` は同一パターンのため単体テスト追加は省略し、既存 e2e で間接カバーとする。
- 単体テスト（ネットワーク入力）: `src/srt/inbound_endpoint.rs` に `srt_aac_emits_sample_entry_on_every_au_with_constant_config` / `srt_aac_updates_sample_entry_on_config_change` を追加し、`SrtTsDemuxer::build_audio_samples` を直接呼んで config 連続/変化の双方をカバーした。rtsp 音声は型システムとコードレビューレベルで保証されるため単体テスト追加せず、既存 e2e でカバーとする。
- 統合テスト: `src/mp4/hybrid_writer.rs` の既存 `hybrid_writer_finalizes_readable_audio_with_per_frame_sample_entry` を `hybrid_writer_finalizes_readable_streams_with_per_frame_sample_entry` に改名し、映像トラックの読み戻しと等価性検証を追加した。さらに `hybrid_writer_finalizes_readable_streams_across_fragments` を追加し、`flush_fragment()` を挟む 3 フラグメントを生成して全フレーム（先頭・後続を問わず）に sample_entry が載ることを検証した。
- 削除テスト: `add_missing_*_sample_entry` の呼び出しが消えたため `hybrid_writer_counts_missing_*` 系 3 件と、`.or_else()` フォールバックが消えたため `hybrid_writer_keeps_*` / `hybrid_writer_captures_*` 系 4 件を削除した。`captures_*_at_ingress` の検証要素のうちフォールバック以外の 3 要素（フィールド代入・received カウンタ計上・finalize 成功）は `hybrid_writer_received_*_counts_only_changes` と `hybrid_writer_finalizes_readable_streams_*` で間接的にカバーされる。

### レビュー指摘の反映

`/review-diff-code` で挙がった指摘を順次対応した。

- 削除済み関数 `fill_missing_sample_entries` の docstring が `fixup_last_sample_duration` に誤って付着していた残骸を `src/dash/writer.rs` / `src/hls/writer.rs` から削除。
- `src/hls/writer.rs` の `handle_audio_frame` 冒頭に残っていた「sample_entry を保持しておかないと codec 情報が失われる」旨の死んだ背景説明コメントを削除。
- `src/mp4/hybrid_writer.rs` の `last_*_sample_entry` フィールドコメント・`handle_*_message` 内の入口取り込みコメント・`maybe_flush_initial_pending` の `recovery` 英単語残存箇所、および `src/mp4/writer.rs` の `Mp4WriterStats` フィールドコメントを実態（received カウンタの `changed_since` 判定専用、issue 0034 で削除予定）に合わせて書き換え。`recovery` を「リカバリ」/「リカバリ用 moov」に日本語化。
- `src/sora/recording_mp4_reader.rs` の `next_sample` 内で発生していた `sample_entry` の二重 clone を `cloned()` の値を直接 move する形に変更。
- `src/srt/inbound_endpoint.rs` の AAC AU 処理に `last_aac_config_key` と `last_aac_sample_entry` の同期更新不変条件を明示するコメントを追加。
- `src/audio.rs` / `src/video.rs` の不変条件コメントから自己言及行（「不変条件成立後に該当 issue の完了条件でこのコメントから例外記述を削除する」）を削除。

### CHANGES.md

記載なし（内部リファクタ・公開 API 変化なし・利用者挙動変化なし）。0017 / 0027 と同方針。
