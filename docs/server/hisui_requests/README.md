# hisui 固有リクエスト

OBS WebSocket 仕様には存在しない、hisui が独自に追加したリクエスト一覧。

hisui 固有リクエストは OBS WebSocket 標準の `requestType` と区別するため、すべて `Hisui` プレフィクスを付与する。

## WebRTC

`obsdc` データチャネル経由でのみ利用可能。RequestBatch（op=8）には非対応。

| リクエスト | 説明 |
|-----------|------|
| [HisuiGetWebRtcStats](HisuiGetWebRtcStats.md) | server 側の WebRTC 統計情報を取得する |
| [HisuiSubscribeProgramTracks](HisuiSubscribeProgramTracks.md) | Program 合成結果トラックを購読する |
| [HisuiUnsubscribeProgramTracks](HisuiUnsubscribeProgramTracks.md) | Program 合成結果トラックの購読を解除する |
| [HisuiListWebRtcVideoTracks](HisuiListWebRtcVideoTracks.md) | client から送信中の video track 一覧を取得する |
| [HisuiAttachWebRtcVideoTrack](HisuiAttachWebRtcVideoTrack.md) | video track を `webrtc_source` input に接続する |
| [HisuiDetachWebRtcVideoTrack](HisuiDetachWebRtcVideoTrack.md) | `webrtc_source` input から video track の接続を外す |

## Output 管理

WebSocket / データチャネル両対応。RequestBatch（op=8）に対応。

| リクエスト | 説明 |
|-----------|------|
| [HisuiCreateOutput](HisuiCreateOutput.md) | output インスタンスを作成する |
| [HisuiRemoveOutput](HisuiRemoveOutput.md) | output インスタンスを削除する |

## SoraSubscriber

WebSocket / データチャネル両方で利用可能。RequestBatch（op=8）に対応。

| リクエスト | 説明 |
|-----------|------|
| [HisuiStartSoraSubscriber](HisuiStartSoraSubscriber.md) | subscriber を作成して RecvOnly 接続を開始する |
| [HisuiStopSoraSubscriber](HisuiStopSoraSubscriber.md) | 接続を停止して subscriber を削除する |
| [HisuiListSoraSubscribers](HisuiListSoraSubscribers.md) | 全 subscriber の一覧・状態・設定を取得する |
| [HisuiListSoraSourceTracks](HisuiListSoraSourceTracks.md) | 受信中のリモートトラック一覧を取得する |
| [HisuiAttachSoraSourceTrack](HisuiAttachSoraSourceTrack.md) | トラックを `sora_source` input に紐付ける |
| [HisuiDetachSoraSourceTrack](HisuiDetachSoraSourceTrack.md) | トラックを `sora_source` input から解除する |

## テキストオーバーレイ

WebSocket / データチャネル両対応。RequestBatch（op=8）に対応。

サーバ起動時に `--font-search-root` と `--default-font` の両方を指定した場合のみ利用可能。
機能無効時は全メソッドが `REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED` を返す。

| リクエスト | 説明 |
|-----------|------|
| [HisuiCreateTextOverlay](HisuiCreateTextOverlay.md) | テキストオーバーレイを作成して即時表示する |
| [HisuiUpdateTextOverlay](HisuiUpdateTextOverlay.md) | 既存テキストオーバーレイの属性を部分更新する |
| [HisuiRemoveTextOverlay](HisuiRemoveTextOverlay.md) | テキストオーバーレイを削除する |
| [HisuiListTextOverlays](HisuiListTextOverlays.md) | サーバ全体のテキストオーバーレイ一覧と現在状態を取得する |
