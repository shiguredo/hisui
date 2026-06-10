# `--dump-metrics-on-exit` を main.rs の共通フラグへ昇格し、main 側で共有 `Stats` を保持する

- Priority: Low
- Created: 2026-06-05
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

issue 0018 で server サブコマンド固有として導入した `--dump-metrics-on-exit`（プロセス終了時に全メトリクスを `{"type":"metrics", ...}` の 1 行 JSON で stdout 出力するフラグ）を、`src/main.rs` で受け付ける共通フラグへ昇格させる。あわせて、`Stats` レジストリを main 側で 1 つ作って `MediaPipeline` を持つ subcommand に渡し、終了時の dump も main 側で実行する枠組みに整える。

既存の batch 系 subcommand（compose / vmaf / inspect）を能動的に dump 対象に組み込むことは本 issue の主目的ではない。あくまで「将来追加する `MediaPipeline` 保有 subcommand が、コマンド側でフラグを意識せずに同じ仕組みへ素直に乗れる状態」を作ることが本 issue のゴールである。

## 優先度根拠

Low。server 固有版（0018）で当面の用途（server の flaky 診断）は満たせており、既存 batch 系への対応拡大は単体ではメリットが薄い（compose は既に `--stats-file` あり、vmaf / inspect は完了時の出力経路が別途ある）。一方、将来追加するであろう新 subcommand が共通フラグで終了時ダンプを得られる枠組みは、後で整理し直すより今のうちに整えておく方が安い。

## 現状

- `--dump-metrics-on-exit` の parse は `src/subcommand_server.rs:113` で server 固有。`run_server` → `run_accept_loop` の SIGTERM 分岐（`src/obsws/server.rs:405`）でダンプする。
- ダンプ本体は `src/obsws/server.rs:100` の `dump_metrics_to_stdout(pipeline_handle)`。内部で `pipeline_handle.stats().to_prometheus_json_families()` を呼んで `{"type":"metrics", "metrics": ...}` を JSON Lines で stdout に書く。
- `src/main.rs:22-30` に共通フラグの前例（`--verbose`）がある。ただし `--verbose` は logger 初期化というグローバル副作用で完結し、subcommand へ値を渡す必要がない。
- `Stats`（`src/stats.rs:11`）は `Arc<Mutex<...>>` で内部状態を共有しており、`clone()` しても同じレジストリを指す shareable な値。
- `MediaPipeline::new()` / `new_with_config()`（`src/media_pipeline.rs:58-92`）は内部で `Stats::new()` を作成する。`Stats` を外から差し込む API は無い。
- 実プロダクションパスでの `MediaPipeline::new()` / `new_with_config()` 呼び出し箇所:
  - `src/subcommand_inspect.rs:105`
  - `src/sora/recording_subcommand_vmaf.rs:247`
  - `src/sora/recording_subcommand_compose.rs:269`
  - `src/obsws/server.rs:270`
- tune は内部で `current_exe` 経由で compose の子プロセスを起動する作りで、tune 親プロセス自身は `MediaPipeline` を持たない。list-codecs も持たない。
- 既存の server e2e: `e2e-tests/obsws/test_output.py:2096-2132` の `test_obsws_dump_metrics_on_exit_outputs_jsonl` / `test_obsws_dump_metrics_on_exit_disabled`。引数組み立ては `e2e-tests/obsws/helpers.py:78,132` で `args = ["--verbose", "server"]` の後ろに `--dump-metrics-on-exit` を append している。
- `CHANGES.md:59-60` に 0018 関連のエントリがある。

## 設計方針

