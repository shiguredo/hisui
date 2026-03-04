# OBS 配信 リクエスト列 例

## 参照仕様

- Protocol: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>
- 想定バージョン: obs-websocket 5.x / rpcVersion = 1
- メッセージ形式: `obswebsocket.json` ( JSON over text )

## このファイルの目的

- OBS WebSocket の仕様に沿って、配信関連の代表的なリクエスト列をまとめる
- シンプルな手順から複雑な手順まで段階的に示す
- 現在の hisui 実装可否ではなく、将来実装時の仕様ベース参照として使う

## 表記ルール

- `C -> S`: Client から Server へ送信
- `S -> C`: Server から Client へ送信
- JSON は要点のみを記載し、一部を省略する

## 主要概念と関係

### 主要概念

- `Input`
  - 映像 / 音声ソースの入力単位
  - 例: `image_source`, `dshow_input`
- `Scene`
  - 複数の `Input` を束ねる論理グループ
- `Scene Item`
  - `Scene` に配置された `Input` インスタンス
- `Program Scene`
  - 現在の出力に使われる `Scene`
  - `SetCurrentProgramScene` で切り替える
- `Canvas`
  - 合成時の基準座標系と出力サイズ
- `Output`
  - 実際の出力処理の実体
  - 主な種類: `Stream` / `Record`
- `Stream`
  - ネットワーク配信出力
  - `StartStream` / `StopStream` で制御
- `Record`
  - ローカル録画出力
  - `StartRecord` / `StopRecord` で制御

### 概念間の関係

- `Input` は `Scene` に `Scene Item` として追加される
- `Program Scene` が現在の出力元として選ばれる
- 選ばれた `Program Scene` が `Canvas` 上で合成される
- 合成結果が `Output` として `Stream` / `Record` に流れる
- `Stream` と `Record` は独立して開始 / 停止できる

### 構造イメージ

```text
Input -> Scene (Scene Item) -> Program Scene -> Canvas -> Output (Stream / Record)
```

### この後の例との対応

- `例 3`: `Canvas` の確認 (`GetCanvasList`)
- `例 5` / `例 6`: `Record` と `Stream` の制御
- `例 7` / `例 8`: `Scene` と `Input` の構築

---

## 例 1: 最小接続 ( Hello -> Identify -> Identified )

目的: Request を送信できる状態に入る

1. `S -> C` Hello (`op: 0`)

```json
{
  "op": 0,
  "d": {
    "obsStudioVersion": "30.2.2",
    "obsWebSocketVersion": "5.5.2",
    "rpcVersion": 1
  }
}
```

2. `C -> S` Identify (`op: 1`)

```json
{
  "op": 1,
  "d": {
    "rpcVersion": 1,
    "eventSubscriptions": 33
  }
}
```

3. `S -> C` Identified (`op: 2`)

```json
{
  "op": 2,
  "d": {
    "negotiatedRpcVersion": 1
  }
}
```

---

## 例 2: 既存設定で配信開始 / 停止

目的: 既存の Scene / Service 設定を使って配信を開始し、停止する

1. `C -> S` GetStreamStatus

```json
{
  "op": 6,
  "d": {
    "requestType": "GetStreamStatus",
    "requestId": "req-001"
  }
}
```

2. `C -> S` StartStream

```json
{
  "op": 6,
  "d": {
    "requestType": "StartStream",
    "requestId": "req-002"
  }
}
```

3. `C -> S` StopStream

```json
{
  "op": 6,
  "d": {
    "requestType": "StopStream",
    "requestId": "req-003"
  }
}
```

---

## 例 3: Canvas を確認して配信前提を検証する

目的: 利用可能な Canvas と Program Scene を事前確認する

1. `C -> S` GetCanvasList

```json
{
  "op": 6,
  "d": {
    "requestType": "GetCanvasList",
    "requestId": "req-101"
  }
}
```

2. `S -> C` RequestResponse ( 抜粋 )

```json
{
  "op": 7,
  "d": {
    "requestType": "GetCanvasList",
    "requestId": "req-101",
    "requestStatus": { "result": true, "code": 100 },
    "responseData": {
      "canvases": [
        {
          "canvasName": "Base",
          "canvasWidth": 1920,
          "canvasHeight": 1080
        }
      ]
    }
  }
}
```

3. `C -> S` GetCurrentProgramScene

```json
{
  "op": 6,
  "d": {
    "requestType": "GetCurrentProgramScene",
    "requestId": "req-102"
  }
}
```

NOTE: `Canvas` は確認系 API が中心で、配信開始そのものは `StartStream` で制御する

---

## 例 4: 配信先を更新してから配信開始

目的: RTMP の接続先を API で設定してから配信する

1. `C -> S` SetStreamServiceSettings

```json
{
  "op": 6,
  "d": {
    "requestType": "SetStreamServiceSettings",
    "requestId": "req-201",
    "requestData": {
      "streamServiceType": "rtmp_custom",
      "streamServiceSettings": {
        "server": "rtmp://127.0.0.1/live",
        "key": "example-stream-key"
      }
    }
  }
}
```

2. `C -> S` GetStreamServiceSettings

```json
{
  "op": 6,
  "d": {
    "requestType": "GetStreamServiceSettings",
    "requestId": "req-202"
  }
}
```

3. `C -> S` StartStream

```json
{
  "op": 6,
  "d": {
    "requestType": "StartStream",
    "requestId": "req-203"
  }
}
```

---

