# nvcodec デコーダーが sample_entry 変化に追従せず解像度変化する H.264 / H.265 の合成結果が壊れる

- Created: 2026-08-07
- Branch: feature/fix-nvcodec-decoder-resolution-change

## 目的

Sora の録画ファイル (MP4) で、1 つの入力ファイル内で解像度が変化する H.264 / H.265 を nvcodec デコーダーで合成した際に、合成結果が壊れる問題を修正する。WebRTC のシミュキャスト / 適応ビットレート録画で顕在化する。

hotfix/2025.3.3 (main 側) で同種のバグを修正済みだが、develop 側は別のモジュール構造を持ち、同等の修正が入っていない。

## 現状

`src/decoder/nvcodec.rs` の `NvcodecDecoder::decode()` の parameter_sets (VPS / SPS / PPS) キャッシュロジックは以下:

```rust
if self.parameter_sets.is_none()
    && let Some(sample_entry) = &frame.sample_entry
{
    self.parameter_sets = Some(extract_parameter_sets_annexb(
        sample_entry.get(),
        frame.format,
    )?);
}
```

`self.parameter_sets.is_none()` により **初回のみ設定** される。解像度が変化して reader が新しい sample_entry を返しても、`parameter_sets` は最初の VPS / SPS / PPS のまま更新されない。以降の keyframe に対して古い parameter_sets が prepend され続け、デコーダーは新しい解像度と古い parameter set のミスマッチしたビットストリームを受け取ることになり、合成結果が破損する。

## 設計方針

hotfix/2025.3.3 で採用した方針と同じ。`sample_entry` が `Some` で来たフレームでは毎回 parameter_sets を取り直す:

```rust
if let Some(sample_entry) = &frame.sample_entry {
    self.parameter_sets = Some(extract_parameter_sets_annexb(
        sample_entry.get(),
        frame.format,
    )?);
}
```

reader 側 (`src/sora/recording_mp4_reader.rs`, `src/mp4/reader.rs`, `src/webm/reader.rs`) の sample_entry 供給ポリシーが「変化時のみ Some」であれば、このロジックで過剰更新にはならない。「全フレームに Some」を返す reader があれば毎フレーム再抽出になるので、必要なら前回値との比較ガードを追加する。

## 完了条件

- 解像度変化する H.264 / H.265 の Sora 録画で、nvcodec デコーダーによる合成結果が破損しなくなる
- リグレッションテスト (解像度変化する MP4 を投入して、出力フレームの解像度シーケンスを assert) を追加する

## 解決方法

- `src/decoder/nvcodec.rs` の `NvcodecDecoder::decode()` から parameter_sets 更新条件の `is_none()` を外す
- reader の sample_entry 供給ポリシーを verify し、必要なら過剰更新回避のガードを追加する
- hotfix/2025.3.3 で追加した `h264_single_track_resolution_change_nvcodec_passthrough` に相当するテストを追加する (テストデータは PR #328 で追加した `testdata/archive-h264-resolution-change.mp4` などが利用可能だが develop の testdata 構成に合わせて配置する)
