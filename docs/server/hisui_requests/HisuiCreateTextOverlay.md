# HisuiCreateTextOverlay

合成映像にテキストオーバーレイを作成して即時表示する。

サーバ起動時に `--font-search-root` と `--default-font` の両方を指定して
テキストオーバーレイ機能が有効になっている場合のみ利用できる。
機能無効時は `RESOURCE_ACTION_NOT_SUPPORTED` を返す。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `textOverlayName` | string | 必須 | サーバ全体で一意な識別名 |
| `text` | string | 必須 | 表示するテキスト。`\n` で改行可能。最大 4096 バイト、最大 64 行 |
| `x` | integer | 必須 | キャンバス絶対座標 X (左上原点、px)。負値・キャンバス外も許容 (描画は raden 側でクリップ) |
| `y` | integer | 必須 | キャンバス絶対座標 Y (左上原点、px)。同上 |
| `fontSize` | integer | 必須 | フォントサイズ (px)。`1` 以上 `canvas_height` 以下 |
| `fontColor` | string | - | 文字色。`#RRGGBB` または `#RRGGBBAA` (正規表現 `^#[0-9A-Fa-f]{6}([0-9A-Fa-f]{2})?$`)。省略時は `#FFFFFFFF` (不透明白) |
| `fontName` | string | - | `--font-search-root` 配下のフォントファイル名 (拡張子付き)。省略時は `--default-font` の値 |
| `z` | integer | - | テキストオーバーレイ間の z-order (`i32::MIN..=i32::MAX - 1`)。`i32::MAX` は内部で全テキストオーバーレイ用に予約されているため指定不可。省略時は宣言順 (現在登録されている最大 z + 1 が自動割り当てされ、後勝ち) |

## ResponseData

なし。

## エラー条件

- 機能無効 (`--font-search-root` / `--default-font` 未指定): `REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED` (606)
- `requestData` 自体が欠落: `REQUEST_STATUS_MISSING_REQUEST_DATA` (301)
- 必須フィールド (`textOverlayName` / `text` / `x` / `y` / `fontSize`) の欠落・空文字列: `REQUEST_STATUS_MISSING_REQUEST_FIELD` (300)
- 必須フィールドの型不一致 (string 期待で integer 等): `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- 同名 overlay が既に存在: `REQUEST_STATUS_RESOURCE_ALREADY_EXISTS` (602)
- `fontName` が `/` `\` `..` NUL バイトを含む: `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- `fontName` 解決失敗 (ファイルなし / ルート外 / シンボリックリンクでルート外 / フォント破損): `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- `fontColor` の形式違反: `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- `fontSize` の範囲外 (`0` または `canvas_height` 超過): `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- `z` が `i32::MIN..=i32::MAX - 1` の範囲外 (`i32::MAX` 含む): `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- `text` のバイト数 / 行数上限超過: `REQUEST_STATUS_INVALID_REQUEST_FIELD` (400)
- 同時保持できるオーバーレイ数 (最大 64) を超える: `REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED` (606)

## 制約

- WebSocket / データチャネル両方で利用可能
- RequestBatch（op=8）に対応
