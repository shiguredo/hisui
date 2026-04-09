# HisuiRemoveOutput

output インスタンスを削除する。

稼働中の output を削除しようとした場合はエラーを返す。先に `StopOutput` で停止してから削除すること。

デフォルトで作成された output（`stream`、`record`）も削除可能。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `outputName` | string | 必須 | 削除する output インスタンスの識別名 |

## ResponseData

なし。

## エラー条件

- 指定された output が存在しない: `RESOURCE_NOT_FOUND` を返す
- 指定された output が稼働中: `OUTPUT_RUNNING` を返す
- `outputName` が未設定: `MISSING_REQUEST_FIELD` を返す

## 制約

- WebSocket / データチャネル両方で利用可能
- RequestBatch（op=8）に対応
