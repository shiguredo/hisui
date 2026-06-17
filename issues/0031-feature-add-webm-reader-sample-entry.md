# WebM リーダーに sample_entry 構築を追加して全フレーム付与に揃える

- Priority: Low
- Created: 2026-06-10
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/add-webm-reader-sample-entry
- Polished: 2026-06-17

## カテゴリ判定

ブランチ `feature/add-webm-reader-sample-entry`（`add` カテゴリ）。WebM リーダー 2 構造体に Track メタデータ解析と sample_entry 構築機能を新規追加し、全出力フレームに `Some(SharedSampleEntry)` を載せる。0027 / 0030 由来の保持・補完ロジック側の変更は本 issue では伴わず、リファクタや破壊的変更との混在は無い。完了条件「不変条件 docstring の例外記述削除」は本 issue の主目的に対する不可分の整合作業で、別カテゴリではない。

## 目的

issue 0030 で確立した不変条件「下流に流れる圧縮 frame (`VideoFrame` / `AudioFrame`) は常に `Some(SharedSampleEntry)` を持つ」を WebM リーダー経路にも拡張する。現状 WebM リーダーは音声 Opus（`src/webm/reader.rs:399`）/ 映像（`:573`）の出力フレームで `sample_entry: None` 固定で、不変条件 docstring（`src/audio.rs:87-93` / `src/video.rs:51-57`）に「現時点で未適用の経路: WebM リーダー。」と例外として明記されている。本 issue でこの例外記述を削除する。

## 優先度根拠

Low。WebM リーダーの consumer は Sora 録画 compose 経路（`src/sora/recording_reader.rs:121` / `:358` 等）と inspect 経路（`src/webm/file_reader.rs`）。compose の下流は必ずデコーダ + エンコーダを通る配線で、出力フレームの `sample_entry` は再エンコード時に確定する。さらに 0034 で writer 入口に導入された `resolve_audio_sample_entry` / `resolve_video_sample_entry`（`src/sample_entry.rs:107-161`）が不変条件違反を検知して fallback 補完するため、現状の `sample_entry: None` が writer 不整合を起こす経路は二重防護で実害化していない。本 issue の動機は実害ゼロの broken window 解消であり、不変条件の境界を「全経路」と言い切れる状態にする。

## 現状

WebM リーダーは `src/webm/reader.rs` に `WebmAudioReader`（音声 Opus 専用）と `WebmVideoReader`（映像専用）を持つ。両者ともエンコード済みフレームを出力するが、現状すべて `sample_entry: None` で生成する。

- 音声 Opus 出力箇所: `src/webm/reader.rs:393-402`（`AudioFrame` 構築。`sample_entry: None` 固定、`channels` / `sample_rate` は Hisui 固定値 `Channels::STEREO` / `SampleRate::HZ_48000`）
- 映像出力箇所: `src/webm/reader.rs:564-574`（`VideoFrame` 構築。`sample_entry: None` / `size: None`）

WebM Track ヘッダの解析状況:

- `WebmAudioReader::new`（`:325-348`）は `skip_until(ID_INFO)` → `check_info_element` → `skip_until(ID_CLUSTER)` の順で進み、TRACKS を読まずに通過する
- `WebmVideoReader::new`（`:459-482`）は INFO の後に `VideoTrackHeader::read`（`:273-310`）を呼ぶが、これは `ID_CODEC_ID` のみ読み、`CodecPrivate` / `PixelWidth` / `PixelHeight` / Video / Audio 子要素は読み飛ばす設計

sample_entry 構築に必要なメタデータ（width / height / pre_skip）が現状リーダー内で取得できていない。

`src/sora/recording_reader.rs` は連続ファイル切り替え時に `WebmAudioReader::new` / `WebmVideoReader::new` を新規構築する設計（`:166-174` / `:404-410`）で、`inherit_stats_from` で統計 5 フィールド（`codec` / `total_cluster_count` / `total_simple_block_count` / `total_track_duration` / `track_duration_offset`）のみ引き継ぐ。

