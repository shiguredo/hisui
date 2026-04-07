# HisuiStartSoraSubscriber

subscriber を作成して Sora チャネルに RecvOnly で接続を開始する。
sora-rust-sdk を使い、リモートトラック（映像・音声）を受信する。受信したトラックは `sora_source` 入力タイプを通じてシーンに配置できる。

複数の subscriber を同時に接続し、異なるチャネルから受信できる。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `subscriberName` | string | 必須 | subscriber の識別名 |
| `signalingUrls` | string[] | 必須（1 件以上） | シグナリング URL 一覧 |
| `channelId` | string | 必須 | チャネル ID |
| `clientId` | string | - | クライアント ID |
| `bundleId` | string | - | バンドル ID |
| `metadata` | object | - | Sora に送信するメタデータ（JSON object のみ） |

## エラー条件

- 同名の subscriber が既に稼働中: `OUTPUT_RUNNING` を返す
- `signalingUrls` が空: `INVALID_REQUEST_FIELD` を返す
- `channelId` が未設定: `MISSING_REQUEST_FIELD` を返す
