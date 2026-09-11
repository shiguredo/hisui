# キーフレーム間隔を共通設定で指定可能にする

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/add-keyframe-interval-setting
- Polished:

## 目的

キーフレーム間隔は一般的に指定したくなる項目だが、現状はエンコーダー固有の設定でのみ指定できる。共通設定として指定でき、既定値も決める。

## pending とした理由

- 共通設定のインターフェース (CLI フラグ / output settings のどの階層に置くか) の設計が必要。
- 一部のエンコーダー (libvpx / SVT-AV1) は現状の共通上書き関数の対象外で、どう反映するかを決める必要がある。
- 既定値 (現在 300 フレーム) の妥当性も合わせて検討する。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/encoder.rs` の `encode_config_with_keyframe_interval` は openh264 の `intra_period`、VideoToolbox の `max_key_frame_interval` (と duration)、NVENC の `idr_period` を上書きする。
- libvpx の `keyframe_interval` と SVT-AV1 の `intra_period_length` は対象外で既定 300 のまま。
- 全エンコーダーの既定は 300 フレーム (`EncodeConfig::default` と各 `default_*_encode_config`)。
- 共通上書き関数の利用箇所は HLS / DASH coordinator のみで、`(segment_duration * fps).ceil()` により間接的に算出している。
- ユーザー向けの共通設定 (CLI フラグ / output settings の `keyframe_interval`) は存在しない。
- `request_upstream_video_keyframe` / `VideoEncoderRpcMessage::RequestKeyframe` による動的なキーフレーム要求はある。

## 設計方針 (要検討・未確定)

- 共通設定を追加し、全エンコーダーに反映する。
- HLS / DASH の `segment_duration` 由来の間接指定との優先順位を決める。
- 既定値を決定する。

## 完了条件

- 共通設定でキーフレーム間隔を指定できること。
- 未指定時に既定値が適用されること。

(解決方法は pending のため、設計確定後に記載する。)
