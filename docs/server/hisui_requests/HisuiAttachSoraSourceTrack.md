# HisuiAttachSoraSourceTrack

受信中のリモートトラックを `sora_source` input に紐付ける。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputName` | string | 必須 | 紐付け先の `sora_source` input 名 |
| `connectionId` | string | 必須 | 紐付けるトラックの接続 ID |
| `trackKind` | string | 必須 | トラック種別（`"video"` or `"audio"`） |

## エラー条件

- `sora_source` 以外の入力への attach は失敗する
