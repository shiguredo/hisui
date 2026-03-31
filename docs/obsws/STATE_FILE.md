# obsws State File

## 目的

obsws の設定を再起動後も復元するための永続化ファイルである。

- 永続化対象: output 設定（`stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash`）、scene 構成、input 定義、scene item 配置
- 永続化しない項目: runtime state（配信中/録画中の状態）、canvas 設定（CLI 引数で指定）、transition runtime state
- state file が未指定の場合、永続化は一切行われない（従来どおり起動引数とデフォルト値で動作する）

## セキュリティに関する注意

state file は以下の secret を平文で保存する。ファイルの権限を適切に管理し、信頼されたローカルファイルとして扱うこと。

- HLS / MPEG-DASH の S3 認証情報（`accessKeyId` / `secretAccessKey` / `sessionToken`）
- SRT inbound の `passphrase`

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
| `scenes` | Object[] | 省略可 | scene 定義の配列（scene_order 順） |
| `inputs` | Object[] | 省略可 | input 定義の配列 |
| `currentProgramScene` | String | 省略可 | 現在の Program Scene 名 |
| `currentPreviewScene` | String | 省略可 | 現在の Preview Scene 名 |
| `nextInputId` | Integer | 省略可 | 次の input UUID 生成用 ID カウンタ |
| `nextSceneId` | Integer | 省略可 | 次の scene UUID 生成用 ID カウンタ |
| `nextSceneItemId` | Integer | 省略可 | 次の scene item ID カウンタ |

省略されたセクションについては state file から上書きせず、起動引数やデフォルト値がそのまま使われる。

### Output 設定セクション

#### `stream`

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `streamServiceType` | String | 必須 | `"rtmp_custom"` のみ受理 |
| `streamServiceSettings` | Object | 省略可 | `server` / `key` を含む |

#### `record`

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `recordDirectory` | String | 必須 | 録画先パス。空文字列は不可 |

#### `rtmpOutbound`

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `outputUrl` | String | 省略可 | RTMP リッスン URL |
| `streamName` | String | 省略可 | ストリーム名 |

#### `sora`

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `signalingUrls` | String[] | 省略可 | シグナリング URL リスト |
| `channelId` | String | 省略可 | チャンネル ID |
| `clientId` | String | 省略可 | クライアント ID |
| `bundleId` | String | 省略可 | バンドル ID |
| `metadata` | Object | 省略可 | メタデータ（JSON object のみ） |

#### `hls` / `mpegDash`

output 設定の詳細は `PROTOCOL_STATUS.md` の独自 Output セクションを参照。state file のフォーマットは `SetOutputSettings` / `GetOutputSettings` と同等だが、S3 destination の場合は `credentials` オブジェクトを含む。

### `scenes` 配列

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `sceneName` | String | 必須 | scene 名 |
| `sceneUuid` | String | 必須 | scene UUID |
| `items` | Object[] | 省略可 | scene item の配列。省略時は空配列として扱う |
| `transitionOverride` | Object | 省略可 | scene 固有の transition override |

#### `items` 配列（scene item）

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `sceneItemId` | Integer | 必須 | scene item ID |
| `inputUuid` | String | 必須 | 参照先 input の UUID。`inputs` 内に存在する必要がある |
| `enabled` | Boolean | 必須 | 有効/無効 |
| `locked` | Boolean | 必須 | ロック状態 |
| `blendMode` | String | 必須 | `OBS_BLEND_NORMAL` 等のブレンドモード |
| `transform` | Object | 必須 | transform 設定（18 フィールド） |

#### `transitionOverride`

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `transitionName` | String | 省略可 | transition 種別名 |
| `transitionDuration` | Integer | 省略可 | transition 時間（ミリ秒） |

### `inputs` 配列

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputUuid` | String | 必須 | input UUID |
| `inputName` | String | 必須 | input 名 |
| `inputKind` | String | 必須 | input 種別（`image_source` 等） |
| `inputSettings` | Object | 必須 | input 固有の設定（種別ごとに異なる） |

NOTE: SRT inbound の `passphrase` は `GetInputSettings` では返されないが、state file には復元のため平文で保存される。WebRTC source の `trackId` は runtime 管理のため state file には含まれない。

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
  "scenes": [
    {
      "sceneName": "Scene",
      "sceneUuid": "10000000-0000-0000-0000-000000000000",
      "items": [
        {
          "sceneItemId": 1,
          "inputUuid": "00000000-0000-0000-0000-000000000000",
          "enabled": true,
          "locked": false,
          "blendMode": "OBS_BLEND_NORMAL",
          "transform": {
            "positionX": 0.0,
            "positionY": 0.0,
            "rotation": 0.0,
            "scaleX": 1.0,
            "scaleY": 1.0,
            "alignment": 5,
            "boundsType": "OBS_BOUNDS_NONE",
            "boundsAlignment": 0,
            "boundsWidth": 0.0,
            "boundsHeight": 0.0,
            "cropTop": 0,
            "cropBottom": 0,
            "cropLeft": 0,
            "cropRight": 0,
            "cropToBounds": false,
            "sourceWidth": 0.0,
            "sourceHeight": 0.0,
            "width": 0.0,
            "height": 0.0
          }
        }
      ]
    }
  ],
  "currentProgramScene": "Scene",
  "inputs": [
    {
      "inputUuid": "00000000-0000-0000-0000-000000000000",
      "inputName": "my-camera",
      "inputKind": "image_source",
      "inputSettings": {
        "file": "/path/to/image.png"
      }
    }
  ],
  "nextInputId": 1,
  "nextSceneId": 1,
  "nextSceneItemId": 2
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
| scene item の `inputUuid` が `inputs` に存在しない | 起動エラーとする |
| `currentProgramScene` が `scenes` に存在しない | 起動エラーとする |

NOTE: 指定された永続 state を信用して起動する設計のため、壊れたファイルを黙って無視しない。

## 書き込み挙動

以下のリクエストが成功した場合に state file を保存する。

**Output 設定:**
- `SetStreamServiceSettings` / `SetRecordDirectory` / `SetOutputSettings`

**Scene:**
- `CreateScene` / `RemoveScene` / `SetCurrentProgramScene`

**Input:**
- `CreateInput` / `RemoveInput` / `SetInputSettings` / `SetInputName`

**Scene Item:**
- `CreateSceneItem` / `RemoveSceneItem` / `DuplicateSceneItem` / `SetSceneItemEnabled` / `SetSceneItemLocked` / `SetSceneItemIndex` / `SetSceneItemBlendMode` / `SetSceneItemTransform`

**Transition:**
- `SetSceneSceneTransitionOverride`

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
