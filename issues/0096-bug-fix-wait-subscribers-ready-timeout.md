# 初期 processor が notify_ready を呼ばずにハングした場合の起動永久待ちを防ぐ

- Created: 2026-08-04
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-wait-subscribers-ready-timeout
- Polished: {YYYY-MM-DD}

## 目的

初期 processor 群の起動ハンドシェイクが、processor 自身のハングによって永久待ちになる問題を防ぐ。

## 現状

- `src/media_pipeline.rs` の `ProcessorHandle::wait_subscribers_ready` は oneshot チャネルで `initial_ready_open` の開放を待つが、 timeout を持たない
- 初期 processor が `notify_ready()` を呼ぶ前に自身がハング (デッドロック等) した場合、 `pending_initial_processors` が空にならず `try_open_initial_ready` が発火しないため、 起動が永久待ちになり得る
- 背景: 0081 (processor 失敗時に残りの processor を中断してハングを防ぐ) は録画機能削除に伴い closed としたが、 その際の調査で「失敗 processor の検知」では残存経路はハングしない一方、 「processor 自身が notify_ready 前にハングする」ケースは timeout がなく永久待ちになり得ることが判明した

## 設計方針

polish で確定する。 候補:

- `ProcessorHandle::wait_subscribers_ready` に timeout を設け、 タイムアウト時にエラー (または明示的な失敗) を返す
- ハングした processor の特定 (processor_id の出力等) を併せて行う

## 完了条件

- 初期 processor が `notify_ready` を呼ばない状況でも、 起動が永久待ちにならず一定時間でエラーになること
- 既存の正常起動 (obsws / server 等) に回帰がないこと
