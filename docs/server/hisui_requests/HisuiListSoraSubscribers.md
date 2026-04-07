# HisuiListSoraSubscribers

全 subscriber の一覧・状態・設定を取得する。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## ResponseData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `subscribers` | array | 成功時に必須 | subscriber 一覧 |

### subscribers 配列の各要素

| フィールド | 型 | 説明 |
|-----------|-----|------|
| `subscriberName` | string | subscriber の識別名 |
| `active` | boolean | 実行中かどうか |
| `settings` | object | 接続設定 |

### settings オブジェクト

| フィールド | 型 | 説明 |
|-----------|-----|------|
| `signalingUrls` | string[] | シグナリング URL 一覧 |
| `channelId` | string \| null | チャネル ID |
| `clientId` | string \| null | クライアント ID |
| `bundleId` | string \| null | バンドル ID |
| `metadata` | object \| null | Sora に送信するメタデータ |
