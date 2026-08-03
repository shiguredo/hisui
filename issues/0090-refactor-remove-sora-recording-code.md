# Sora 録画機能 (compose / vmaf / tune) と関連コードの削除

- Created: 2026-08-03
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-remove-sora-recording-code
- Polished: {YYYY-MM-DD}

## 目的

Sora 録画合成機能 (compose / vmaf / tune サブコマンド) は sora-archive-compositor に移行する方針が
README「今後の Hisui について」に記載されている。hisui に残る録画専用のコード・ドキュメント・
テストデータを削除し、Media Processing Tool としての構成に整理する。

## 現状

hisui には録画合成専用の実装が残っている。

- `src/sora/` 配下の 19 ファイル (約 7900 行) が録画合成専用モジュール (`recording_*`)
- `src/main.rs` が compose / vmaf / tune の 3 サブコマンドをディスパッチしている
- `src/tune.rs` と `src/tune/` (nsga2 / storage / rng / json_value) は tune サブコマンド専用
- `src/encoder.rs` の `default_video_encode_config_for_rpc` が `recording_layout_encode_params::LayoutEncodeParams::default()` に依存している (RPC 既定値が録画モジュールに依存)
- `src/stats.rs` の `as_bool_for_sora_recording_compose` は compose stats JSON 専用
- 録画専用ドキュメント: `docs/command_compose.md` / `docs/command_tune.md` / `docs/command_vmaf.md` /
  `docs/layout.md` / `docs/layout_decode_params.md` / `docs/layout_encode_params.md` /
  `docs/layout_region.md` / `docs/layout_spec.md` / `docs/migrate_hisui_legacy.md` (compose 移行ガイドが中心)。
  さらに `docs/internals/` の一部 (architecture_overview / stats / sample_entry_invariant 等) や
  `docs/usage.md` / `docs/build.md` / `docs/docker.md` にも compose 等への言及がある
- 録画専用テストデータ: `testdata/files/` / `testdata/files2/` / `testdata/layouts/` /
  `testdata/source_timestamps/` / `testdata/trim/` / `testdata/e2e/` (compose 系ディレクトリ) 等。
  `testdata/e2e/transcribe/` や音声 PCM は transcribe 用なので残す
- 録画モジュールに依存するテスト: `tests/e2e.rs` (compose 系)、`tests/layout_tests.rs`、
  `tests/mixer_audio_tests.rs`、`tests/mixer_video_tests.rs`、`tests/test_tune.rs`、
  `tests/decoder_tests.rs`、`tests/writer_mp4_tests.rs` (`recording_mp4_reader` /
  `recording_layout` 等を参照)
- `src/mp4/hybrid_writer.rs` のテストが `recording_mp4_reader` に依存
- Cargo.toml に録画専用の依存ライブラリが残る (shiguredo_vmaf 等。削除後 unused になるものを
  実装時に特定する)

なお、以下は削除対象外とする。

- `src/sora_source.rs` / `src/sora_publisher.rs` / `src/obsws/coordinator/output_sora.rs` /
  `examples/sora_*`: Sora WebRTC によるリアルタイム入出力であり、録画機能ではない
- `src/subcommand_inspect.rs`: 録画ファイルの情報取得コマンドだが `src/sora/` には依存せず、
  汎用ツールとして残す

## 設計方針

- 削除は録画合成機能 (compose / vmaf / tune) に限定し、リアルタイム系の機能は変更しない
- 録画モジュールに依存していた共通基盤は依存を解消して残す
  (`default_video_encode_config_for_rpc` の RPC 既定値を録画モジュール非依存の構成へ置き換える等)
- 録画モジュールを参照するテストは、テスト自体の削除または他モジュールへの書き換えで対応する
  (decoder / encoder / mixer 基盤のテストを録画モジュールなしで成立させる)
- `testdata/` は録画専用分のみ削除する。inspect が参照する `testdata/archive-*.mp4` 等は残す
- 残るドキュメントは録画機能への参照をすべて除去・改訂する

## 完了条件

- `src/sora/` が削除され、compose / vmaf / tune サブコマンドが無くなっている
- `src/encoder.rs` の `default_video_encode_config_for_rpc` 等、録画モジュールへの依存が
  すべて解消されている
- 録画専用ドキュメント・テストデータが削除され、残るドキュメントに録画機能への参照が残っていない
- `cargo build` / `cargo test` と CI が通る
- README が録画機能削除後の構成に整合している

## 解決方法

1. `src/sora/` (19 ファイル) を削除する
2. `src/main.rs` から compose / vmaf / tune のディスパッチを削除する
3. `src/tune.rs` と `src/tune/` を削除する
4. `src/encoder.rs` の `default_video_encode_config_for_rpc` を録画モジュール非依存に書き換える
5. `src/stats.rs` の `as_bool_for_sora_recording_compose` を削除する
6. 録画専用ドキュメントを削除し、残るドキュメントから compose / vmaf / tune / recording への
   参照を除去する
7. 録画モジュールに依存するテストを削除・書き換える (`tests/e2e.rs` は compose テストを削除して
   inspect テストを残す、`src/mp4/hybrid_writer.rs` のテストは別の reader に書き換える等)
8. 録画専用テストデータを削除する
9. Cargo.toml から録画専用の依存を削除する
10. README を更新し、CHANGES.md に削除を記載する
11. `src/sora/` を参照する既存 issue (0005 / 0069 / 0070 / 0074 / 0075 / 0085 / 0087、
    pending の 0015 / 0016 / 0029 等) を確認し、録画機能を対象とするものは closed または
    pending へ整理する
