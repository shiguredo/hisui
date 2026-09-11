# RTP に対応する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-rtp
- Polished:

## 目的

汎用の RTP メディア入出力を追加する。現在 RTP は RTSP と WebRTC の内部でのみ扱われており、単体の RTP 入出力ができない。

## pending とした理由

- 単体の RTP 入出力をどう設計するか (UDP / TCP、RTCP、ペイロード種別、SRTP の扱い) の検討が必要。
- 対応するペイロード (H.264 / H.265 / AV1 / AAC / Opus など) の範囲を決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/rtsp/subscriber.rs` の `H264RtpDepacketizer` が RTSP over TCP interleaved の RTP を処理する。RTP パケット型は `shiguredo_rtsp` の `RtpPacket` を使う。
- WebRTC / Sora は `shiguredo_webrtc` が内部で RTP を処理し、hisui 側は `RtpReceiver` / `RtpTransceiver` を繋ぐだけ。
- `shiguredo_rtp` への依存は無く、`rtp://` / `udp://` のような単体 RTP 入出力は存在しない。

## 設計方針 (要検討・未確定)

- RTP/RTCP の送受信を共通ライブラリ化する。
- 入力ソースおよび出力として RTP を追加し、ペイロード種別を扱う。
- SRTP を対象に含めるか (WebRTC に任せるか) を決める。

## 完了条件

- RTP 経由のメディア入出力ができること。

(解決方法は pending のため、設計確定後に記載する。)
