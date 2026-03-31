# obsws State File

## 目的

obsws の output 設定を再起動後も復元するための永続化ファイルである。

- 永続化対象: `stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash`
- scenes / inputs / transition / runtime state は含めない
- state file が未指定の場合、永続化は一切行われない（従来どおり起動引数とデフォルト値で動作する）

## セキュリティに関する注意

state file は HLS / MPEG-DASH の S3 認証情報（`accessKeyId` / `secretAccessKey` / `sessionToken`）を平文で保存する。ファイルの権限を適切に管理し、信頼されたローカルファイルとして扱うこと。

## 指定方法

| 指定方法 | 値 |
|---------|---|
| CLI オプション | `--state-file <PATH>` |
| 環境変数 | `HISUI_OBSWS_STATE_FILE` |

- 優先順位: `--state-file` > `HISUI_OBSWS_STATE_FILE`
- 相対パスを指定した場合、起動時に絶対パスへ解決される
- 親ディレクトリが存在しない場合は初回保存時に自動作成される
- ファイルが存在しない場合は初回起動時点ではエラーにならず、初回の保存成功時に新規作成される

## ファイルフォーマット

- 形式: JSONC（JSON with Comments）
- 推奨拡張子: `.jsonc`（`.json` でも読み込み可能だが、`.jsonc` の場合のみコメントが有効になる）

### トップレベル

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `version` | Integer | 必須 | state file のフォーマットバージョン。現在は `1` 固定 |
| `stream` | Object | 省略可 | 配信サービス設定 |
| `record` | Object | 省略可 | 録画設定 |
| `rtmpOutbound` | Object | 省略可 | RTMP アウトバウンド設定 |
| `sora` | Object | 省略可 | Sora WebRTC Publisher 設定 |
| `hls` | Object | 省略可 | HLS 出力設定 |
| `mpegDash` | Object | 省略可 | MPEG-DASH 出力設定 |

省略されたセクションについては state file から上書きせず、起動引数やデフォルト値がそのまま使われる。

### `stream` セクション

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `streamServiceType` | String | 必須 | 配信サービス種別。現在は `"rtmp_custom"` のみ受理する |
| `streamServiceSettings` | Object | 省略可 | 配信サービスの接続設定 |

### `stream.streamServiceSettings` セクション

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `server` | String | 省略可 | RTMP サーバーの URL |
| `key` | String | 省略可 | ストリームキー |

### `record` セクション

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `recordDirectory` | String | 必須 | 録画ファイルの出力先ディレクトリパス。空文字列は不可 |

### `rtmpOutbound` セクション

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `outputUrl` | String | 省略可 | RTMP リッスン URL |
| `streamName` | String | 省略可 | ストリーム名 |

### `sora` セクション

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `signalingUrls` | String[] | 省略可 | シグナリング URL のリスト |
| `channelId` | String | 省略可 | チャンネル ID |
| `clientId` | String | 省略可 | クライアント ID |
| `bundleId` | String | 省略可 | バンドル ID |
| `metadata` | Object | 省略可 | メタデータ（JSON object のみ受理） |

### `hls` セクション

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `destination` | Object | 省略可 | 出力先設定（`type` が `"filesystem"` または `"s3"`） |
| `segmentDuration` | Number | 省略可 | セグメント尺（秒）。正の値。デフォルト: `2.0` |
| `maxRetainedSegments` | Integer | 省略可 | 保持セグメント数。1 以上。デフォルト: `6` |
| `segmentFormat` | String | 省略可 | `"mpegts"` または `"fmp4"`。デフォルト: `"mpegts"` |
| `variants` | Object[] | 省略可 | ABR バリアント定義。空配列は不可 |

### `mpegDash` セクション

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `destination` | Object | 省略可 | 出力先設定（`type` が `"filesystem"` または `"s3"`） |
| `segmentDuration` | Number | 省略可 | セグメント尺（秒）。正の値。デフォルト: `2.0` |
| `maxRetainedSegments` | Integer | 省略可 | 保持セグメント数。1 以上。デフォルト: `6` |
| `variants` | Object[] | 省略可 | ABR バリアント定義。空配列は不可 |
| `videoCodec` | String | 省略可 | ビデオコーデック名。デフォルト: `"H264"` |
| `audioCodec` | String | 省略可 | オーディオコーデック名。デフォルト: `"AAC"` |

### `destination` 共通（`hls` / `mpegDash`）

**filesystem の場合:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `type` | String | 必須 | `"filesystem"` |
| `directory` | String | 必須 | 出力先ディレクトリパス。空文字列は不可 |