- `--dump-metrics-on-exit`（＋ env `HISUI_DUMP_METRICS_ON_EXIT`）の parse を `src/main.rs` の `--verbose` 直後（subcommand 分岐前）へ移す。CLI 上の位置は `hisui --dump-metrics-on-exit server ...` になる（現状の `hisui server ... --dump-metrics-on-exit` から変わる。0018 は未リリースのため互換問題なし）。
- main 側で `Stats` を 1 つ作り、`MediaPipeline` を持つ subcommand の `try_run` 引数で `stats: Stats` を渡す。`MediaPipeline` を持たない list-codecs / tune には渡さない。
- `MediaPipeline::new_with_stats(stats)` および `new_with_config_and_stats(config, stats)` を新設し、既存の `new()` / `new_with_config()` は内部で `Stats::new()` を作って委譲する形にする（既存シグネチャを維持し、テストや obsws/source 内の `MediaPipeline::new()` への波及をゼロにする）。
- 上記「現状」に挙げた実プロダクションパスの 4 箇所だけ、新 API（`new_with_stats` / `new_with_config_and_stats`）に置き換える。
- 現状の `src/obsws/server.rs:100` `dump_metrics_to_stdout(pipeline_handle: &MediaPipelineHandle)` を `src/stats.rs` 内の自由関数 `dump_to_stdout(stats: &Stats)` として再配置する（呼び出し側で `pipeline_handle.stats()` を取らずに直接 `Stats` を渡せるようにする）。
- main の subcommand 分岐の return 後、フラグが立っていれば `crate::stats::dump_to_stdout(&stats)` を呼ぶ。
- server 側の `dump_metrics_on_exit` 受け渡しチェーン（`src/subcommand_server.rs:113-117, 171, 192, 247, 286` と `src/obsws/server.rs:140, 364, 389, 405-407`）は全削除する。SIGTERM 分岐内での dump 呼び出しも削除し、dump は main の末尾に一本化する。
- server のエラー return 時（SIGTERM 以外の原因で `try_run` が `?` で抜けるケース）に dump するかは本 issue では現状維持（出さない）。RAII guard 等は導入しない。

### dump タイミングの変化に関する補足

現状の server: SIGTERM 受信 → SIGTERM 分岐内で dump → graceful shutdown → return。

新方式の server: SIGTERM 受信 → graceful shutdown → return → main 末尾で dump。

server return から main 末尾までの間に他の重い処理はないため、出力される JSON Line の内容と発生事実は変わらない。e2e テストは「stdout に `type=metrics` の JSON Line が含まれるか」だけを見ているので、タイミングのズレは試験結果に影響しない想定。

## 完了条件

- `--dump-metrics-on-exit` が `src/main.rs` の共通フラグとして subcommand 分岐前に受け付けられること（CLI フラグおよび env `HISUI_DUMP_METRICS_ON_EXIT` の両方）。
- main 側で 1 つの `Stats` インスタンスを作り、`MediaPipeline` を持つ全 subcommand（server / compose / vmaf / inspect）の `MediaPipeline` が同じ `Stats` を参照すること。
- `--dump-metrics-on-exit` を ON にして server を起動 → SIGTERM で停止したとき、`{"type":"metrics", ...}` の 1 行が stdout に出ること。
- 既存の server e2e（`test_obsws_dump_metrics_on_exit_outputs_jsonl` / `test_obsws_dump_metrics_on_exit_disabled`）が、フラグ位置変更後も通ること。
- `e2e-tests/obsws/helpers.py:78,132` の引数組み立てが、`--dump-metrics-on-exit` を `server` の前に置く新しいフラグ位置に追従していること。
- `src/subcommand_server.rs` と `src/obsws/server.rs` から `dump_metrics_on_exit` の受け渡しチェーンが削除され、SIGTERM 分岐内での dump 呼び出しも削除されていること。
- `dump_to_stdout(stats: &Stats)` が `src/stats.rs` 内に再配置されていること。
- `CHANGES.md` の 0018 関連エントリを共通フラグ前提に更新すること（0018 と同一リリースに含まれる場合。別リリースなら別エントリとする）。

## 非対象

- compose / vmaf / inspect を「能動的に dump 対象として整備すること」: 本 issue では何もしない。ただし設計方針の自然な帰結として、これらは main 側で作った共有 `Stats` を参照する `MediaPipeline` を持つため、`--dump-metrics-on-exit` を立てた状態で実行すれば main 末尾で dump は出る状態になる。「対応コマンドとして doc に列挙する」「dump 内容を batch 系に合わせて整える」といった対応は行わない。
- フラグの doc 文に対応 subcommand 一覧を網羅すること: 将来増えるたびに doc 修正が必要になるため、対応コマンドの列挙はしない。「終了時にメトリクスを stdout へ JSON Lines で出力する」程度の説明にとどめる。
- compose の `--stats-file` との棲み分け整理: 両者は独立した出力経路として残す（stdout の Prometheus JSON families と、compose 独自スキーマの file 出力）。
- server のエラー return 時にも dump すること: 現状は SIGTERM 分岐でのみ dump しており、新方式でも「正常 return 時のみ main 末尾で dump」となる。RAII guard 等は導入しない。必要が出れば別 issue で扱う。
- tune が呼ぶ子プロセスへのフラグ伝搬: tune 親は `MediaPipeline` を直接持たないため自動で対象外。子プロセスへの伝搬は将来必要になれば別 issue で扱う。

## 関連

- issues/closed/0018-feature-add-dump-metrics-on-exit.md（server 固有版。本 issue はその枠組み整理）