`src/audio.rs:92` / `src/video.rs:56` の不変条件 docstring は 0032 / 0033 のマージ後に整理済みで、「現時点で未適用の経路: WebM リーダー。」の 1 行のみ残る（grep で確認済み）。

## 対応スコープ

Sora の WebM 録画は VP8 / VP9 / Opus を出力するため、本 issue ではこの 3 codec を対象とする。Sora の WebM 録画で出力されない以下 2 codec は本 issue 範囲では sample_entry 構築を実装せず、`WebmVideoReader::new` の sample_entry 構築段階で `Err` を返してサポート対象外とする（`VideoTrackHeader::read` の 4 codec CodecID マッピング自体は変更せず、`new` の中の `match self.header.codec` で AV1 / H264AnnexB のみ `Err` 分岐させる）:

- `V_AV1`（`VideoFormat::Av1`）: WebM CodecPrivate（AV1CodecConfigurationRecord）の `configOBUs` 抽出と `av1_sample_entry(EvenUsize, EvenUsize, &[u8])` への変換が必要になるため、サポートする場合は別 issue で扱う
- `V_MPEG4/ISO/AVC`（`VideoFormat::H264AnnexB`）: WebM CodecPrivate は AVCDecoderConfigurationRecord（avcC）形式で、`h264_sample_entry_from_annexb`（Annex-B 入力）は流用不可（型ミスマッチ）。サポートする場合は別 issue で扱う

`testdata/archive-black-silent.webm`（VP8 + Opus）が既存テストフィクスチャとして使える。

## 設計方針

### 1. WebM Track メタデータ解析の拡張

#### 新規 EBML ID 定数（4 個）

`src/webm/reader.rs` 冒頭の ID 定義群（`:14-34`）に以下を追加する。値は EBML vint 形式で、既存の `ElementReader::read_id`（`:208-232`）の規約（vint バイトをそのまま u32 に詰める）に適合する:

- `ID_CODEC_PRIVATE: u32 = 0x63A2`（2 バイト vint。既存 `ID_MUXING_APP = 0x4D80`（`:18`）と同形式）
- `ID_VIDEO: u32 = 0xE0`（1 バイト vint）
- `ID_PIXEL_WIDTH: u32 = 0xB0`（1 バイト vint）
- `ID_PIXEL_HEIGHT: u32 = 0xBA`（1 バイト vint）

#### `WebmAudioReader::new` の TRACKS 読み込み

現状 INFO の後に TRACKS を読み飛ばす流れを以下に変更する。中間構造体は新設せず、`WebmAudioReader::new` 内で直接処理する:

1. `skip_until(ID_INFO)` → `check_info_element`（既存通り）
2. `skip_until(ID_TRACKS)` で TRACKS まで進む
3. `read_master(ID_TRACKS)` で TRACKS master を開き、`while !reader.is_eos()` で TRACK_ENTRY を順次走査
4. 各 TRACK_ENTRY 内で `read_u64(ID_TRACK_NUMBER)` → `TRACK_NUMBER_AUDIO`（= 2）でなければ `skip_all` で次へ
5. 対象 TRACK_ENTRY 内で `while !reader.is_eos()` の `peek_id` ループ（`peek_id` は内部キャッシュに ID を保持し、後続の `read_bytes` の `expect_id` が同じ ID をそのまま消費する）:
   - `peek_id == ID_CODEC_ID` → `read_bytes(ID_CODEC_ID)` で値を読み取り、`A_OPUS` のみ受理（それ以外は `Err`）
   - `peek_id == ID_CODEC_PRIVATE` → `read_bytes(ID_CODEC_PRIVATE)` で値を読み取り、`parse_opus_head_pre_skip`（新設）に渡し pre_skip を取得
   - その他の ID → `read_id` + `skip_element_data` で読み捨て
