# HisuiRemoveTextOverlay

テキストオーバーレイを削除する。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `textOverlayName` | string | 必須 | 対象識別子 |

## ResponseData

なし。

## エラー条件

- 機能無効: `REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED` (606)
- `requestData` 自体が欠落: `REQUEST_STATUS_MISSING_REQUEST_DATA` (301)
- `textOverlayName` の欠落・空文字列: `REQUEST_STATUS_MISSING_REQUEST_FIELD` (300)
- 対象 overlay が存在しない: `REQUEST_STATUS_RESOURCE_NOT_FOUND` (601)

## 制約

- WebSocket / データチャネル両方で利用可能
- RequestBatch（op=8）に対応
