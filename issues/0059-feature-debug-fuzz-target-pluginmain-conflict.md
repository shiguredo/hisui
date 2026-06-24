# cargo fuzz バイナリの main が protobuf compiler の PluginMain に乗っ取られる問題を調査する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/debug-fuzz-target-pluginmain-conflict
- Polished:

## 目的

CI に追加した `test-fuzz` ジョブが `fuzz_h264_sample_entry` の起動時点で
`Unknown flag: -a` を出して exit code 1 で終了する。原因を突き止めて
`test-fuzz` ジョブを正しく動くようにする。

## 優先度根拠

CI の `test-fuzz` ジョブを一時無効化しているため、 fuzz target が
ビルド + 起動できる状態を継続確認できていない。 fuzz バイナリ自体が
全く起動できないため、ローカルで cargo fuzz を回そうとしても同様に
失敗し、 fuzz による回帰検査ができない状態である。リリースを止める
ほどではないが、放置すれば fuzz が事実上機能しないので Medium。

## 現状

- CI ログ (run 28077107441, job 83123562653) では
  `fuzz/target/.../release/fuzz_h264_sample_entry -artifact_prefix=... -max_total_time=30 .../corpus/...`
  実行直後に `Unknown flag: -a` が出力されて exit code 1 で終了している。
- ローカル (macOS aarch64) でも `cargo +nightly fuzz run fuzz_h264_sample_entry -- -max_total_time=5`
  でほぼ同じ症状 (`Unknown option: -artifact_prefix=...`) が出る。
- `strings` で fuzz バイナリ内に protobuf の plugin 用文字列
  (`: Unknown option: `、 `protoc asked plugin to generate a file but did not provide a descriptor for the file: ` など) が含まれる。
- `nm` + `c++filt` で fuzz バイナリの `_main` を確認したところ、
  `google::protobuf::compiler::PluginMain(int, char**, CodeGenerator const*)` を呼ぶ
  protoc plugin の main がリンクされていた。 libFuzzer の main が選ばれていない。
- fuzz target 自体は `#![no_main]` で libfuzzer-sys 0.4.13 の `fuzz_target!` を使う
  通常構成。 hisui crate を経由して何らかの C++ 静的ライブラリ
  (おそらく `libwebrtc_c.a` か関連) が protoc plugin の `int main` 入り
  オブジェクトファイルを巻き込んでいる、というのが現時点の仮説。

## 設計方針

- fuzz バイナリの中の `_main` がどの object file 由来かを特定する。
  - 静的ライブラリ (`libwebrtc_c.a` ほか) を `ar t` で展開し、 `main` を持つ
    object を `nm` で抜き出すなどして突き止める。
- 原因が prebuilt の静的ライブラリにある場合は、上流 (shiguredo/webrtc-build など)
  に対する報告 / 修正と、 hisui 側の暫定回避策の両方を検討する。
- hisui の fuzz 側で回避できる場合 (例: `-z muldefs` 相当の linker フラグや、
  protoc plugin のオブジェクトを除外するリンク順序の制御) はその対処も
  選択肢に含める。

## 完了条件

- `cargo +nightly fuzz run fuzz_h264_sample_entry -- -max_total_time=30` が
  ローカルおよび CI で fuzz バイナリとして起動し、 libFuzzer のフラグを
  正しく受け付けるようになっている。
- CI の `test-fuzz` ジョブの `if: false` を外して再有効化できている。
