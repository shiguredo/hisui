# e2e 失敗診断のメトリクス出力を改善する（/metrics 全文を assert メッセージに埋めない）

- Priority: Medium
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

obsws e2e テストの失敗診断で、関連メトリクスを切り詰めなく確実に読めるようにする。issues/0011 の調査時に、失敗時の /metrics スナップショットが CI ログで切れて目的のメトリクスが読めず、原因切り分けが阻害された。

## 優先度根拠

Medium。テスト失敗時の原因調査の効率に直結する。今回実際に診断が阻害され、別経路（コード読解・ローカル推論）での切り分けを強いられた。ユーザー機能には影響しないため High ではない。

## 現状

- `e2e-tests/obsws/helpers.py` の `_collect_obsws_metrics_snapshot_async`（`e2e-tests/obsws/helpers.py:565-572`）は `/metrics` 全文を無加工で取得して返す。
- テスト（例: `obsws/test_output.py` の録画系）はこのスナップショットを `AssertionError` のメッセージにそのまま埋め込む。
- `/metrics` の Prometheus 出力は `BTreeMap` 由来でメトリクス名のアルファベット順に並ぶ。巨大なスナップショットを assert メッセージに丸ごと載せるため、CI ログ（pytest / gh の表示）で後半が切り詰められると、アルファベット後方のメトリクス（`hisui_total_missing_*` / `hisui_total_received_*` 等）が読めなくなる。
- 実際に issues/0011 の調査では `hisui_total_finalize_*` は読めたが `hisui_total_missing_audio_sample_entry_count` / `hisui_total_received_audio_sample_entry_count` が切り詰めで読めなかった。

## 設計方針

- `/metrics` 全文を assert メッセージに丸ごと埋め込むのをやめる。次のいずれか（または併用）:
  - (a) 診断に必要な関連メトリクスだけを抽出して出力する（例: 対象 processor の `finalize` 成否、`sample_entry` の received / missing、`error` 等）。
  - (b) full snapshot は CI アーティファクト、もしくはサーバ stdout/stderr を print している既存方式（`e2e-tests/obsws/helpers.py` の `_emit_captured_output`）と同様に別 print として出力し、assert メッセージには要約のみ載せる。

## 完了条件

- 録画系 e2e テストの失敗時に、診断に必要な関連メトリクスが切り詰められず確実に読めること（CI ログ上で確認できること）。
- 失敗時のサーバ stdout/stderr 出力（既存）と整合する形で、メトリクス診断が得られること。

## 関連

- issues/0011（本改善の必要性が判明した調査元）
