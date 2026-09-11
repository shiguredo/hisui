# RTMP を Enhanced RTMP に対応する

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/add-ertmp
- Polished:

## 目的

AV1 / H.265 の映像や Opus / FLAC の音声を RTMP で配信できるようにする。現在の RTMP は H.264 / AAC のみに対応している。

## pending とした理由

- Enhanced RTMP (ExVideoTagHeader / ExVideoTagBody、ExAudioTagHeader、FourCC、Multitrack、Metadata) の対応は依存クレート `shiguredo_rtmp` の拡張が前提となり、hisui 単独では完結しない。
- どの範囲 (FourCC、Multitrack、Metadata など) を対象にするかを決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `shiguredo_rtmp` の `VideoCodec` は AVC までで、FourCC による拡張表現が無い。
- `src/rtmp/frame.rs` の `RtmpOutgoingFrameHandler` は `shiguredo_rtmp::VideoCodec::Avc` 固定、`RtmpIncomingFrameHandler` は `AvcPacketType` 前提。
- `src/rtmp/outbound_endpoint.rs` の `handle_video_message` は H.264 / H.264 Annex-B のみ、`handle_audio_message` は AAC のみを許可する。
- RTMP のタイムスタンプは `shiguredo_rtmp` の `RtmpTimestamp(u32)` (ミリ秒) で扱われる。

## 設計方針 (要検討・未確定)

- `shiguredo_rtmp` に ExVideoTag / ExAudioTag と FourCC、必要な Multitrack を追加する。
- hisui の `RtmpIncomingFrameHandler` / `RtmpOutgoingFrameHandler` と各 endpoint を拡張コーデックに対応させる。
- H.265 / AV1 の sample_entry 表現は既存の sample_entry 変換経路を流用する。

## 完了条件

- AV1 / H.265 の映像と Opus / FLAC の音声を RTMP で送受信できること。
- 既存の H.264 / AAC の経路に回帰がないこと。

(解決方法は pending のため、設計確定後に記載する。)
