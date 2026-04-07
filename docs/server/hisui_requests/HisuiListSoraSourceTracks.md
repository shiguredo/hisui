# HisuiListSoraSourceTracks

受信中のリモートトラック一覧を取得する。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `subscriberName` | string | - | 対象の subscriber 名。省略時は全 subscriber のトラックを返す |
