# --dump-metrics-on-exit を main.rs の共通フラグにして全 subcommand で使えるようにする

- Priority: Low
- Created: 2026-06-05
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

issue 0018 で追加した `--dump-metrics-on-exit`（プロセス終了時に全メトリクスを `{"type":"metrics", ...}` の 1 行 JSON で stdout 出力するフラグ）は server サブコマンド固有だが、メトリクス（`Stats`）を持つのは server だけではない。`--dump-metrics-on-exit` を `src/main.rs` で処理する共通フラグに格上げし、メトリクスを持つ全 subcommand の終了時にダンプできるようにする。

## 優先度根拠

Low。server 固有版（issue 0018）で当面の用途（server の flaky 診断）は満たせており、本件はその汎用化。時間があれば対応する。

## 現状

- `--dump-metrics-on-exit` は server サブコマンド固有。`src/subcommand_server.rs` で parse し、`run_server` → `run_accept_loop` の SIGTERM signal 分岐でダンプする。
- `src/main.rs:22-30` に共通フラグの前例（`--verbose`）がある。ただし `--verbose` は logger 初期化というグローバル副作用で完結し、subcommand へ値を渡す必要がない。
- メトリクス（`MediaPipeline` / `Stats`）を持つ subcommand は server（常駐）のほか、grep 上 compose / vmaf / inspect が `MediaPipeline` を参照する（実際にダンプ対象とするかは実装時に確認する）。
- compose は既に `--stats-file`（`src/sora/recording_subcommand_compose.rs:56`）で stats を**ファイル**出力できる。stdout の終了時ダンプとは出力先・形式が異なる。
- プロセス全体で共有する `Stats` レジストリは無く、`Stats` は各 subcommand の pipeline が持ち、subcommand の return で寿命が尽きる。そのため main.rs から一律にダンプすることはできない。

## 設計方針

- `--dump-metrics-on-exit`（＋ env `HISUI_DUMP_METRICS_ON_EXIT`）の parse を `src/main.rs` の共通フラグ領域（`--verbose` 付近）へ移し、subcommand 分岐前に取得する。CLI 上の位置は `--dump-metrics-on-exit <subcommand>` になる（現状の `server --dump-metrics-on-exit` から変わる。0018 は未リリースのため互換問題なし）。
- ダンプ実行は各 subcommand の終了点で行う（main.rs からは一律にできない）。フラグ値を各 subcommand の `try_run` へ渡す。
  - server: 既存の SIGTERM signal 分岐でダンプする（issue 0018 で実装済み。フラグの受け取り元が変わるだけ）。
  - バッチ系（compose / vmaf / inspect 等）: pipeline 完了後・return 直前に、その `Stats` の `to_metrics_dump_json_line()` で 1 行ダンプする。
- メトリクスを持たない subcommand（list-codecs / tune 等）はフラグを受けても no-op。混乱を避けるため、ダンプ対象の subcommand を help / ドキュメントで明示する。
- compose の既存 `--stats-file` との棲み分けを整理する（両立でよいか、stdout ダンプで代替するか）。

## 完了条件

- `--dump-metrics-on-exit` が `src/main.rs` の共通フラグとして subcommand 分岐前に受け付けられること。
- メトリクスを持つ subcommand（server ＋ ダンプ対象とした batch 系）の終了時に `{"type":"metrics", ...}` の 1 行が stdout に出ること。
- 既存の server e2e（issue 0018 で追加）がフラグ位置変更後も通ること。
- ダンプ対象の subcommand と no-op の subcommand が help / ドキュメントで判別できること。
- `CHANGES.md` の 0018 関連エントリを共通フラグ前提に更新すること（0018 が同一リリースに含まれる場合。別リリースなら別エントリとする）。

## 関連

- issues/0018（server 固有版。本 issue はその汎用化）
