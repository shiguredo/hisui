# nvcodec デコーダーが sample_entry 変化に追従せず解像度変化する H.264 / H.265 のデコード結果が壊れる

- Created: 2026-08-07
- Updated: 2026-08-13
- Branch: feature/fix-nvcodec-decoder-resolution-change

## 目的

1 つの MP4 内で解像度が変化する H.264 / H.265 (多エントリ stsd で sample_entry が切り替わるファイル) を nvcodec デコーダーでデコードした際に、デコード結果が壊れる問題を修正する。

WebRTC のシミュキャスト / 適応ビットレート録画で顕在化する。

hotfix/2025.3.3 (main 側) で同種のバグを修正済みだが、develop 側は別のモジュール構造を持ち、同等の修正が入っていない。

なお、起票当初は「Sora の録画ファイル」を前提としていたが、develop では Sora 録画機能自体が削除済み (issue 0090, PR #327) のため、本 issue は汎用の MP4 デコード経路 (`inspect --decode` / `src/mp4/sync_reader.rs` + nvcodec デコーダー) を対象とする。

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

`self.parameter_sets.is_none()` により **初回のみ設定** される。解像度が変化して reader が新しい sample_entry を返しても、`parameter_sets` は最初の VPS / SPS / PPS のまま更新されない。以降の keyframe に対して古い parameter_sets が prepend され続け、デコーダーは新しい解像度と古い parameter set のミスマッチしたビットストリームを受け取ることになり、デコード結果が破損する。

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

reader 側の sample_entry 供給ポリシーを確認する必要がある。develop の `src/mp4/sync_reader.rs` は、`last_sample_entry` を保持して **全フレームに** sample_entry を付与する (`sample_entry: self.last_sample_entry.clone()`)。つまり「変化時のみ Some」ではなく **毎フレーム Some** を返す。したがって上記ロジックだけでは毎フレーム再抽出になるため、**抽出結果が前回値と変化したときのみ更新する比較ガードを必ず追加する**。

## 完了条件

- 解像度変化する H.264 / H.265 (多エントリ stsd) の MP4 で、nvcodec デコーダーによるデコード結果が破損しなくなる
- リグレッションテスト (解像度変化する MP4 を投入して、出力フレームの解像度シーケンスを assert) を追加する

## 解決方法

- `src/decoder/nvcodec.rs` の `NvcodecDecoder::decode()` から parameter_sets 更新条件の `is_none()` を外す
- 毎フレーム Some を返す reader (`src/mp4/sync_reader.rs`) に対して、抽出結果が前回値と変化したときのみ更新する比較ガードを追加する
- テストデータ: hotfix/2025.3.3 (PR #328) で追加した **多エントリ stsd** の `testdata/archive-h264-resolution-change.mp4` (+ json) を git 履歴から develop へ復元する。既存の `testdata/h264-resolution-change.mp4` / `h265-resolution-change.mp4` は単一 stsd で sample_entry が変化しないため、0093 の再現には不適 (in-band パラメータセット変化のみを検出し、VideoToolbox の回帰テストとして維持する)
- hotfix/2025.3.3 で追加した `h264_single_track_resolution_change_nvcodec_passthrough` に相当するテストを、develop の `tests/decoder_tests.rs` の構成 (`Mp4VideoReader::new(path)` + `VideoDecoder::new` + `handle_input_sample` / `poll_output`) に合わせて追加する
