# プロセス終了時に全メトリクスを JSONL で出力するフラグを追加し、失敗診断を改善する

- Priority: Medium
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

hisui プロセスの終了時に、全メトリクスを JSON Lines で標準出力（または標準エラー出力）へ吐けるフラグを追加する。短命プロセス（e2e テストのサーバ、compose バッチ等）の最終状態を確実かつ完全に取得でき、失敗診断や障害解析が容易になる。直接の動機は issues/0011 の調査で、e2e が `/metrics` 全文を `AssertionError` メッセージに埋め込んだ結果 CI ログで切り詰められ、目的のメトリクス（`total_missing_*` / `total_received_*` 等）が読めなかったこと。

## 優先度根拠

Medium。テスト失敗や障害の原因調査の効率に直結し、汎用的に役立つ。ユーザー向け機能ではないため High ではない。

## 現状

- hisui は HTTP で `/metrics`（Prometheus テキスト）と `/metrics?format=json`（prom2json 準拠 JSON。`src/endpoint_http_metrics.rs:95-100` が `Stats::entries()` を JSON 化）を公開するが、プロセス終了時に自分でメトリクスを吐く手段が無い。
- obsws e2e は失敗時に `/metrics` 全文をスクレイプし（`e2e-tests/obsws/helpers.py:565` の `_collect_obsws_metrics_snapshot_async`）、それを `_format_obsws_diagnostics`（`e2e-tests/obsws/helpers.py:516`）で `AssertionError` メッセージに埋め込む（`e2e-tests/obsws/helpers.py:497-499`、呼び出しは `e2e-tests/obsws/test_output.py:876 / 992 / 1113`）。
- `/metrics` 出力は `BTreeMap` 由来でメトリクス名のアルファベット順。巨大なスナップショットを assert メッセージに丸ごと載せるため、CI ログで後方が切り詰められると後ろのメトリクスが読めない。実際に issues/0011 の調査で `total_finalize_*` は読めたが `total_missing_audio_sample_entry_count` / `total_received_audio_sample_entry_count` が読めなかった。

## 設計方針

### 1. hisui 側: 終了時メトリクスダンプ

- `--dump-metrics-on-exit`（`noargs::opt`、env: `HISUI_DUMP_METRICS_ON_EXIT`）を追加する。出力先（stdout / stderr、または destination を値で受けられるようにするか）と boolean / 値あり opt のどちらにするかは実装時に詰める。
- 有効時、プロセスの graceful 終了時に全メトリクスを JSON Lines（1 行 1 メトリクス。各行に metric 名・labels・value）で出力する。
- データ源は `/metrics?format=json` と同じく `Stats::entries()`（`src/stats.rs:151`）を再利用し、各 `StatsEntry`（`src/stats.rs:525`）を 1 行の JSON として nojson でシリアライズする。
- 出力フックは obsws server のシャットダウン経路（`src/obsws/server.rs` / `src/obsws/coordinator.rs`）等に置く。
- 制約: SIGKILL では出力できない（graceful 終了のみ）。server プロセスはシャットダウン時にまとめて出すため、録画ごとではなく最終状態のダンプになる（録画 processor 終了後もメトリクスはレジストリに残存することを issues/0011 の停止後スナップショットで確認済み）。

### 2. e2e 側: フラグを既定で使い、スクレイプ・埋め込みを縮小する

- E2E サーバ起動（`ObswsServer.start`、`e2e-tests/obsws/helpers.py:68-126`）で `--dump-metrics-on-exit` を既定で付与し、全テストが graceful 停止時に終了ダンプを得られるようにする。
- 失敗診断は server 終了ダンプ（captured output。`_emit_captured_output` で失敗時に print 済み）の最終メトリクスに委ねる。`AssertionError` への `/metrics` 全文埋め込み（`_format_obsws_diagnostics` の metrics_snapshot 部分）と、それ向けの `_collect_obsws_metrics_snapshot_async` でのスクレイプを廃止し、assert は簡潔な失敗理由のみにする。未使用の `_print_obsws_diagnostics` は整理（活用または削除）する。
- 録画中のライブ待ち（`_has_positive_metric` で `/metrics` を見て映像サンプル書き込みを待つ等、`e2e-tests/obsws/test_output.py:288-292`）は終了前の状態が必要なので HTTP `/metrics` のまま残す。

## 完了条件

- フラグ有効時、graceful 終了で全メトリクスが JSON Lines で出力されること（行ごとに 1 メトリクスで grep / parse できる）。
- E2E サーバ（`ObswsServer`）が既定で `--dump-metrics-on-exit` を使い、全テストで終了ダンプが得られること。
- obsws 録画系 e2e テストの失敗時に、最終メトリクス（finalize 成否 / sample_entry の received・missing 等）が切り詰められず確実に読めること。
- `AssertionError` メッセージに `/metrics` 全文が埋め込まれないこと。

## 関連

- issues/0011（本改善の必要性が判明した調査元。reopen 済み）
- issues/0017（音声 sample_entry 要求機構の修正。その flaky 検証時に本機能の診断が役立つ）
