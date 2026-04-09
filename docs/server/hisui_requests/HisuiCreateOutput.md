# HisuiCreateOutput

output インスタンスを作成する。同じ outputKind で複数の output インスタンスを作成できる。

作成直後の output は停止状態であり、`StartOutput` で開始する必要がある。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `outputName` | string | 必須 | output インスタンスの識別名（一意） |
| `outputKind` | string | 必須 | output の種別 |
| `outputSettings` | object | - | 種別固有の設定。省略時はデフォルト値で初期化 |

### outputKind 一覧

| outputKind | 説明 |
|-----------|------|
| `rtmp_output` | RTMP 配信 |
| `mp4_output` | MP4 録画 |
| `hls_output` | HLS ライブ出力 |
| `mpeg_dash_output` | MPEG-DASH ライブ出力 |
| `rtmp_outbound_output` | RTMP 再配信 |
| `sora_webrtc_output` | Sora WebRTC Publisher |

### outputSettings の形式

outputSettings の形式は `SetOutputSettings` の `outputSettings` と同じ。

## ResponseData

なし。

## エラー条件

- 同名の output が既に存在: `RESOURCE_ALREADY_EXISTS` を返す
- 未知の `outputKind`: `INVALID_REQUEST_FIELD` を返す
- `outputName` が未設定: `MISSING_REQUEST_FIELD` を返す
- `outputKind` が未設定: `MISSING_REQUEST_FIELD` を返す

## 制約

- WebSocket / データチャネル両方で利用可能
- RequestBatch（op=8）に対応
