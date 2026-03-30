# obsws State File

## 目的

obsws の `streamServiceSettings` と `recordDirectory` を再起動後も復元するための永続化ファイルである。

- 永続化対象は `streamServiceSettings`（配信サービス設定）と `recordDirectory`（録画ディレクトリ）のみ
- state file が未指定の場合、永続化は一切行われない（従来どおり起動引数とデフォルト値で動作する）

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
| `stream` | Object | 省略可 | 配信サービス設定。省略時は起動引数のデフォルト値を使用する |
| `record` | Object | 省略可 | 録画設定。省略時は起動引数のデフォルト値を使用する |

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

## 例

```jsonc
{
  // state file のバージョン（現在は 1 固定）
  "version": 1,
  // 配信サービス設定
  "stream": {
    "streamServiceType": "rtmp_custom",
    "streamServiceSettings": {
      "server": "rtmp://127.0.0.1:1935/live",
      "key": "stream-main"
    }
  },
  // 録画設定
  "record": {
    "recordDirectory": "/var/hisui/recordings"
  }
}
```

`stream` のみ、`record` のみの記述も可能である。省略されたセクションについては state file から上書きせず、起動引数やデフォルト値がそのまま使われる。

```jsonc
{
  "version": 1,
  "stream": {
    "streamServiceType": "rtmp_custom",
    "streamServiceSettings": {
      "server": "rtmp://192.168.1.100:1935/live"
    }
  }
}
```

## 読み込み挙動

state file は `--state-file` が指定されている場合のみ読み込まれる。

| 状況 | 挙動 |
|------|------|
| `--state-file` 未指定 | state の読み書きを一切行わない |
| ファイルが存在しない | 空の state として扱う（エラーにしない） |
| ファイルのパースに成功 | `stream` / `record` の値を `ObswsInputRegistry` の初期値に反映する |
| ファイルのパースに失敗 | 起動エラーとする |
| `version` が `1` 以外 | 起動エラーとする |
| `streamServiceType` が `"rtmp_custom"` 以外 | 起動エラーとする |
| `record` セクションに `recordDirectory` がない、または空文字列 | 起動エラーとする |

NOTE: 指定された永続 state を信用して起動する設計のため、壊れたファイルを黙って無視しない。

## 書き込み挙動

以下のリクエストが成功した場合に state file を保存する。

- `SetStreamServiceSettings`
- `SetRecordDirectory`
- `SetOutputSettings`

保存時の挙動:

- `ObswsInputRegistry` の現在値から `stream` と `record` の両方を含む JSON を毎回再生成して書き出す
- 一時ファイルへ書き込み後に `rename` する atomic write を行う
- 既存のコメントは保持されない（再生成のため消える）

## エラー時の挙動

| エラー種別 | 挙動 |
|-----------|------|
| 読み込み失敗（パースエラー等） | obsws サーバーを起動しない（起動エラー） |
| 書き込み失敗（I/O エラー等） | 該当リクエストにエラーレスポンス（ステータスコード `205`）を返した後、obsws サーバーを終了する |

NOTE: 書き込み失敗時にプロセスを終了するのは、「リクエストは成功したが保存されていない」状態で運用が続くことを防ぐためである。
