# HisuiGetWebRtcStats

server 側 `PeerConnection::get_stats()` の生 JSON を取得する。
WebRTC 接続の切り分けや観測用途を想定したリクエスト。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## ResponseData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `stats` | object | 成功時に必須 | libwebrtc `get_stats()` の生 JSON |

## 制約

- `obsdc` データチャネル経由でのみ利用可能
- 単発 Request（op=6）でのみ使用可能。RequestBatch（op=8）には非対応

## 備考

- `stats` は libwebrtc の JSON 形式をそのまま返すため、構造は libwebrtc 側の出力に従う
