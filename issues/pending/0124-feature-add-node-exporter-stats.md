# node exporter 連携の統計出力に対応する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-node-exporter-stats
- Polished:

## 目的

Prometheus の node exporter の textfile collector と連携できる統計出力を追加する。ファイルを 1 つにまとめ、プロセスの開始と終了時に排他ロックを取って更新する方式を想定する。

## pending とした理由

- 出力ファイルの排他制御、複数プロセスでの衝突回避、書き込みタイミングの設計が必要。
- 既存の `/metrics` エンドポイントとの役割分担を決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/stats.rs` の `Stats` が `to_prometheus_text` / `to_prometheus_json_families` を提供し、`PROMETHEUS_METRIC_PREFIX` は `hisui_`。
- `src/endpoint_http_metrics.rs` の `handle_request` が `GET /metrics` (Prometheus text) と `GET /metrics?format=json` を返す。
- `src/metrics.rs` の `emit_exit_metrics_to_stdout` と `--emit-exit-metrics` (`HISUI_EMIT_EXIT_METRICS`) で終了時に stdout へ JSON Lines を出す。
- `docs/internals/stats.md` に仕様がある。
- textfile collector 向けのファイル出力は存在しない。

## 設計方針 (要検討・未確定)

- `.prom` ファイルへ書き出すオプションを追加する。
- ファイルの排他ロックと、開始 / 終了時の更新タイミングを設計する。
- `/metrics` との使い分けをドキュメント化する。

## 完了条件

- node exporter の textfile collector から統計を取得できること。

(解決方法は pending のため、設計確定後に記載する。)
