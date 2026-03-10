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

## hisui 対応状況の見方

- `対応済み`: 現在の hisui 実装でその例の主要フローを実行できる
- `部分対応`: 主要フローは実行できるが、制約や未実装部分がある
- `未対応`: 現在の hisui 実装ではその例を実行できない

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
  - 多くの運用では実質 1 つを使うが、仕様上は `GetCanvasList` で複数要素を返せる
- `Output`
  - 実際の出力処理の実体
  - この文書で扱う種類: `Stream` / `Record`
- `Stream`
  - ネットワーク配信出力
  - `StartStream` / `StopStream` で制御
- `Record`
  - ローカル録画出力
  - `StartRecord` / `StopRecord` で制御

### 概念間の関係

- `Input` は `Scene` に `Scene Item` として追加される
- `Program Scene` が現在の出力元として選ばれる
- 概念モデル上は、選ばれた `Program Scene` が `Canvas` の座標系で合成される
- 合成結果が `Output` として `Stream` / `Record` に流れる
- `Stream` と `Record` は独立して開始 / 停止できる

### Canvas がリストで返る理由

- 仕様の拡張性を保つため、`Canvas` は単数ではなくリストとして扱える形で定義されている
- 実装差分や将来機能を吸収しやすくするため、クライアントには複数要素の可能性を残している
- そのためクライアント実装は「`Canvas` は必ず 1 個」という前提を置かない方が安全
- 一方で、単純な配信用途では 1 個の `Canvas` だけで十分なケースが多い

### Canvas 操作の必須性について

- 配信の最小フローは `StartStream` / `StopStream` を中心に成立し、`Canvas` の明示操作が毎回必要なわけではない（ `ToggleStream` でも同様に制御可能 ）
- `Canvas` は内部の合成モデルを理解するための概念として重要だが、API 呼び出しとしては参照 (`GetCanvasList`) が中心になる
- 複数 `Canvas` を返す実装に備えるため、必要時のみ `GetCanvasList` を参照して判断する運用が現実的

### 構造イメージ

```text
Input -> Scene (Scene Item) -> Program Scene -> Canvas -> Output (Stream / Record)
```

NOTE: この図は概念上の依存関係を示す。実際の最小 API フローでは `Canvas` を直接操作しない場合がある

### この後の例との対応

- `例 2`: `Canvas` を直接操作しない最小配信フロー
- `例 3`: `Canvas` の確認 (`GetCanvasList`)
- `例 5` / `例 6`: `Record` と `Stream` の制御
- `例 7` / `例 8`: `Scene` と `Input` の構築

---

## 例 1: 最小接続 ( Hello -> Identify -> Identified )

目的: Request を送信できる状態に入る
hisui 対応状況: 対応済み

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
hisui 対応状況: 部分対応（ `StartStream` は `image_source` 1 件 + `rtmp_custom` 前提 ）

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

目的: 利用可能な Canvas 一覧を事前確認する（ Program Scene は別軸で確認する ）
hisui 対応状況: 部分対応（ `GetCanvasList` の内容は現時点で固定値中心 ）

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

NOTE: `Canvas` は確認系 API が中心で、配信開始そのものは `StartStream` で制御する。`GetCurrentProgramScene` は `Canvas` 非対応の request であり、Canvas ごとの Program Scene を返す API ではない

---

## 例 4: 配信先を更新してから配信開始

目的: RTMP の接続先を API で設定してから配信する
hisui 対応状況: 部分対応（ `SetStreamServiceSettings` は現時点で `rtmp_custom` の `server` / `key` を対象 ）

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

NOTE: hisui の `RequestBatch` は現時点で `executionType = 0` のみを受け付ける。`haltOnFailure` は対応済み

---

## 例 5: 録画の開始 / 停止

目的: 録画の基本制御を行う
hisui 対応状況: 部分対応（ `GetRecordStatus` / `ToggleRecord` / `StartRecord` / `StopRecord` / `PauseRecord` / `ResumeRecord` / `ToggleRecordPause` は対応済み。入力種別と構成に制約あり ）

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

4. `C -> S` PauseRecord

```json
{
  "op": 6,
  "d": {
    "requestType": "PauseRecord",
    "requestId": "req-304"
  }
}
```

5. `C -> S` ResumeRecord

```json
{
  "op": 6,
  "d": {
    "requestType": "ResumeRecord",
    "requestId": "req-305"
  }
}
```

6. `C -> S` StopRecord

```json
{
  "op": 6,
  "d": {
    "requestType": "StopRecord",
    "requestId": "req-306"
  }
}
```

