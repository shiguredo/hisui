# 映像合成で NV12 を扱えるようにすることを検討する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-nv12-compositing
- Polished:

## 目的

現在 hisui は内部の生データを I420 前提で扱っているが、NVIDIA 系は NV12 前提のため、エンコーダー / デコーダーとの境界で毎回フォーマット変換が入る。内部で NV12 のまま合成できれば、余計な変換を省いて高速化できる可能性がある。

## pending とした理由

- 入出力で複数の画像フォーマットが混在した場合の考慮が必要で、コードが複雑になる。
- 合成で使うフォーマットをレイアウト側で指定できるようにするか、候補から選択するロジックを決める必要がある。
- 内部で NV12 にしても実際に高速になるかは不明瞭で、リサイズ・合成が I420 の方が速ければ全体コストは下がらない可能性がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/video.rs` の `VideoFormat` は `I420` / `I420A` のみで、`NV12` は無い。`RawVideoFrame::from_video_frame` と `as_i420_planes` は I420 / I420A 以外をエラーにする。
- 合成は `src/mixer/video.rs` の `VideoRealtimeMixer` が行い、キャンバスは `RealtimeI420Canvas`。テキストオーバーレイは raden と `shiguredo_libyuv::argb_to_i420_alpha` で I420A を作る。
- NV12 は境界にのみ現れ、`src/encoder/nvcodec.rs` が `i420_to_nv12`、`src/decoder/nvcodec.rs` が `convert_nv12_to_i420` で変換する。`src/obsws/source/video_device.rs` の `convert_captured_frame_to_i420` も同様。

## 設計方針 (要検討・未確定)

- 内部で扱うフォーマットを選択可能にする (複数フォーマットの混在を考慮する)。
- 合成・リサイズを NV12 のまま行えるか検証し、I420 と比較して計測する。
- 入力と出力でフォーマットが異なる場合の変換コストを見積もる。

## 完了条件

- NV12 での合成が I420 と比べて有利かどうかを計測し、採用方針を決めること。

(解決方法は pending のため、設計確定後に記載する。)
