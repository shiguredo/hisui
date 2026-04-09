# Output

hisui の output は OBS WebSocket の output 概念に基づいている。各 output は `outputName`（インスタンス名）と `outputKind`（種別）を持つ。

## 組み込み output

起動時に以下の output が自動作成される。これらは `HisuiRemoveOutput` で削除可能。

| outputName | outputKind | 説明 |
|-----------|------------|------|
| `stream` | `rtmp_output` | RTMP 配信（OBS 互換の主配信） |
| `record` | `mp4_output` | MP4 録画 |

## 利用可能な outputKind 一覧

| outputKind | 説明 |
|-----------|------|
| `rtmp_output` | RTMP 配信 |
| `mp4_output` | MP4 録画 |
| `hls_output` | HLS ライブ出力 |
| `mpeg_dash_output` | MPEG-DASH ライブ出力 |
| `rtmp_outbound_output` | RTMP 再配信 |
| `sora_webrtc_output` | Sora WebRTC Publisher |

## output の作成と削除

組み込み output 以外の output は `HisuiCreateOutput` で作成し、`HisuiRemoveOutput` で削除する。

- [HisuiCreateOutput](hisui_requests/HisuiCreateOutput.md): output インスタンスを作成する
- [HisuiRemoveOutput](hisui_requests/HisuiRemoveOutput.md): output インスタンスを削除する

同じ `outputKind` で複数の `outputName` を作成できる。

```json
{"requestType": "HisuiCreateOutput", "requestData": {"outputName": "my_hls", "outputKind": "hls_output"}}
{"requestType": "HisuiCreateOutput", "requestData": {"outputName": "my_hls_2", "outputKind": "hls_output"}}
```

## output の操作

作成した output は OBS WebSocket 互換の API で操作する。

- `SetOutputSettings(outputName, outputSettings)`: 設定を変更する
- `GetOutputSettings(outputName)`: 設定を取得する
- `StartOutput(outputName)`: 出力を開始する
- `StopOutput(outputName)`: 出力を停止する
- `ToggleOutput(outputName)`: 出力を切り替える
- `GetOutputStatus(outputName)`: 出力の状態を取得する
- `GetOutputList`: 全 output の一覧を取得する

## output の永続化

`--state-file` オプションを指定すると、output の一覧と設定が state file に保存される。再起動時に state file から output が復元される。

state file に output リストがない場合は、組み込み output（`stream` / `record`）のみがデフォルト設定で作成される。
