# RTSP 出力に対応する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-rtsp-outbound
- Polished:

## 目的

RTSP の入力 (クライアントとしての受信) は対応済みだが、出力 (サーバーとしての配信、または ANNOUNCE / RECORD による受信) が無い。RTSP 出力に対応する。

## pending とした理由

- RTSP 出力は利用場面が限られ、入出力のどちらを追加するか (サーバー配信 / パブリッシュ受信) の検討が必要。
- ベンダー固有のカメラ探索プロトコルなど、入力側にも残る検討事項がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/rtsp/subscriber.rs` の `RtspSubscriber` が RTSP クライアントとして H.264 (Annex-B) と AAC (MPEG4-GENERIC) を受信する。
- 入力は `ObswsInputSettings::RtspSubscriber` (`rtsp_subscriber`) として登録されている。
- 出力側の実装は無く、`src/obsws/coordinator/output_registry.rs` の `OutputKind` にも RTSP 種別は無い。

## 設計方針 (要検討・未確定)

- RTSP サーバーとしてメディアを配信する出力、または ANNOUNCE / RECORD を受けて取り込む入力を追加する。
- `OutputKind` / `ObswsInputSettings` への追加方法を決める。
- 対応コーデックと認証方式を決める。

## 完了条件

- RTSP の出力 (または受信サーバー) が動作すること。

(解決方法は pending のため、設計確定後に記載する。)
