# HisuiDetachWebRtcVideoTrack

`webrtc_source` input と上り video track の接続を外す。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputName` | string | 必須 | 接続を外す `webrtc_source` input 名 |

## ResponseData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputName` | string | 成功時に必須 | input 名 |
| `trackId` | string \| null | 成功時に必須 | 接続が外された video track の ID。未接続だった場合は null |

## 制約

- `obsdc` データチャネル経由でのみ利用可能
- 単発 Request（op=6）でのみ使用可能。RequestBatch（op=8）には非対応

## 備考

- 未接続時は no-op で成功する