**S3 の場合:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `type` | String | 必須 | `"s3"` |
| `bucket` | String | 必須 | S3 バケット名。空文字列は不可 |
| `prefix` | String | 省略可 | S3 プレフィックス |
| `region` | String | 必須 | AWS リージョン。空文字列は不可 |
| `endpoint` | String | 省略可 | カスタム S3 互換エンドポイント |
| `usePathStyle` | Boolean | 省略可 | パススタイル URL を使用する。デフォルト: `false` |
| `credentials` | Object | 必須 | 認証情報 |
| `lifetimeDays` | Integer | 省略可 | オブジェクトのライフタイム（日数）。正の値。設定時は `prefix` が必須 |

### `credentials` オブジェクト

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `accessKeyId` | String | 必須 | AWS アクセスキー ID |
| `secretAccessKey` | String | 必須 | AWS シークレットアクセスキー |
| `sessionToken` | String | 省略可 | 一時的な AWS セッショントークン |

NOTE: `GetOutputSettings` レスポンスには credentials は含まれないが、state file には復元のため平文で保存される。

### `variants` 共通（`hls` / `mpegDash`）

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `videoBitrate` | Integer | 必須 | ビデオビットレート (bps)。正の値 |
| `audioBitrate` | Integer | 必須 | オーディオビットレート (bps)。正の値 |
| `width` | Integer | 省略可 | ビデオ幅（正の偶数）。`height` と両方指定または両方省略 |
| `height` | Integer | 省略可 | ビデオ高さ（正の偶数）。`width` と両方指定または両方省略 |

## 例

```jsonc
{
  "version": 1,
  "stream": {
    "streamServiceType": "rtmp_custom",
    "streamServiceSettings": {
      "server": "rtmp://127.0.0.1:1935/live",
      "key": "stream-main"
    }
  },
  "record": {
    "recordDirectory": "/var/hisui/recordings"
  },
  "rtmpOutbound": {
    "outputUrl": "rtmp://127.0.0.1:1935/live",
    "streamName": "backup"
  },
  "sora": {
    "signalingUrls": ["wss://example.com/signaling"],
    "channelId": "test",
    "metadata": {}
  },
  "hls": {
    "destination": {
      "type": "filesystem",
      "directory": "/var/hisui/hls"
    },
    "segmentDuration": 2.0,
    "maxRetainedSegments": 6,
    "segmentFormat": "mpegts",
    "variants": [
      {"videoBitrate": 2000000, "audioBitrate": 128000}
    ]
  },
  "mpegDash": {
    "destination": {
      "type": "s3",
      "bucket": "my-bucket",
      "prefix": "dash",
      "region": "us-east-1",
      "credentials": {
        "accessKeyId": "AKID...",
        "secretAccessKey": "SECRET..."
      }
    },
    "segmentDuration": 2.0,
    "maxRetainedSegments": 6,
    "variants": [
      {"videoBitrate": 2000000, "audioBitrate": 128000}
    ],
    "videoCodec": "H264",
    "audioCodec": "AAC"
  }
}
```

## 読み込み挙動

state file は `--state-file` が指定されている場合のみ読み込まれる。

| 状況 | 挙動 |
|------|------|
| `--state-file` 未指定 | state の読み書きを一切行わない |
| ファイルが存在しない | 空の state として扱う（エラーにしない） |
| ファイルのパースに成功 | 各 section の値を registry の初期値に反映する |
| ファイルのパースに失敗 | 起動エラーとする |
| `version` が `1` 以外 | 起動エラーとする |

NOTE: 指定された永続 state を信用して起動する設計のため、壊れたファイルを黙って無視しない。

## 書き込み挙動

以下のリクエストが成功した場合に state file を保存する。

- `SetStreamServiceSettings`
- `SetRecordDirectory`
- `SetOutputSettings`

保存時の挙動:

- registry の現在値から全 section を含む JSON を毎回再生成して書き出す（差分保存ではなく完全スナップショット）
- 一時ファイルへ書き込み後に `rename` する atomic write を行う
- 既存のコメントは保持されない（再生成のため消える）

## エラー時の挙動

| エラー種別 | 挙動 |
|-----------|------|
| 読み込み失敗（パースエラー等） | obsws サーバーを起動しない（起動エラー） |
| 書き込み失敗（I/O エラー等） | 該当リクエストにエラーレスポンス（ステータスコード `205`）を返した後、obsws サーバーを終了する |

NOTE: 書き込み失敗時にプロセスを終了するのは、「リクエストは成功したが保存されていない」状態で運用が続くことを防ぐためである。
