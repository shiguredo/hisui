# Video Toolbox H.264 の大解像度エラーを扱う

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/fix-videotoolbox-h264-large-resolution
- Polished:

## 目的

Apple Video Toolbox の H.264 エンコーダーで 960x960 を超える解像度を指定するとエンコードに失敗する (H.265 では問題ない)。内部的な制約に起因するため hisui 側では根本対応が難しいが、外部仕様として公開されているならそれに基づくバリデーションを入れるのが親切。

## pending とした理由

- Apple Video Toolbox の内部制約であり、公開仕様の有無と検証方法の調査が必要。
- 制約が公開されていない場合は、エラー時に分かりやすいメッセージを返す以上の対応が取れない可能性がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/encoder/video_toolbox.rs` の `VideoToolboxEncoder` は hisui 側で解像度の上下限や偶数チェックを行わない (偶数の担保は他の層)。
- 依存クレート `shiguredo_video_toolbox` の `validate_video_dimensions_for_toolbox` は 0 と `i32::MAX` 超のみを検証する。
- H.264 のプロファイルは `default_video_toolbox_h264_encode_config` で Baseline / Cavlc が既定になっている。

## 設計方針 (要検討・未確定)

- Video Toolbox H.264 の解像度制約を調査する。
- 判明した制約を `shiguredo_video_toolbox` または hisui 側のバリデーションに反映する。
- 制約が不明な場合は、失敗時に解像度起因であることが分かるエラーを返す。

## 完了条件

- 失敗する解像度を事前に検出できる、または制約が明確になっていること。

(解決方法は pending のため、調査結果と設計確定後に記載する。)
