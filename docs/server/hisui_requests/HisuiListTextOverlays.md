# HisuiListTextOverlays

サーバ全体に登録されているテキストオーバーレイの一覧と現在状態を返す。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

なし。

## ResponseData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `textOverlays` | Object[] | 必須 | テキストオーバーレイ配列 (登録順) |

### `textOverlays` 配列の各要素

Create 時に省略可能なフィールド (`fontColor` / `fontName` / `z`) は、 省略時のデフォルト適用または自動割り当て後の現在値が返ります。

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `textOverlayName` | string | 必須 | 識別名 |
| `text` | string | 必須 | 表示中のテキスト |
| `x` | integer | 必須 | キャンバス絶対座標 X |
| `y` | integer | 必須 | キャンバス絶対座標 Y |
| `fontSize` | integer | 必須 | フォントサイズ (px) |
| `fontColor` | string | 必須 | `#RRGGBBAA` 形式 |
| `fontName` | string | 必須 | フォントファイル名 |
| `z` | integer | 必須 | z-order |

## エラー条件

- 機能無効: `REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED` (606)

## 制約

- WebSocket / データチャネル両方で利用可能
- RequestBatch（op=8）に対応
