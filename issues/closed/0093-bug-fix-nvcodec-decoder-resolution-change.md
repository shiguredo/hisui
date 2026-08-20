# nvcodec デコーダーが sample_entry 変化に追従せず解像度変化する H.264 / H.265 のデコード結果が壊れる

- Created: 2026-08-07
- Updated: 2026-08-13
- Completed: 2026-08-14
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

reader 側の sample_entry 供給ポリシーを確認する必要がある。develop の `src/mp4/sync_reader.rs` は、`last_sample_entry` を保持して **全フレームに** sample_entry を付与する (`sample_entry: self.last_sample_entry.clone()`)。つまり「変化時のみ Some」ではなく **毎フレーム Some** を返す。

毎フレーム再抽出になっても問題はない。`extract_parameter_sets_annexb` は同一 sample_entry から決定的に同じバイト列を返すため、毎フレーム上書きしても機能的に等価だからである。AGENTS.md の「Premature Optimization is the Root of All Evil」に照らし、機能的に正しい実装に前回値比較ガードを追加しないことを選択した (比較ガードは最適化であり、それが実際に必要な証拠がない)。

## 完了条件

- 解像度変化する H.264 / H.265 (多エントリ stsd) の MP4 で、nvcodec デコーダーによるデコード結果が破損しなくなる
- リグレッションテスト (解像度変化する MP4 を投入して、出力フレームの解像度シーケンスを assert) を追加する

## 解決方法

- `src/decoder/nvcodec.rs` の `NvcodecDecoder::decode()` から parameter_sets 更新条件の `is_none()` を外し、毎フレーム抽出してキャッシュを更新する (前回値比較ガードは入れない。設計方針参照)
- テストデータ: hotfix/2025.3.3 (PR #328) で追加した **多エントリ stsd** の `testdata/archive-h264-resolution-change.mp4` (+ json) を git 履歴から develop へ復元する。既存の `testdata/h264-resolution-change.mp4` / `h265-resolution-change.mp4` は単一 stsd で sample_entry が変化しないため、0093 の再現には不適 (in-band パラメータセット変化のみを検出し、VideoToolbox の回帰テストとして維持する)
- hotfix/2025.3.3 で追加した `h264_single_track_resolution_change_nvcodec_passthrough` に相当するテストを、develop の `tests/decoder_tests.rs` の構成 (`Mp4VideoReader::new(path)` + `VideoDecoder::new` + `handle_input_sample` / `poll_output`) に合わせて追加する

### 対応内容

- `src/decoder/nvcodec.rs` の `NvcodecDecoder::decode()` から `is_none()` を外し、毎フレーム抽出してキャッシュを更新するよう修正
- `extract_parameter_sets_annexb` / `contains_parameter_sets` の単体テストを追加
- `testdata/archive-h264-resolution-change.mp4` / `testdata/archive-h265-resolution-change.mp4` (多エントリ stsd) を develop へ復元
- `tests/decoder_tests.rs` に `h264_single_track_resolution_change_nvcodec` / `h265_single_track_resolution_change_nvcodec` の回帰テストを追加
- issue 0093 の設計方針 (比較ガードは入れない) に沿って、CHANGES.md の記述も実装に合わせて修正
