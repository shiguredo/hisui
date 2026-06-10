# RTSP subscriber に sprop-parameter-sets 解析を追加して Annex-B 映像 sample_entry を構築する

- Priority: Low
- Created: 2026-06-10
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/add-rtsp-annexb-video-sample-entry
- Polished:

## 目的

issue 0030 で「エンコード済みフレーム（圧縮フォーマットの `VideoFrame` / `AudioFrame`）は常に sample_entry を持つ」という不変条件を mp4 リーダー / rtsp / srt 音声入力経路に適用したが、RTSP 経路の Annex-B 映像はスコープから外れている。本 issue では RTSP の H.264 Annex-B 映像入力に対して、SDP の fmtp 行から `sprop-parameter-sets` を抽出し初期 sample_entry を構築する経路と、IDR 内 inline SPS / PPS から構築する経路を追加し、全映像フレームに sample_entry を付与する。

## 優先度根拠

Low。現状の obsws 配線では subscriber 出力は必ず `VideoDecoder`（`src/rtsp/subscriber.rs:102-114` で `output_video_track_id.is_some()` 時に `VideoDecoder` を強制生成）を経由して I420 raw track へ流れるため、`sample_entry: None` のフレームがそのまま writer に到達する経路は実在しない。よって機能バグではなく、issue 0030 で確立した不変条件「下流に流れる圧縮 frame は常に Some」を Annex-B 経路にも拡張するための予防的整備として実施する。

## 現状

`src/rtsp/subscriber.rs:639` で映像フレームは `sample_entry: None` 固定で生成される（depacketizer 出力をそのまま `VideoFrame` に詰める）。SDP の `sprop-parameter-sets`（RFC 6184 §6.2 で規定される H.264 fmtp パラメータ）は、現状の `select_video_track`（`:1264-1288`）でパースされていない（payload type と clock rate のみ抽出）。`find_fmtp` / `parse_fmtp_parameters` ヘルパは既存だが映像経路では未使用（音声経路の `select_audio_track`（`:1309-1311`）でのみ使用）。

`VideoTrackConfig` 構造体（`:244-249`）と `VideoRtpReceiver` 構造体（`:300-306`）に sample_entry を保持するフィールドは存在しない。

## 設計方針

### 1. SDP `sprop-parameter-sets` パース経路の新設

`VideoTrackConfig`（`:244-249`）に `sample_entry: Option<SampleEntry>` フィールドを追加する。`select_video_track`（`:1264-1288`）で以下を行う:

- 既存の `find_fmtp` / `parse_fmtp_parameters` を H.264 映像にも適用
- `sprop-parameter-sets` パラメータ値を comma 区切りで分割
- 各要素を Base64 デコード（既存依存の `base64ct` 等を利用可否は実装着手時に確認）
- デコード結果（生 SPS / PPS NAL ユニット）を Annex-B 形式（`0x00 0x00 0x00 0x01` プレフィックス）で連結
- `h264_sample_entry_from_annexb(0, 0, &annexb)`（`src/video/h264.rs:87`）に渡して `SampleEntry` を構築
- 構築結果を `VideoTrackConfig.sample_entry` に格納

width / height は RTMP 実装と同様に 0 で構築する。SPS 内 Exp-Golomb パースによる解像度抽出は別 issue 扱い。

### 2. depacketizer 出力での全フレーム付与

`VideoRtpReceiver`（`:300-306`）に `last_sample_entry: Option<SharedSampleEntry>` フィールドを追加する。`setup_session`（`:448-459`）で `VideoTrackConfig.sample_entry` を `SharedSampleEntry::new(...)` でラップして初期化する。

depacketizer 出力フレーム（`:633-640`）に対して以下を行う:

- depacketizer 出力が SPS / PPS を inline で含む場合は、`h264_sample_entry_from_annexb` を呼び直して `last_sample_entry` を更新する（mid-stream で SPS / PPS が変わるケースに追従。`openh264` エンコーダの実装と同方針）
- 全映像フレームに `sample_entry: Some(self.last_sample_entry.clone())` を載せる（`:639` の `sample_entry: None` を置き換え）

### 3. SPS / PPS 取得経路の優先順位とフォールバック

- (a) SDP `sprop-parameter-sets` を優先する。SDP に存在する場合は初期 frame から sample_entry が確定する
- (b) depacketizer 出力に SPS / PPS が inline で含まれる場合は更新する（mid-stream の SPS / PPS 変化に追従）
- (c) SDP にも IDR にも SPS / PPS が無い場合は `tracing::warn!` で警告し、当該 IDR を破棄して後続の SPS 含有フレームを待つ。`last_sample_entry` が `None` の間はフレームを下流に流さない（既存の `received_video_keyframe` 相当のゲートを併設するか、`last_sample_entry.is_some()` をゲートとして使う）

## 完了条件

- RTSP の Annex-B 映像出力フレームが全て `Some(SharedSampleEntry)` を持つこと
- SDP に `sprop-parameter-sets` がある場合は初期フレームから sample_entry が確定すること
- IDR 内 inline SPS / PPS でも sample_entry が更新できること
- SPS / PPS が両経路とも取得できない間はフレームを下流に流さず、警告ログが出ること
- 既存 RTSP テスト（`src/rtsp/subscriber.rs` の `#[cfg(test)] mod tests`）が通ること
- 新規単体テストで以下を検証する: (a) SDP `sprop-parameter-sets` 由来の初期 sample_entry が全フレームに付与される、(b) IDR inline SPS / PPS で sample_entry が更新される、(c) SPS / PPS 不在 IDR は破棄される
- issue 0030 で適用範囲を限定した不変条件のコメント（`VideoFrame.sample_entry`）が RTSP Annex-B 経路も含めて成立すること

## 関連

- issue 0030（直接の前提。本 issue は 0030 で確立した不変条件を RTSP Annex-B 映像経路にも拡張する）
- issue 0033（SRT inbound endpoint の Annex-B 映像 sample_entry 構築。本 issue と並行・独立で進める）
