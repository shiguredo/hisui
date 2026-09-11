# Media Pipeline のフックと JSON-RPC 拡張を実装する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-media-pipeline-rpc
- Polished:

## 目的

デコード前後・合成前後・エンコード前後などのフックポイントから JSON-RPC 2.0 で外部プロセスを呼び出せるようにし、プラグインなしで hisui の処理を拡張できるようにする。

## pending とした理由

- フックポイントの位置と、やりとりするデータ (音声 / 映像 / バイナリ) とプロトコルの設計が必要。
- 現状の `MediaPipeline` は crate 内 processor 間の型付き RPC のみで、外部 JSON-RPC とは別物であり、どう接続するかの検討が必要。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/media_pipeline.rs` の `MediaPipeline` / `MediaPipelineHandle` / `ProcessorHandle` / `ErasedRpcSender` は processor 間の RPC sender 登録 (`register_rpc_sender` / `get_rpc_sender`) を提供する。
- `docs/internals/media_pipeline.md` に processor 間 RPC の説明がある。
- 外部から接続できる JSON-RPC 2.0 サーバー / クライアント、フック、プラグインの実装は無い。README の将来構想として記載されている。

## 設計方針 (要検討・未確定)

- フックポイント (デコード前後・合成前後・エンコード前後など) を定義する。
- JSON-RPC 2.0 のクライアント / サーバーを追加し、`Content-Type` / `Content-Length` と `application/octet-stream` の扱いを決める。
- 既存の `MediaPipeline` の processor 間 RPC との接続方法を決める。

## 完了条件

- 外部プロセスをフック経由で呼び出せること。
- 音声 / 映像の受け渡し方法がドキュメント化されていること。

(解決方法は pending のため、設計確定後に記載する。)