6. CodecID と pre_skip の両方が取得できなければ `Err`（音声 track が無い・OpusHead が壊れている）
7. 対象 TRACK_ENTRY 処理後、外側の TRACKS master ループの `is_eos` まで残りの TRACK_ENTRY を step 4 経由（非 audio → `skip_all`）で消化する。`read_master(ID_TRACKS)` で得た TRACKS master 自体は、外側ループ終了時点で完全消費されている
8. `skip_until(ID_CLUSTER)` で CLUSTER まで進む（既存通り）

`SamplingFrequency` / `Channels` / `OutputGain` は本 issue では読まない（Sora WebM Opus は 48kHz / Stereo 固定で、既存 `sample_entry_audio_fields()` と `Channels::STEREO` / `SampleRate::HZ_48000` の固定値で `DopsBox` を埋める。`ElementReader` への Float 読み込みメソッド追加も不要）。

`WebmFileReader` 経由の inspect で「音声トラックを持たない WebM ファイルに `audio_track_id` を指定する」と、現状の TRACKS 素通り設計では `read_simple_block` が 0 件返して正常終了するが、本変更後は `WebmAudioReader::new` 段階で `Err` を返す挙動になる（互換性のない挙動変化）。これは「音声を要求したのに無い」状態を明示的に通知する形になるため許容する。

#### `VideoTrackHeader::read` の拡張

CODEC_ID 取得後、TRACK_ENTRY 内の残り子要素を `while !reader.is_eos()` の `peek_id` ループで走査する（`skip_until(ID_VIDEO)` は使わない。VIDEO マスター不在時に EOF まで読み進めて `Err` 化し、後述の 0 フォールバックに到達できないため）:

- `peek_id == ID_VIDEO` → `read_master(ID_VIDEO)` で開き、その中の `while !inner.is_eos()` ループで `ID_PIXEL_WIDTH` / `ID_PIXEL_HEIGHT` を `read_u64` で取得（VP8 / VP9 では実値が入る）。他の Video 子要素は `read_id` + `skip_element_data` で読み捨て
- その他の ID（CODEC_ID は既読、FlagLacing 等が来る可能性） → `read_id` + `skip_element_data` で読み捨て

VIDEO マスターが存在しない / VIDEO 内に PixelWidth / PixelHeight が無い場合は width / height = 0 でフォールバックして警告ログを出す（VP8 / VP9 の `vp8_sample_entry` / `vp9_sample_entry` は profile / level / bit_depth に Hisui 固定値を入れる設計で、width / height = 0 でも sample_entry は組み立て可能）。

「映像トラックが存在しない」既存フォールバック経路（`:277-284`、`VideoFormat::I420` を返す）は維持する。この経路では sample_entry を構築せず `WebmVideoReader.sample_entry` を `None` のまま返す。`I420` は `codec_name() == None` の生フォーマットで、不変条件（圧縮 frame のみ対象）の対象外。`read_simple_block` は track_number 不一致で `None` を返すため、構造的に圧縮 `VideoFrame` を出力しない。

### 2. SampleEntry 構築

Sora 録画スコープの 3 codec に既存ヘルパを流用する。現状 private なヘルパ関数を `pub(crate)` 化し、Opus は名前を一意化する:

- 音声 Opus: `src/encoder/opus.rs:68` の `fn sample_entry(pre_skip: u16) -> SampleEntry` を `opus_sample_entry` にリネームしてから `pub(crate)` 化（webm 側から `crate::encoder::opus::opus_sample_entry` で呼び出す際に意図を明示できる）
- 映像 VP8: `src/encoder/libvpx.rs:153` の `fn vp8_sample_entry(width: usize, height: usize) -> SampleEntry` を `pub(crate)` 化（profile / level / bit_depth は関数内部の Hisui 固定値）
- 映像 VP9: `src/encoder/libvpx.rs:174` の `fn vp9_sample_entry(width: usize, height: usize) -> SampleEntry` を `pub(crate)` 化（同上）

VP8 / VP9 側は既に `vp8_sample_entry` / `vp9_sample_entry` の codec 接頭辞付き命名なのでリネーム不要。

#### OpusHead パース（`parse_opus_head_pre_skip` 新設）

