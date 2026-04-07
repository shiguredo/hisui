# HisuiStopSoraSubscriber

接続を停止して subscriber を削除する。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `subscriberName` | string | 必須 | 停止する subscriber の識別名 |

## エラー条件

- 未登録の subscriber を停止: `RESOURCE_NOT_FOUND` を返す