---

## 例 6: 配信と録画を同時運用する

目的: 配信と録画を独立に開始 / 停止する
hisui 対応状況: 部分対応（ `StartStream` / `StopStream` / `StartRecord` / `StopRecord` は対応済み。入力種別と構成に制約あり ）

1. `C -> S` StartStream
2. `C -> S` StartRecord
3. `C -> S` GetStreamStatus
4. `C -> S` GetRecordStatus
5. `C -> S` StopRecord
6. `C -> S` StopStream

NOTE: 停止順序は運用要件で決める。意図を明確にするため、クライアントは順序を固定して送信する
NOTE: 配信と録画の encoder 共有 / 非共有は出力設定依存。obsws の request / event の並びだけでは直接識別しない
NOTE: 共有 / 非共有の判定は設定値・ログ・メトリクスを併用して確認する

---

## 例 7: Scene と Input を作成してから配信開始

目的: 空状態から配信用の Scene / Input を構築する
hisui 対応状況: 部分対応（ `Scene` / `Input` 系は対応済み。`StartStream` は入力種別と構成に制約あり ）

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

## 例 7.1: Input 設定を更新する

目的: 既存 Input の `inputSettings` を上書き / 置換する  
hisui 対応状況: 対応済み（ `SetInputSettings` は `overlay` 指定に対応 ）

1. `C -> S` SetInputSettings（ overlay 更新 ）

```json
{
  "op": 6,
  "d": {
    "requestType": "SetInputSettings",
    "requestId": "req-405",
    "requestData": {
      "inputName": "Main Camera",
      "inputSettings": {
        "device_id": "camera-2"
      }
    }
  }
}
```

2. `C -> S` SetInputSettings（ 置換更新 ）

```json
{
  "op": 6,
  "d": {
    "requestType": "SetInputSettings",
    "requestId": "req-406",
    "requestData": {
      "inputName": "Main Camera",
      "inputSettings": {},
      "overlay": false
    }
  }
}
```

3. `C -> S` GetInputSettings

```json
{
  "op": 6,
  "d": {
    "requestType": "GetInputSettings",
    "requestId": "req-407",
    "requestData": {
      "inputName": "Main Camera"
    }
  }
}
```

NOTE: Inputs 購読（ `eventSubscriptions` に `OBSWS_EVENT_SUB_INPUTS` を設定 ）時は、更新成功後に `InputSettingsChanged` が配信される

---

## 例 7.2: Input 名変更と既定設定を取得する

目的: 既存 Input 名を変更し、`inputKind` の既定設定を取得する  
hisui 対応状況: 対応済み（ `SetInputName` / `GetInputDefaultSettings` に対応 ）

1. `C -> S` SetInputName

```json
{
  "op": 6,
  "d": {
    "requestType": "SetInputName",
    "requestId": "req-408",
    "requestData": {
      "inputName": "Main Camera",
      "newInputName": "Main Camera Renamed"
    }
  }
}
```

NOTE: Inputs 購読（ `eventSubscriptions` に `OBSWS_EVENT_SUB_INPUTS` を設定 ）時は、変更成功後に `InputNameChanged` が配信される

2. `C -> S` GetInputSettings

```json
{
  "op": 6,
  "d": {
    "requestType": "GetInputSettings",
    "requestId": "req-409",
    "requestData": {
      "inputName": "Main Camera Renamed"
    }
  }
}
```

3. `C -> S` GetInputDefaultSettings

```json
{
  "op": 6,
  "d": {
    "requestType": "GetInputDefaultSettings",
    "requestId": "req-410",
    "requestData": {
      "inputKind": "video_capture_device"
    }
  }
}
```

NOTE: `GetInputDefaultSettings` は現時点で `image_source` / `video_capture_device` を返す

---

## 例 8: RequestBatch で配信準備をまとめる

目的: 複数の準備 Request を 1 回で送る
hisui 対応状況: 部分対応（ `RequestBatch` `op=8/9` は `executionType = 0` のみ対応 ）

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
hisui 対応状況: 部分対応（ 認証付き `Identify` / `Reidentify` は対応済み。`StreamStateChanged` / `RecordStateChanged` / `CurrentProgramSceneChanged` / `SceneCreated` / `SceneRemoved` / `InputCreated` / `InputRemoved` を配信 ）

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

5. `S -> C` Identified (`op: 2`)

```json
{
  "op": 2,
  "d": {
    "negotiatedRpcVersion": 1
  }
}
```

