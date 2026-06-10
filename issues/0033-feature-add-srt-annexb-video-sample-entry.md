# SRT inbound endpoint で Annex-B 映像から SPS/PPS を抽出して sample_entry を構築する

- Priority: Low
- Created: 2026-06-10
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/add-srt-annexb-video-sample-entry
- Polished:

## 目的

issue 0030 で「エンコード済みフレーム（圧縮フォーマットの `VideoFrame` / `AudioFrame`）は常に sample_entry を持つ」という不変条件を mp4 リーダー / rtsp / srt 音声入力経路に適用したが、SRT 経路の Annex-B 映像はスコープから外れている。本 issue では SRT MPEG-TS 入力の H.264 Annex-B 映像に対して、IDR フレーム内 SPS / PPS を抽出して sample_entry を構築し、全映像フレームに付与する。

## 優先度根拠

Low。現状の obsws 配線では subscriber 出力は必ず `VideoDecoder`（`src/srt/inbound_endpoint.rs:166-178` で `output_video_track_id.is_some()` 時に強制生成）を経由して I420 raw track へ流れるため、`sample_entry: None` のフレームがそのまま writer に到達する経路は実在しない。よって機能バグではなく、issue 0030 で確立した不変条件「下流に流れる圧縮 frame は常に Some」を Annex-B 経路にも拡張するための予防的整備として実施する。

## 現状

`src/srt/inbound_endpoint.rs:935` で映像フレームは `sample_entry: None` 固定で生成される（コメント「Annex-B 入力では sample_entry は付与しない」）。

`build_video_sample`（`:890-937`）は `H264AnnexBNalUnits::new(&pending.data)` を IDR 判定のためにのみ使用しており（`:912-918` で `nalu.ty == H264_NALU_TYPE_IDR` で `break`）、SPS / PPS の抽出は行わない。`SrtTsDemuxer` 構造体（`:720-`）に sample_entry を保持するフィールドは存在しない。

既存の `received_video_keyframe` ゲート（`:920-925`）は「初回 keyframe（IDR）到達まで全フレームを破棄」する仕組みで、IDR を見たら以後 keyframe / 非 keyframe を区別なく流す。

## 設計方針

### 1. SrtTsDemuxer への sample_entry 保持フィールド追加

`SrtTsDemuxer` 構造体（`:720-`）に `last_video_sample_entry: Option<SharedSampleEntry>` フィールドを追加する。コンストラクタ（`:732-754`）で `None` に初期化する。

### 2. IDR 内 SPS / PPS の抽出

`build_video_sample`（`:890-937`）の `H264AnnexBNalUnits::new(&pending.data)` ループを拡張し、SPS（`H264_NALU_TYPE_SPS`、type 7）と PPS（`H264_NALU_TYPE_PPS`、type 8）を蓄積する。IDR を検出した時点で蓄積した SPS / PPS が揃っていれば、`h264_sample_entry_from_annexb(0, 0, &pending.data)`（`src/video/h264.rs:87`）を呼んで sample_entry を構築し、`self.last_video_sample_entry` を `Some(SharedSampleEntry::new(...))` で更新する。

width / height は RTMP 実装と同様に 0 で構築する。SPS 内 Exp-Golomb パースによる解像度抽出は別 issue 扱い。

### 3. ゲート再構築

既存の `received_video_keyframe` ゲートは「sample_entry が確定したか」と判定基準が異なるため、以下のいずれかで再構築する:

- (a) `received_video_keyframe` を「SPS / PPS 含有 IDR を受信したか」に意味変更する（IDR フラグは別途追跡）
- (b) `received_video_keyframe` は維持しつつ、新規に `last_video_sample_entry.is_some()` をゲートに併用する

実装着手時にどちらを採るかは整合性で判断する。いずれの場合も「sample_entry が確定するまで全フレームを破棄」する挙動を保つ。

### 4. SPS / PPS 不在 IDR の扱い

SPS / PPS を含まない IDR が来た場合は `tracing::warn!` で警告を出し、当該 IDR を破棄して後続の SPS 含有フレームを待つ。`last_video_sample_entry` は更新しない。

MPEG-TS PMT からの SPS / PPS 抽出は本 issue では行わない（IDR の inline のみを対象とする）。

### 5. 全フレーム付与

`build_video_sample` の `TsSample::Video(crate::VideoFrame { ... })` 構築箇所（`:929-936`）で、`sample_entry: None` を `sample_entry: Some(self.last_video_sample_entry.as_ref().expect("invariant").clone())` 相当に置き換える。ゲートで「sample_entry 未確定のフレームは下流に流さない」を保証しているため、ここでは `Some` が確定している前提で良い。

### 6. mid-stream の SPS / PPS 更新追従

新しい IDR が SPS / PPS を含む場合は、`h264_sample_entry_from_annexb` を呼び直して `last_video_sample_entry` を更新する。これは `openh264` エンコーダの実装と同方針（途中で SPS / PPS が変わる可能性に対応する）。

## 完了条件

- SRT の Annex-B 映像出力フレームが全て `Some(SharedSampleEntry)` を持つこと
- SPS / PPS 含有 IDR を受信した時点で sample_entry が確定し、以後の全 P フレームにも付与されること
- SPS / PPS 不在 IDR が来た場合は警告ログを出し、当該 IDR を破棄して後続を待つこと
- mid-stream で SPS / PPS が更新された場合は sample_entry が更新されること
- 既存 SRT テスト（`src/srt/inbound_endpoint.rs` の `#[cfg(test)] mod tests`）が通ること
- 新規単体テストで以下を検証する: (a) SPS / PPS 含有 IDR で sample_entry が確定して以後の P フレームに付与される、(b) SPS / PPS 不在 IDR が破棄される、(c) mid-stream SPS / PPS 更新で sample_entry が差し替わる
- issue 0030 で適用範囲を限定した不変条件のコメント（`VideoFrame.sample_entry`）が SRT Annex-B 経路も含めて成立すること

## 関連

- issue 0030（直接の前提。本 issue は 0030 で確立した不変条件を SRT Annex-B 映像経路にも拡張する）
- issue 0032（RTSP の同等対応。並行・独立で進める）
