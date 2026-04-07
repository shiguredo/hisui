# HisuiDetachSoraSourceTrack

`sora_source` input からトラックの紐付けを解除する。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputName` | string | 必須 | 解除する `sora_source` input 名 |
| `trackKind` | string | 必須 | 解除するトラック種別（`"video"` or `"audio"`） |
