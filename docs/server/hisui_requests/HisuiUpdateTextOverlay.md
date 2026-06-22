# HisuiUpdateTextOverlay

既存テキストオーバーレイの属性を部分更新する。

送信したフィールドのみ更新され、省略したフィールドは現状値が維持される。
`textOverlayName` は識別子なので変更不可。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `textOverlayName` | string | 必須 | 対象識別子 (変更不可) |
| `text` | string | - | 省略時は現状維持 |
| `x` | integer | - | 同上 |
| `y` | integer | - | 同上 |
| `fontSize` | integer | - | 同上 |
| `fontColor` | string | - | 同上 (形式は `HisuiCreateTextOverlay` と同じ) |
| `fontName` | string | - | 同上 (解決可能なファイル名であること) |
| `z` | integer | - | 同上 (i32 範囲。Auto z への再戻しはサポートしない) |

## ResponseData

なし。

## エラー条件

- 機能無効: `REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED` (606)
- `requestData` 自体が欠落: `REQUEST_STATUS_MISSING_REQUEST_DATA` (301)
- `textOverlayName` の欠落・空文字列: `REQUEST_STATUS_MISSING_REQUEST_FIELD` (300)
- 対象 overlay が存在しない: `REQUEST_STATUS_RESOURCE_NOT_FOUND` (601)
- 更新後の値の検証エラー (`fontName` / `fontColor` / `fontSize` / `text` 等): `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- `z` が i32 範囲外: `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- raden 描画失敗: `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)

検証ロジックは `HisuiCreateTextOverlay` と共通。

## 制約

- WebSocket / データチャネル両方で利用可能
- RequestBatch（op=8）に対応