WebM Opus の CodecPrivate は RFC 7845 §5.1 の OpusHead 形式（固定オフセット）。

| オフセット | サイズ | 内容 |
|---|---|---|
| 0-7 | 8 | `b"OpusHead"` マジック |
| 8 | 1 | Version（= 1） |
| 9 | 1 | OutputChannelCount |
| 10-11 | 2 | Pre-Skip（little-endian u16） |
| 12-15 | 4 | InputSampleRate（LE u32、参考値） |
| 16-17 | 2 | OutputGain（LE i16） |
| 18 | 1 | ChannelMappingFamily |
| 19+ | 可変 | ChannelMappingTable（ChannelMappingFamily != 0 のときのみ） |

`parse_opus_head_pre_skip(&[u8]) -> crate::Result<u16>` は pre_skip のみ抽出する。Sora は ChannelMappingFamily = 0（stereo 標準）を出力するため ChannelMappingTable は無く、CodecPrivate は 19 バイトに収まる（`ElementReader::read_bytes` の 1024 バイト上限（`:176`）内）。CodecPrivate が 19 バイト未満、または `OpusHead` マジック不一致の場合は `Err` を返す。

### 3. 全フレーム付与

`WebmAudioReader` / `WebmVideoReader` に private な `sample_entry: Option<SharedSampleEntry>` フィールドを追加し、コンストラクタで Track メタデータ解析結果から構築して `Some` で確定する。`read_simple_block` で `AudioFrame` / `VideoFrame` 構築時に `self.sample_entry.clone()`（Arc clone）を載せる:

- 音声: `:399` の `sample_entry: None` を `sample_entry: self.sample_entry.clone()` に変更
- 映像: `:573` の `sample_entry: None` を `sample_entry: self.sample_entry.clone()` に変更

`Option` 表現を採る理由は `VideoTrackHeader::read` の「映像トラックなし」フォールバック経路（`:277-284`）で sample_entry が構築できないため。`read_simple_block` がこの経路で呼ばれることは構造的に発生せず、不変条件は維持される。

### 4. `inherit_stats_from` の扱い

`src/sora/recording_reader.rs` のファイル切り替えは新規 `WebmAudioReader::new` / `WebmVideoReader::new` で再生成するため、新フィールド `sample_entry` はコンストラクタで自動的に再構築される。`inherit_stats_from`（音声 `:358-364` / 映像 `:492-498`）の継承対象には追加しない（既存 5 フィールドのまま）。

同一クライアントの連続録画では Sora エンコーダの初期化パラメータ（lookahead）が一定で、OpusHead の pre_skip 値もファイル間で変わらない前提。`SharedSampleEntry` の `Arc` 同一性はファイル境界で失われるが、中身の `SampleEntry` 実体が等しいため `changed_since`（`src/sample_entry.rs:54-66`）は `PartialEq` 比較で false を返し、writer 側 muxer の dedup（0017 の muxer 契約に従う）と整合する。

## 影響範囲確認（着手前 grep）

```
# WebM リーダー本体の None 固定
rg -n 'sample_entry: None' src/webm/reader.rs

# WebM リーダー consumer
rg -nc 'WebmAudioReader|WebmVideoReader' src/

# 不変条件 docstring の WebM 経路言及
rg -n '現時点で未適用の経路' src/audio.rs src/video.rs

# 流用関数の可視性（対象の opus / libvpx のみに限定）
rg -n 'fn (sample_entry|vp8_sample_entry|vp9_sample_entry|opus_sample_entry)\(' src/encoder/opus.rs src/encoder/libvpx.rs
```

着手時点で期待される hit:

- `sample_entry: None`: 2 件（音声 `:399` / 映像 `:573`）
- `WebmAudioReader|WebmVideoReader`: 計 18 件（`src/webm/reader.rs` 6 件 + `src/webm/file_reader.rs` 5 件 + `src/sora/recording_reader.rs` 7 件）
- `現時点で未適用の経路`: 2 件（`src/audio.rs:92` / `src/video.rs:56` 各 1 行）
- 流用関数（上記 grep）: 3 件、すべて `fn`（private）。`fn sample_entry`（opus）/ `fn vp8_sample_entry` / `fn vp9_sample_entry`