## 例 5: 録画の開始 / 停止

目的: 録画の基本制御を行う

1. `C -> S` GetRecordStatus

```json
{
  "op": 6,
  "d": {
    "requestType": "GetRecordStatus",
    "requestId": "req-301"
  }
}
```

2. `C -> S` StartRecord

```json
{
  "op": 6,
  "d": {
    "requestType": "StartRecord",
    "requestId": "req-302"
  }
}
```

3. `C -> S` GetRecordStatus

```json
{
  "op": 6,
  "d": {
    "requestType": "GetRecordStatus",
    "requestId": "req-303"
  }
}
```

4. `C -> S` StopRecord

```json
{
  "op": 6,
  "d": {
    "requestType": "StopRecord",
    "requestId": "req-304"
  }
}
```

---

## 例 6: 配信と録画を同時運用する

目的: 配信と録画を独立に開始 / 停止する

1. `C -> S` StartStream
2. `C -> S` StartRecord
3. `C -> S` GetStreamStatus
4. `C -> S` GetRecordStatus
5. `C -> S` StopRecord
6. `C -> S` StopStream

NOTE: 停止順序は運用要件で決める。意図を明確にするため、クライアントは順序を固定して送信する

---

## 例 7: Scene と Input を作成してから配信開始

目的: 空状態から配信用の Scene / Input を構築する

1. `C -> S` CreateScene

```json
{
  "op": 6,
  "d": {
    "requestType": "CreateScene",
    "requestId": "req-401",
    "requestData": {
      "sceneName": "Program"
    }
  }
}
```

2. `C -> S` CreateInput

```json
{
  "op": 6,
  "d": {
    "requestType": "CreateInput",
    "requestId": "req-402",
    "requestData": {
      "sceneName": "Program",
      "inputName": "Main Camera",
      "inputKind": "dshow_input",
      "inputSettings": {
        "video_device_id": "default"
      },
      "sceneItemEnabled": true
    }
  }
}
```

3. `C -> S` SetCurrentProgramScene

```json
{
  "op": 6,
  "d": {
    "requestType": "SetCurrentProgramScene",
    "requestId": "req-403",
    "requestData": {
      "sceneName": "Program"
    }
  }
}
```

4. `C -> S` StartStream

```json
{
  "op": 6,
  "d": {
    "requestType": "StartStream",
    "requestId": "req-404"
  }
}
```

NOTE: `inputKind` と `inputSettings` の詳細キーはプラットフォームや Input 実装ごとに異なる

---

## 例 8: RequestBatch で配信準備をまとめる

目的: 複数の準備 Request を 1 回で送る

1. `C -> S` RequestBatch (`haltOnFailure = true`)

```json
{
  "op": 8,
  "d": {
    "requestId": "batch-001",
    "haltOnFailure": true,
    "executionType": 0,
    "requests": [
      {
        "requestType": "CreateScene",
        "requestData": { "sceneName": "BatchScene" }
      },
      {
        "requestType": "CreateInput",
        "requestData": {
          "sceneName": "BatchScene",
          "inputName": "Batch Image",
          "inputKind": "image_source",
          "inputSettings": { "file": "/path/to/image.png" },
          "sceneItemEnabled": true
        }
      },
      {
        "requestType": "SetCurrentProgramScene",
        "requestData": { "sceneName": "BatchScene" }
      },
      {
        "requestType": "SetStreamServiceSettings",
        "requestData": {
          "streamServiceType": "rtmp_custom",
          "streamServiceSettings": {
            "server": "rtmp://127.0.0.1/live",
            "key": "batch-key"
          }
        }
      }
    ]
  }
}
```

2. `S -> C` RequestBatchResponse (`op: 9`) ( 抜粋 )

```json
{
  "op": 9,
  "d": {
    "requestId": "batch-001",
    "results": [
      {
        "requestType": "CreateScene",
        "requestStatus": { "result": true, "code": 100 }
      },
      {
        "requestType": "CreateInput",
        "requestStatus": { "result": true, "code": 100 }
      }
    ]
  }
}
```

3. `C -> S` StartStream

```json
{
  "op": 6,
  "d": {
    "requestType": "StartStream",
    "requestId": "req-501"
  }
}
```

---

## 例 9: 認証あり接続と Reidentify

目的: 認証付きで接続し、接続中にイベント購読設定を更新する

1. `S -> C` Hello ( `authentication` あり )
2. `C -> S` Identify ( `authentication` を含む )
3. `S -> C` Identified
4. `C -> S` Reidentify (`op: 3`)

```json
{
  "op": 3,
  "d": {
    "eventSubscriptions": 1
  }
}
```

NOTE: `authentication` 文字列は `protocol.md` の手順 ( SHA256 + Base64 ) で生成する

---

## 例 10: 失敗時レスポンスの扱い

目的: `requestStatus` の `code` / `comment` で分岐する

1. `C -> S` 失敗しうる Request ( 例: 不正な Scene 名 )
2. `S -> C` RequestResponse (`result = false`)

```json
{
  "op": 7,
  "d": {
    "requestType": "SetCurrentProgramScene",
    "requestId": "req-err-1",
    "requestStatus": {
      "result": false,
      "code": 608,
      "comment": "Parameter: sceneName"
    }
  }
}
```

3. クライアント側の推奨対応

- `requestId` 単位で成否を追跡する
- `result = false` の場合は `code` と `comment` をログに残す
- 冪等な Request は再試行し、非冪等な Request は状態再取得後に再実行する
