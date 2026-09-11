# SRT の未知ハンドシェイク種別を許容する

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/fix-srt-unknown-handshake-type
- Polished:

## 目的

SRT で外部サーバーに接続した際、未知の handshake type を受信すると `unknown handshake type` エラーで接続が即座に終了する。ハンドシェイクタイムアウトなどの他の異常系は warn で継続するため、扱いを揃える。

## pending とした理由

- 修正対象は依存クレート `shiguredo_srt` のハンドシェイクデコードであり、hisui 単独では完結しない。
- 未知 type を無視するのか、接続単位のエラーとして warn に留めるのか、プロトコル上の妥当性を含めて方針を決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/srt/inbound_endpoint.rs` の `SrtInboundEndpoint::run` は `SrtConnection::feed_recv_buf` のエラーを `?` で伝播し、endpoint を停止させる。
- `shiguredo_srt` の `HandshakePacket::decode` は `HandshakeType::from_u32` が `None` を返すと `Error::invalid_data("unknown handshake type: ...")` を返す。
- 一方、ハンドシェイクタイムアウトは `ConnectionEvent::Error` として通知され、`handle_connection_event` は warn ログのみで継続する。
- 拡張フィールドは未知 type を読み飛ばす実装で、未知 handshake type だけが即エラーになる非対称な扱いになっている。

## 設計方針 (要検討・未確定)

- 未知 handshake type を無視する、または接続単位の警告として扱い endpoint を継続させる。
- 拡張フィールドの未知 type と同じ扱いに揃える。
- 上記を `shiguredo_srt` 側の変更として反映し、hisui の `SrtInboundEndpoint` のエラー伝播を見直す。

## 完了条件

- 未知 handshake type を受信しても endpoint が即停止しないこと。
- `shiguredo_srt` に未知 type のテストが追加されること。

(解決方法は pending のため、設計確定後に記載する。)