完了時点で期待される hit:

- `sample_entry: None`: 0 件
- `WebmAudioReader|WebmVideoReader`: 18 件（consumer は不変、リネームなし）
- `現時点で未適用の経路`: 0 件
- 流用関数（上記 grep）: 3 件、すべて `pub(crate) fn`。`fn opus_sample_entry` / `fn vp8_sample_entry` / `fn vp9_sample_entry`

## 実装スコープ（変更対象ファイル）

1. `src/webm/reader.rs`: 設計方針 1-3。新規 EBML ID 定数 4 個追加、`WebmAudioReader::new` の TRACKS 解析、`VideoTrackHeader::read` 拡張、両 reader に `sample_entry: Option<SharedSampleEntry>` フィールド追加、`WebmVideoReader::new` での AV1 / H264AnnexB `Err` 分岐、`read_simple_block` の `None` を `self.sample_entry.clone()` に置換、`parse_opus_head_pre_skip` 新設
2. `src/encoder/opus.rs`: `fn sample_entry` を `opus_sample_entry` にリネームし `pub(crate)` 化
3. `src/encoder/libvpx.rs`: `fn vp8_sample_entry` / `fn vp9_sample_entry` の `pub(crate)` 化
4. `src/audio.rs` / `src/video.rs`: 不変条件 docstring の「現時点で未適用の経路: WebM リーダー。」1 行削除（着手時に HEAD で grep して文面が変わっていないことを再確認）
5. `tests/reader_webm_tests.rs`: 既存 VP8 + Opus テストに sample_entry 検証アサート追加
6. `src/webm/reader.rs::#[cfg(test)] mod tests`（新設）: OpusHead パース・全フレーム付与不変条件・対応スコープ外 codec の Err 検証

## テスト

CLAUDE.md のテスト役割分担に従う。本リポジトリは PBT 基盤がない（0017 / 0027 / 0030 と同方針）ため、検証は既存テスト機構で行う。

### 既存テスト更新（`tests/reader_webm_tests.rs`）

`testdata/archive-black-silent.webm`（VP8 + Opus）を使う既存 2 テスト（`webm_audio_reader_test` / `webm_video_reader_test`）に以下のアサートを追加:

- 全 `AudioFrame` / `VideoFrame` が `sample_entry.is_some()` を満たす
- 後続フレームが初回フレームと同一 `Arc`（`SharedSampleEntry::ptr_eq` で短絡経路を直接検証）
- 初回フレームの `sample_entry.get()` の中身（`SampleEntry::Opus` の `pre_skip` / `SampleEntry::Vp08` の `visual.width` / `visual.height`）が testdata の OpusHead / PixelWidth / PixelHeight と一致（具体値は実装時に testdata を読んで確定する）

### 新規単体テスト（`src/webm/reader.rs` の `#[cfg(test)] mod tests`）

実フィクスチャ（バイト列定数）を組み立てて検証する:

