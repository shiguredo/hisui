# devtools を GPUI ベースのネイティブ GUI アプリとして実装する

- Created: 2026-08-03
- Completed:
- Branch: feature/gpui
- Polished:

## 目的

ブラウザ向けの devtools (Web 開発ツール) を、Rust + GPUI によるネイティブ GUI アプリとして実装する。ブラウザを起動せずに hisui server への P2P 接続・映像表示・OBS WebSocket 操作を可能にする。

ブラウザ版 devtools (`devtools/`) は廃止せず、既存のまま並行して提供する。

## 現状

- `devtools/` は Vite + Preact + TypeScript のブラウザアプリで、P2P ページ (`/`) と Debug ページ (`/debug`) を持つ。
- `devtools/src/p2p/client.ts` の `createP2PClient` が `RTCPeerConnection` を直接使用し、HTTP Bootstrap・シグナリング DataChannel・OBS WebSocket over DataChannel (obsdc) を管理している。
- 映像表示は `devtools/src/components/VideoDisplay.tsx` が `<video>` 要素を使用している。
- OBS WebSocket 直接接続は `devtools/src/obsdc/client.ts` が `WebSocket` API を使用している。
- プロトコル層 (`devtools/src/p2p/signaling.ts`、`devtools/src/obsdc/protocol.ts`、`devtools/src/obsdc/auth.ts`) は純粋なシリアライズ・パース処理であり、ブラウザ API に依存しないため Rust への移植が容易。
- hisui 本体は Rust 実装で、`shiguredo_webrtc` (libwebrtc バインディング) を `src/webrtc/` で使用している。同クレートはサーバー用途だけでなくクライアント用途の API (PeerConnectionFactory / PeerConnection / DataChannel / 映像デコーダ) も提供している。
- workspace 依存に `shiguredo_websocket` が存在し、WebSocket クライアントも利用できる。

## 設計方針

- ネイティブ GUI には GPUI (Zed の GPU アクセラレーション UI フレームワーク) を使用する。crates.io に公開済み (0.2.2) で、macOS / Linux / Windows に対応している。
- 配置は `devtools/gui/` を新規 workspace member として Rust バイナリを実装する。
- WebRTC は `shiguredo_webrtc` を使用し、`devtools/src/p2p/client.ts` の `createP2PClient` と同じセマンティクスでクライアントを実装する (createOffer → POST Bootstrap → setRemoteDescription(answer) → DataChannel の確立 → renegotiation 対応)。
- プロトコル層 (`signaling.ts` / `protocol.ts` / `auth.ts`) は Rust に移植し、既存のテスト相当を unittest + proptest で実装する。
- HTTP Bootstrap は `shiguredo_http11` または reqwest を使用する。
- OBS WebSocket 直接接続は `shiguredo_websocket` を使用する。
- 映像表示は shiguredo_webrtc のフレームコールバック (I420) を RGBA に変換し、GPUI の ImageSource にアップロードしてウィンドウ内に表示する。
- Stats 表示は `RTCRtpReceiver::get_stats` 相当の API を定期的に取得して表示する。

## 完了条件

- hisui server に対して HTTP Bootstrap からの P2P 接続が確立し、シグナリング DataChannel と obsdc DataChannel が open になる。
- 受信映像が GPUI ウィンドウ内に表示される。
- DataChannel 状態・Stats が表示され、OBS WebSocket over DataChannel のリクエスト送信とイベント表示ができる。
- プロトコル層に `signaling.ts` / `protocol.ts` / `auth.ts` のテスト相当を Rust (unittest + proptest) で実装する。
