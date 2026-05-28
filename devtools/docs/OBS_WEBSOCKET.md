# OBS WebSocket 5.x プロトコル仕様

hisui-devtools の OBS WebSocket クライアント実装で参照する仕様のまとめ。

## 接続

- デフォルトポート: 4455
- サブプロトコル: `obswebsocket.json` (JSON over text frames)
- 接続直後にサーバーから Hello (OpCode 0) が送信される

## メッセージ構造

```json
{
  "op": <OpCode>,
  "d": <Data>
}
```

## OpCode 一覧

| OpCode | 名前                 | 方向             |
| ------ | -------------------- | ---------------- |
| 0      | Hello                | server -> client |
| 1      | Identify             | client -> server |
| 2      | Identified           | server -> client |
| 3      | Reidentify           | client -> server |
| 5      | Event                | server -> client |
| 6      | Request              | client -> server |
| 7      | RequestResponse      | server -> client |
| 8      | RequestBatch         | client -> server |
| 9      | RequestBatchResponse | server -> client |

## 認証フロー

1. サーバーが Hello で `authentication.challenge` と `authentication.salt` を送信
2. クライアントが以下の手順で認証文字列を生成:
   - `base64_secret = base64(sha256(password + salt))`
   - `authentication = base64(sha256(base64_secret + challenge))`
3. クライアントが Identify で `authentication` を送信

## Hello (OpCode 0)

```json
{
  "obsStudioVersion": "30.2.2",
  "obsWebSocketVersion": "5.5.2",
  "rpcVersion": 1,
  "authentication": {
    "challenge": "<base64>",
    "salt": "<base64>"
  }
}
```

## Identify (OpCode 1)

```json
{
  "rpcVersion": 1,
  "authentication": "<base64 string>",
  "eventSubscriptions": 4095
}
```

## Identified (OpCode 2)

```json
{
  "negotiatedRpcVersion": 1
}
```

## Event (OpCode 5)

```json
{
  "eventType": "CurrentProgramSceneChanged",
  "eventIntent": 4,
  "eventData": { ... }
}
```

## Request (OpCode 6)

```json
{
  "requestType": "GetSceneList",
  "requestId": "<uuid>",
  "requestData": { ... }
}
```

## RequestResponse (OpCode 7)

```json
{
  "requestType": "GetSceneList",
  "requestId": "<uuid>",
  "requestStatus": {
    "result": true,
    "code": 100
  },
  "responseData": { ... }
}
```

## EventSubscription ビットマスク

| 名前                      | 値      |
| ------------------------- | ------- |
| None                      | 0       |
| General                   | 1 << 0  |
| Config                    | 1 << 1  |
| Scenes                    | 1 << 2  |
| Inputs                    | 1 << 3  |
| Transitions               | 1 << 4  |
| Filters                   | 1 << 5  |
| Outputs                   | 1 << 6  |
| SceneItems                | 1 << 7  |
| MediaInputs               | 1 << 8  |
| Vendors                   | 1 << 9  |
| Ui                        | 1 << 10 |
| Canvases                  | 1 << 11 |
| All                       | 4095    |
| InputVolumeMeters         | 1 << 16 |
| InputActiveStateChanged   | 1 << 17 |
| InputShowStateChanged     | 1 << 18 |
| SceneItemTransformChanged | 1 << 19 |

## WebSocketCloseCode

| コード | 名前                  |
| ------ | --------------------- |
| 4000   | UnknownReason         |
| 4002   | MessageDecodeError    |
| 4003   | MissingDataField      |
| 4004   | InvalidDataFieldType  |
| 4005   | InvalidDataFieldValue |
| 4006   | UnknownOpCode         |
| 4007   | NotIdentified         |
| 4008   | AlreadyIdentified     |
| 4009   | AuthenticationFailed  |
| 4010   | UnsupportedRpcVersion |
| 4011   | SessionInvalidated    |
| 4012   | UnsupportedFeature    |

## 主要リクエスト

### General

- GetVersion, GetStats

### Scenes

- GetSceneList, GetCurrentProgramScene, SetCurrentProgramScene
- GetCurrentPreviewScene, SetCurrentPreviewScene

### Inputs

- GetInputList, GetInputMute, SetInputMute, ToggleInputMute
- GetInputVolume, SetInputVolume

### Stream

- GetStreamStatus, ToggleStream, StartStream, StopStream

### Record

- GetRecordStatus, ToggleRecord, StartRecord, StopRecord
- ToggleRecordPause, PauseRecord, ResumeRecord

### Scene Items

- GetSceneItemList, GetSceneItemEnabled, SetSceneItemEnabled

### Outputs

- GetVirtualCamStatus, ToggleVirtualCam, StartVirtualCam, StopVirtualCam

### Ui

- GetStudioModeEnabled, SetStudioModeEnabled
