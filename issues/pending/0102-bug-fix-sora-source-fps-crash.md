# sora source の低 FPS と停止時クラッシュを調査する

- Priority: High
- Created: 2026-09-11
- Completed:
- Branch: feature/fix-sora-source-fps-crash
- Polished:

## 目的

sora source 入力を使ったときに FPS が低い、またプロセス停止時にクラッシュレポートが出ることがある。再現条件と原因を特定し、必要なら修正する。

## pending とした理由

- 実機での再現と libwebrtc 内部 (sink の破棄・配送スレッド) の調査が必要で、原因が単一箇所に特定できていない。
- 現時点で再現手順が安定しておらず、修正方針を確定できないため `issues/pending/` で保留する。

## 現状

- `src/sora_source.rs` の `SoraSubscriber` が Sora に RecvOnly で接続する。
- `VideoFrameSinkHandler::on_frame` は `tokio::sync::mpsc::channel::<RawI420Frame>(2)` に `try_send` し、満杯時はフレームを破棄する (音声側の容量 4 より小さい)。
- `video_forward_task` は全フレームを `keyframe: true` として配送する。`VideoSinkWants::new()` にフレームレート指定は無い。
- 停止経路は `handle_stop_sora_subscriber` と `handle_sora_source_event` の `TrackRemoved` / `Disconnected` で `holder_abort.abort()` を呼び、`sora_track_holder_task` 側で `forward_abort.abort()` と `attached` の Drop を行う。
- sink の UAF については `AttachedSink` の `Drop` が `remove_sink` を呼ぶ対策が入っている (`src/sora_source.rs` のコメント参照)。

## 設計方針 (要検討・未確定)

- 低 FPS について、チャネル容量と `try_send` の破棄が支配的かを計測する。
- `VideoSinkWants` にフレームレート指定を渡すべきか検討する。
- 停止時のクラッシュが残っている場合、sink の破棄順序とタスク abort のタイミングを確認する。

## 完了条件

- 低 FPS の再現条件と原因が特定され、必要なら修正されること。
- プロセス停止時にクラッシュレポートが出ないことが確認されること。

(解決方法は pending のため、調査結果と設計確定後に記載する。)