NOTE: `authentication` 文字列は `protocol.md` の手順 ( SHA256 + Base64 ) で生成する

---

## 例 10: 失敗時レスポンスの扱い

目的: `requestStatus` の `code` / `comment` で分岐する
hisui 対応状況: 対応済み（ 未対応 request / 不正 requestData に対する `RequestResponse` エラーは実装済み ）

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

---

## 例 11: Scene Item 参照フロー（ 一覧 / ソース / インデックス ）

目的: `Scene Item` を参照して制御前の前提情報を取得する
hisui 対応状況: 対応済み（ `GetSceneItemList` / `GetSceneItemSource` / `GetSceneItemIndex` ）

1. `C -> S` GetSceneItemList

```json
{
  "op": 6,
  "d": {
    "requestType": "GetSceneItemList",
    "requestId": "req-601",
    "requestData": {
      "sceneName": "Scene"
    }
  }
}
```

2. `S -> C` RequestResponse ( 抜粋 )

```json
{
  "op": 7,
  "d": {
    "requestType": "GetSceneItemList",
    "requestId": "req-601",
    "requestStatus": { "result": true, "code": 100 },
    "responseData": {
      "sceneItems": [
        {
          "sceneItemId": 1,
          "sourceName": "image-1",
          "sourceType": "OBS_SOURCE_TYPE_INPUT",
          "sceneItemEnabled": true,
          "sceneItemIndex": 0
        }
      ]
    }
  }
}
```

3. `C -> S` GetSceneItemSource

```json
{
  "op": 6,
  "d": {
    "requestType": "GetSceneItemSource",
    "requestId": "req-602",
    "requestData": {
      "sceneName": "Scene",
      "sceneItemId": 1
    }
  }
}
```

4. `C -> S` GetSceneItemIndex

```json
{
  "op": 6,
  "d": {
    "requestType": "GetSceneItemIndex",
    "requestId": "req-603",
    "requestData": {
      "sceneName": "Scene",
      "sceneItemId": 1
    }
  }
}
```

---

## 例 12: Scene Item 作成 / 削除 / 複製

目的: `Scene` 内の `Scene Item` を追加・削除し、必要に応じて別 `Scene` へ複製する
hisui 対応状況: 対応済み（ `CreateSceneItem` / `RemoveSceneItem` / `DuplicateSceneItem` ）

1. `C -> S` CreateSceneItem

```json
{
  "op": 6,
  "d": {
    "requestType": "CreateSceneItem",
    "requestId": "req-701",
    "requestData": {
      "sceneName": "Scene",
      "sourceName": "image-1",
      "sceneItemEnabled": true
    }
  }
}
```

2. `S -> C` RequestResponse ( 抜粋 )

```json
{
  "op": 7,
  "d": {
    "requestType": "CreateSceneItem",
    "requestId": "req-701",
    "requestStatus": { "result": true, "code": 100 },
    "responseData": {
      "sceneItemId": 2
    }
  }
}
```

3. `C -> S` DuplicateSceneItem

```json
{
  "op": 6,
  "d": {
    "requestType": "DuplicateSceneItem",
    "requestId": "req-702",
    "requestData": {
      "fromSceneName": "Scene",
      "toSceneName": "Scene2",
      "sceneItemId": 2
    }
  }
}
```

4. `C -> S` RemoveSceneItem

```json
{
  "op": 6,
  "d": {
    "requestType": "RemoveSceneItem",
    "requestId": "req-703",
    "requestData": {
      "sceneName": "Scene",
      "sceneItemId": 2
    }
  }
}
```

NOTE: `CreateSceneItem` / `DuplicateSceneItem` の結果 `sceneItemId` は移動ではなく新規採番されるため、クライアントは戻り値を保持して後続操作に使う

---

## 例 13: Scene Item 並び替えとイベント受信

目的: `SetSceneItemIndex` で表示順序を変更し、`SCENES` 購読時のイベントを処理する
hisui 対応状況: 対応済み（ `SetSceneItemIndex` と `SceneItemListReindexed` ）

前提:

- `Identify` または `Reidentify` で `eventSubscriptions` に `OBSWS_EVENT_SUB_SCENES` を含める

1. `C -> S` SetSceneItemIndex

```json
{
  "op": 6,
  "d": {
    "requestType": "SetSceneItemIndex",
    "requestId": "req-801",
    "requestData": {
      "sceneName": "Scene",
      "sceneItemId": 5,
      "sceneItemIndex": 0
    }
  }
}
```

