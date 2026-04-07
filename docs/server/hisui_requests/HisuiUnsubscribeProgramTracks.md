# HisuiUnsubscribeProgramTracks

Program 合成結果トラックの購読を解除する。
解除後は renegotiation が発生し、Program の映像・音声トラックの配信が停止する。

## Request

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `requestId` | string | 必須 | Request ID |

## ResponseData

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `videoTrackId` | string | 成功時に必須 | Program の映像トラック ID（固定値: `program:mixed_video`） |
| `audioTrackId` | string | 成功時に必須 | Program の音声トラック ID（固定値: `program:mixed_audio`） |

## 制約

- `obsdc` データチャネル経由でのみ利用可能
- 単発 Request（op=6）でのみ使用可能。RequestBatch（op=8）には非対応

## 備考

- 未購読の場合は no-op で成功する（renegotiation は発生しない）
