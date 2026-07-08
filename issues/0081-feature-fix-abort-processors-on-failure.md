# processor 失敗時に残りの processor を中断してハングを防ぐ

- Priority: Medium
- Created: 2026-07-07
- Completed:
- Model: Opus 4.8
- Branch: feature/fix-abort-processors-on-failure
- Polished:

## 目的

`compose` / `vmaf` サブコマンドで、1 つの processor が起動途中でエラー終了した際に、相互接続された別の processor が起動ハンドシェイク待ちで停止し、処理全体が永久ハングしうる問題を修正する。

## 優先度根拠

Medium。ハングは「processor が起動途中で失敗する」構成ミスや異常時に限られ、正常系では発生しない。ただし発生するとプロセスがハングして CI・バッチ処理を止めるため、放置はできない。

## 現状

MediaPipeline 上の複数 processor は publish/subscribe で相互接続され、`publish_track` / `wait_subscribers_ready` などの起動ハンドシェイクで待ち合わせる。1 つが起動途中でエラー終了すると、それを待っている別 processor が永久に待ち続けうる。両サブコマンドの `wait_processor_tasks` は、この失敗時に残タスクを中断しない。

- `src/sora/recording_subcommand_compose.rs` の `wait_processor_tasks`: `tokio::task::JoinSet` を `join_next` で待つ。processor が失敗しても `success = false` にしてログするだけで残タスクを中断せず、timeout も持たない。
- `src/sora/recording_subcommand_vmaf.rs` の `wait_processor_tasks`: `Vec<SpawnedProcessorTask>` を pop して 1 つずつ await する構造で `timeout` 引数を持つ。timeout 到達時は残タスクを `abort()` するが、processor が Err で失敗した場合は中断せず次を await する。timeout を指定しない経路ではハングしうる。

両者は構造 (JoinSet と Vec + 手動 abort) が異なるため、修正は同一にはならない。

## 設計方針

- compose: 1 つでも processor が失敗 (`Ok(Err)` または `JoinError`) した時点で `JoinSet::abort_all()` を呼び、残タスクを中断する。
- vmaf: 失敗検知時に残 `Vec` のタスクを `abort()` して打ち切る (既存の timeout 時 abort と揃える)。
- 中断されたタスクを「失敗」として扱わないこと。中断後に join したタスクは `JoinError::is_cancelled()` が真になるため、これを判定して意図的中断を error ログや失敗カウントに含めず、最初に検知した本来の失敗だけを記録する。compose で `abort_all()` 後に `join_next` ループを継続すると中断タスクが `Err(cancelled)` として再取得されるため、この区別が必須。
- timeout 由来の中断 (vmaf の既存挙動) と失敗由来の中断で、ログと戻り値の扱いを一貫させる。

## 完了条件

- compose / vmaf のどちらでも、1 つの processor が失敗したら残 processor が中断され、`wait_processor_tasks` がハングせず速やかに返る。
- 中断されたタスクが失敗としてログ・カウントされず、本来の失敗のみが記録される。
- 上記を検証するテストが追加され green である。

## 解決方法

- compose の `wait_processor_tasks` に失敗時の `abort_all()` を追加し、`is_cancelled()` で中断タスクのログを抑制する。
- vmaf の `wait_processor_tasks` に失敗時の abort を追加し、既存の timeout abort と挙動を揃える。
- テスト: 意図的に Err で終了する processor を含む pipeline を組み、(1) 残 processor が中断されて `wait_processor_tasks` が返ること、(2) 中断タスクが失敗として記録されないこと、を assert する。pipeline 組み立ての前例は `tests/decoder_tests.rs` を参照する。
