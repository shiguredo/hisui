# WebM リーダーに sample_entry 構築を追加して全フレーム付与に揃える

- Priority: Low
- Created: 2026-06-10
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/add-webm-reader-sample-entry
- Polished:

## 目的

issue 0030 で「エンコード済みフレーム（圧縮フォーマットの `VideoFrame` / `AudioFrame`）は常に sample_entry を持つ」という不変条件を mp4 リーダー / rtsp / srt 音声入力経路に適用したが、WebM リーダー経路はスコープから外れている。本 issue では WebM リーダーにも sample_entry 構築機能を追加して、不変条件を WebM 経路にも揃える。

これは broken window（不変条件を文章で謳いつつ一部経路で破られている状態）を解消するための後続作業である。

## 優先度根拠

Low。WebM リーダー（`src/webm/reader.rs`）の consumer は sora compose 経路（`src/sora/recording_reader.rs:121` 他）で最終的に `Mp4Writer` に流れるが、`recording_reader` の下流が必ずデコーダ + エンコーダを通る配線のため、`sample_entry: None` のフレームがそのまま writer に到達することは無く、実害化していない。

ただし、issue 0030 で確立した不変条件「下流に流れる圧縮 frame は常に Some」を WebM 経路に拡張することで、不変条件の境界を限定せずに「全経路」と言い切れる状態になる。配線が将来変わって WebM → writer 直結が発生した場合の予防にもなる。

## 現状

WebM リーダーは `src/webm/reader.rs` に `WebmAudioReader` と `WebmVideoReader` の 2 構造体を持つ。両者ともエンコード済みフレーム（音声 Opus・映像 VP8 / VP9 / AV1 / H264AnnexB）を出力するが、現状すべて `sample_entry: None` で生成する。

- 音声 Opus 出力箇所: `src/webm/reader.rs:399`（コメント「以降のフィールドはデコーダーには参照されないのでダミー値を設定しておく」）
- 映像出力箇所: `src/webm/reader.rs:573`

WebM Track ヘッダの解析状況:

- `WebmAudioReader::new`（`:325-348`）は TRACKS 要素を完全にスキップする（CodecID も読まず INFO → CLUSTER に直接進む）
- `WebmVideoReader::new`（`:459-482`）は `VideoTrackHeader::read`（`:280-308`）を呼ぶが、これは `ID_CODEC_ID` だけ読み、`CodecPrivate` / `PixelWidth` / `PixelHeight` / Video 子要素 / Audio 子要素は読み飛ばす設計

つまり sample_entry 構築に必要なメタデータ（width / height / pre_skip / channels / sample_rate 等）が現状リーダー内で取得できていない。

## 設計方針

### 1. WebM Track メタデータ解析の拡張

`WebmAudioReader::new` と `WebmVideoReader::new` の TRACKS 解析を拡張して、sample_entry 構築に必要な情報を読み取れるようにする。

追加で読む WebM 要素 ID:

- `ID_VIDEO = 0xE0`（Video マスター要素）
- `ID_PIXEL_WIDTH = 0xB0`
- `ID_PIXEL_HEIGHT = 0xBA`
- `ID_AUDIO = 0xE1`（Audio マスター要素）
- `ID_SAMPLING_FREQUENCY = 0xB5`
- `ID_CHANNELS = 0x9F`
- `ID_CODEC_PRIVATE = 0x63A2`

`WebmAudioReader` には新規構造体 `AudioTrackHeader::read` を追加して `WebmVideoReader::VideoTrackHeader::read` 相当の役割を持たせる。

### 2. SampleEntry 構築

各 codec ごとに既存の構築関数を流用する:

- 音声 Opus: `src/encoder/opus.rs` の `sample_entry(pre_skip)` 同等処理（pre_skip は CodecPrivate 内 OpusHead から取得、取得不能なら 0 固定）
- 映像 VP8: `src/encoder/libvpx.rs` の `vp8_sample_entry(width, height)`
- 映像 VP9: `src/encoder/libvpx.rs` の `vp9_sample_entry(width, height)`（profile / level / bit_depth は libvpx.rs 同様の固定値で代用）
- 映像 AV1: `src/video/av1.rs` の `av1_sample_entry(width, height, config_obus)`（config_obus は CodecPrivate 内 Sequence OBU から取得）
- 映像 H264AnnexB: `src/video/h264.rs` の `h264_sample_entry_from_annexb(width, height, data)`（SPS / PPS は CodecPrivate から抽出）

PixelWidth / PixelHeight が Track 情報から取得できない場合は 0 でフォールバック（RTMP 経路と同方針）。

### 3. 全フレーム付与

`WebmAudioReader::read_simple_block`（`:399`）と `WebmVideoReader::read_video_frame`（`:573`）の `sample_entry: None` を `sample_entry: Some(self.sample_entry.clone())` に変更する。`sample_entry` は `SharedSampleEntry` 型で構造体フィールドとして保持し、各リーダーのコンストラクタで一度だけ構築する。

### 4. Sora WebM 出力で扱う codec の実態調査

スコープを確定するため、Sora の WebM 録画で実際に扱われる codec を着手前に調査する。VP8 / VP9 / Opus が中心で AV1 / H264AnnexB は実用上扱わないなら、後者 2 種は `match` で `Err` を返す形にしてサポート対象から外しても良い（実装規模を削減できる）。

### 5. `inherit_stats_from` の扱い

`WebmAudioReader::inherit_stats_from`（`:358-364`）と `WebmVideoReader::inherit_stats_from`（`:492-498`）は連続ファイル切り替え時の統計引き継ぎ API。SampleEntry は各ファイル単位で新規構築し、`inherit_stats_from` の継承対象に含めない（同一クライアントの連続録画で codec が変わらない前提でも、ファイル単位での正規化を保つ）。

## 完了条件

- WebM リーダー（`WebmAudioReader` / `WebmVideoReader`）の出力フレームが全て `Some(SharedSampleEntry)` を持つこと
- 既存テスト（`testdata/archive-black-silent.webm` を使う VP8 + Opus テスト）が通ること
- 新規単体テストで Opus / VP8 / VP9 の全フレーム付与を検証する（AV1 / H264AnnexB は対応するなら同様に検証）
- compose サブコマンドでの Sora 録画合成にリグレッションが無いこと
- issue 0030 で適用範囲を限定した不変条件のコメント（`VideoFrame.sample_entry` / `AudioFrame.sample_entry`）が WebM 経路も含めて成立すること（コメントの境界記述を更新する）

## 関連

- issue 0030（直接の前提。本 issue は 0030 で確立した不変条件を WebM 経路にも拡張する）
- issue 0017（音声側の `SharedSampleEntry` 共通型導入。間接的な前提）
- issue 0027（映像エンコーダの全フレーム付与とフレーム構造体の `SharedSampleEntry` 化）
