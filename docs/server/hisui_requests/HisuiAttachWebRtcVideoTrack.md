# HisuiAttachWebRtcVideoTrack

bootstrap セッション上の client 送信 video track を既存の `webrtc_source` input に接続する。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## RequestData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputName` | string | 必須 | 接続先の `webrtc_source` input 名 |
| `trackId` | string | 必須 | 接続する video track の ID |

## ResponseData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputName` | string | 成功時に必須 | 接続先の input 名 |
| `trackId` | string | 成功時に必須 | 接続した video track の ID |

## 制約

- `obsdc` データチャネル経由でのみ利用可能
- 単発 Request（op=6）でのみ使用可能。RequestBatch（op=8）には非対応

## エラー条件

- `webrtc_source` 以外の input kind への attach はエラー
- 存在しない `trackId`、非 video track はエラー
- 別 input に attach 済みの track はエラー

## 備考

- 既にその input に別 `trackId` が attach 済みなら差し替え可能。旧 track 側の接続は解除される
- `trackId` は client 側の renegotiation 完了後に `HisuiListWebRtcVideoTracks` で取得する
- WebRTC renegotiation 自体は別で、attach は既存 track の論理接続だけを行う
