# デコーダーの AVCC NAL パースが lengthSizeMinusOne 非対応で外部 MP4 を誤パースする

- Created: 2026-08-18
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-decoder-avcc-length-size
- Polished: {YYYY-MM-DD}

## 目的

デコーダーが受け取る AVCC 形式の H.264 / H.265 フレームを、対応トラックの avcC / hvcC の `lengthSizeMinusOne` に基づいて可変長でパースできるようにする。`lengthSizeMinusOne` が 3 以外の外部 MP4 をデコードする経路での誤パースを解消する。

## 現状

- デコーダーの AVCC パースは NAL 長フィールドを 4 バイト固定 (`NALU_HEADER_LENGTH`) で読む:
  - `src/decoder/openh264.rs` の `build_annexb_input`
  - `src/video/h264.rs` の `extract_h264_sps_pps_from_avcc` (`src/decoder/video_toolbox.rs` / `src/decoder/nvcodec.rs` が使用)
  - `src/video/h265.rs` の `extract_h265_vps_sps_pps_from_avcc` (`src/decoder/video_toolbox.rs` / `src/decoder/nvcodec.rs` が使用)
- MP4 リーダーは `src/mp4/demuxer.rs` の `video_format_from_entry` で `SampleEntry::Avc1` / `Hev1` / `Hvc1` を `VideoFormat::H264` / `H265` (AVCC 形式) としてサンプルデータをそのまま渡す
- そのため、`lengthSizeMinusOne` が 3 以外の外部 MP4 (例: inspect `--decode` で読み込む場合) では、デコーダーが NAL 長フィールドを誤って解釈する
- `src/subcommand_inspect.rs` の `VideoCodecSpecificInfo::new` (H.264 分岐) は同課題を解決済み (issue 0095) だが、デコーダー側は未対応
- デコーダーの `build_annexb_input` と `extract_h264_sps_pps_from_avcc` / `extract_h265_vps_sps_pps_from_avcc` は、いずれも `frame.sample_entry` にアクセスできる文脈で使われており、`lengthSizeMinusOne` を取得可能

## 設計方針

- デコーダーの AVCC パースを、対応トラックの avcC / hvcC の `lengthSizeMinusOne` に基づく可変長 NAL 長フィールド読み取りに変更する
- 呼び出し側 (デコーダー) は `frame.sample_entry` から長さを取得してパース関数へ渡す
- 取得パターンは `src/subcommand_inspect.rs` の `VideoCodecSpecificInfo::new` (issue 0095 で実装済み) を参考にする
- H.264 / H.265 は同一の根本問題のため、1 つの issue でまとめて対応する

## 完了条件

- `lengthSizeMinusOne` が 3 以外の MP4 をデコードしても、NAL パースが誤らないこと
- `lengthSizeMinusOne` が 3 の既存入力に回帰がないこと
- `build_annexb_input` / `extract_h264_sps_pps_from_avcc` / `extract_h265_vps_sps_pps_from_avcc` の単体テストで length_size 1〜4 の各ケースを検証すること