2. `S -> C` RequestResponse ( `op: 7` )
3. `S -> C` Event ( `op: 5`, `eventType = "SceneItemListReindexed"` )

NOTE: hisui では `SetSceneItemIndex` 成功時、`RequestResponse` の後に `SceneItemListReindexed` を送信する

---

## 例 14: Scene Item の lock / blend / transform 制御

目的: `Scene Item` のロック状態・合成モード・変形情報を更新する  
hisui 対応状況: 対応済み（ `Get/SetSceneItemLocked` / `Get/SetSceneItemBlendMode` / `Get/SetSceneItemTransform` ）

前提:

- `Identify` または `Reidentify` で `eventSubscriptions` に `OBSWS_EVENT_SUB_SCENES` を含めると、lock / transform 変更イベントを受信できる

1. `C -> S` GetSceneItemLocked

```json
{
  "op": 6,
  "d": {
    "requestType": "GetSceneItemLocked",
    "requestId": "req-901",
    "requestData": {
      "sceneName": "Scene",
      "sceneItemId": 5
    }
  }
}
```

2. `C -> S` SetSceneItemLocked

```json
{
  "op": 6,
  "d": {
    "requestType": "SetSceneItemLocked",
    "requestId": "req-902",
    "requestData": {
      "sceneName": "Scene",
      "sceneItemId": 5,
      "sceneItemLocked": true
    }
  }
}
```

3. `S -> C` Event ( `op: 5`, `eventType = "SceneItemLockStateChanged"` )

4. `C -> S` SetSceneItemBlendMode

```json
{
  "op": 6,
  "d": {
    "requestType": "SetSceneItemBlendMode",
    "requestId": "req-903",
    "requestData": {
      "sceneName": "Scene",
      "sceneItemId": 5,
      "sceneItemBlendMode": "OBS_BLEND_ADDITIVE"
    }
  }
}
```

5. `C -> S` SetSceneItemTransform

```json
{
  "op": 6,
  "d": {
    "requestType": "SetSceneItemTransform",
    "requestId": "req-904",
    "requestData": {
      "sceneName": "Scene",
      "sceneItemId": 5,
      "sceneItemTransform": {
        "positionX": 64.0,
        "positionY": 32.0,
        "boundsType": "OBS_BOUNDS_STRETCH"
      }
    }
  }
}
```

6. `S -> C` Event ( `op: 5`, `eventType = "SceneItemTransformChanged"` )

NOTE: `SetSceneItemTransform` はパッチ更新で、`sceneItemTransform` に含めたフィールドのみ更新する  
NOTE: hisui では現時点で `SetSceneItemBlendMode` に対応する専用イベントは送信しない
NOTE: `Get/SetSceneItemLocked` / `Get/SetSceneItemIndex` / `Get/SetSceneItemBlendMode` / `Get/SetSceneItemTransform` は現時点で状態保持と `Event` 配信のみ対応し、実際の映像出力には反映しない

---

## 例 15: Transition の取得と更新

目的: 現在の Transition 設定（ 種別 / 時間 ）を取得・更新する  
hisui 対応状況: 対応済み（ `GetTransitionKindList` / `GetSceneTransitionList` / `GetCurrentSceneTransition` / `SetCurrentSceneTransition` / `SetCurrentSceneTransitionDuration` / `GetCurrentSceneTransitionCursor` ）

1. `C -> S` GetTransitionKindList
2. `S -> C` RequestResponse（ `responseData.transitionKinds = ["Cut", "Fade"]` ）
3. `C -> S` SetCurrentSceneTransition（ `transitionName = "Fade"` ）
4. `S -> C` RequestResponse（ success ）
5. `C -> S` SetCurrentSceneTransitionDuration（ `transitionDuration = 500` ）
6. `S -> C` RequestResponse（ success ）
7. `C -> S` GetCurrentSceneTransition
8. `S -> C` RequestResponse（ `transitionName = "Fade"`, `transitionDuration = 500` ）
9. `C -> S` GetCurrentSceneTransitionCursor
10. `S -> C` RequestResponse（ `transitionCursor = 0.0` ）

NOTE: hisui の Transition は現時点で API の状態保持のみ対応し、実際の映像切り替え描画には反映しない  
NOTE: 現時点の対応遷移は `Cut` / `Fade` のみで、それ以外は not found エラーを返す  
NOTE: `SetCurrentSceneTransitionDuration.transitionDuration` は `50..=20000` のみ受理する  
NOTE: `GetCurrentSceneTransitionCursor.transitionCursor` は `0.0` 固定
