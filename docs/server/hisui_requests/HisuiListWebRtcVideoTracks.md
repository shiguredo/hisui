# HisuiListWebRtcVideoTracks

現在の bootstrap セッションで client から送信中の video track 一覧を取得する。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## ResponseData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `tracks` | array | 成功時に必須 | video track 一覧 |

### tracks 配列の各要素

| フィールド | 型 | 説明 |
|-----------|-----|------|
| `trackId` | string | WebRTC video track の ID |
| `attachedInputName` | string \| null | attach 先の input 名。未接続時は null |

## 制約

- `obsdc` データチャネル経由でのみ利用可能
- 単発 Request（op=6）でのみ使用可能。RequestBatch（op=8）には非対応

## 備考

- track が 0 本でも成功で空配列を返す
- 対象は video のみ