- `opus_head_parser_returns_err_on_too_short_codec_private`: 19 バイト未満入力 → `Err`
- `opus_head_parser_returns_err_on_magic_mismatch`: 先頭 8 バイトが `OpusHead` でない → `Err`
- `opus_head_parser_extracts_pre_skip_in_little_endian`: 正常 OpusHead（pre_skip = 0x1234 等）→ 取得値が指定値と一致
- `webm_audio_reader_constructs_sample_entry_from_opus_head`: 最小 EBML/Segment/Info/Tracks/Cluster フィクスチャ → `WebmAudioReader::new` で `sample_entry` フィールドが `Some` になり、内部の `SampleEntry::Opus` の `pre_skip` が OpusHead の値と一致
- `webm_video_reader_constructs_sample_entry_for_vp8`: VP8 用最小フィクスチャ → `sample_entry` が `Some` で、`Vp08Box` の `visual.width` / `visual.height` が PixelWidth / PixelHeight と一致
- `webm_video_reader_constructs_sample_entry_for_vp9`: VP9 用最小フィクスチャ → 同上 `Vp09Box`
- `webm_video_reader_returns_err_for_av1`: `V_AV1` の CodecID で `WebmVideoReader::new` → `Err`
- `webm_video_reader_returns_err_for_h264_annexb`: `V_MPEG4/ISO/AVC` の CodecID で `WebmVideoReader::new` → `Err`
- `webm_video_reader_constructs_sample_entry_with_zero_when_pixel_dimensions_missing`: VIDEO マスター不在 → width / height = 0 で sample_entry を構築（`Err` にはしない）
- `webm_video_reader_keeps_sample_entry_arc_identity_across_frames`: 同一 reader で複数 SimpleBlock を読み、後続フレームが初回フレームと同一 Arc（`ptr_eq` で短絡）
- `webm_video_reader_releases_new_arc_per_construction`: 同じ testdata で 2 回 `WebmVideoReader::new` を呼ぶと、それぞれの `sample_entry.get()` は `PartialEq` で等しいが `ptr_eq` では別 Arc（ファイル切り替え時の挙動を再現）

### 統合テスト

`tests/reader_webm_tests.rs` の更新で実 testdata 経由の検証を兼ねる。compose サブコマンドの e2e は本 issue では新規追加しない。

## エッジケース

- 音声トラック不在の WebM: `WebmAudioReader::new` で `Err`（既存「0 件で正常終了」から挙動変化、許容）
- 映像トラック不在の WebM: 既存挙動維持（`VideoFormat::I420` フォールバック + `sample_entry: None`）。`read_simple_block` で圧縮 `VideoFrame` を生成しないため不変条件は維持
- PixelWidth / PixelHeight 欠落: width / height = 0 で sample_entry 構築
- CodecID が AV1 / H264AnnexB: `WebmVideoReader::new` で `Err`
- CodecID が `A_OPUS` 以外（例: `A_VORBIS`）: `WebmAudioReader::new` で `Err`
- CodecPrivate が 19 バイト未満 / `OpusHead` マジック不一致: `WebmAudioReader::new` で `Err`（現状は TRACKS 素通りのため起動成功するが、新設計では起動失敗。壊れた録画ファイルの互換性が低下するが、Sora 録画では発生しない異常系で許容）

## 完了条件

- `WebmAudioReader` / `WebmVideoReader` の出力 `AudioFrame` / `VideoFrame` がすべて `Some(SharedSampleEntry)` を持つこと（`VideoFormat::I420` フォールバック経路は `read_simple_block` から圧縮フレームを生成しないため不変条件の対象外）
- `src/audio.rs` / `src/video.rs` の不変条件 docstring から「現時点で未適用の経路: WebM リーダー。」の 1 行を削除すること（着手時に HEAD で grep して文面が変わっていないことを再確認）
- 既存テスト（`tests/reader_webm_tests.rs` の VP8 + Opus 2 テスト）に sample_entry 検証アサートを追加して通ること
- 新規単体テスト 11 件（OpusHead パース 3 件・VP8 / VP9 / 寸法不在 / 対応スコープ外 codec / Arc 同一性 / ファイル切り替え再構築 8 件）が通ること
- compose サブコマンドでの Sora 録画合成にリグレッションが無いこと（既存 e2e の WebM ソース利用テスト全通過）
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `fdk-aac` / `nvcodec` / `video_toolbox` を含む）

### CHANGES.md

記載しない（内部リファクタ・公開 API 変化なし・利用者挙動変化なし）。0017 / 0027 / 0030 / 0033 と同方針。

## 関連

- issue 0030（直接の前提）
- issue 0017（音声側の `SharedSampleEntry` 共通型導入。間接的な前提）
- issue 0027（映像エンコーダの全フレーム付与とフレーム構造体の `SharedSampleEntry` 化）
- issue 0034（writer 入口の不変条件違反検知）
