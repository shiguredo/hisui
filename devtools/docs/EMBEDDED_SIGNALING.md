# 組み込みシグナリング

## 概要

組み込みシグナリングは、WebRTC 接続の確立とシグナリングを 2 つのフェーズで実現する仕組みである。

1. **Bootstrap フェーズ**: HTTP POST で SDP offer/answer を交換し、WebRTC 接続を確立する
2. **DataChannel シグナリングフェーズ**: 確立した WebRTC 接続上の DataChannel (`label: "signaling"`) を使って再ネゴシエーションや切断を行う

WebSocket などの常時接続プロトコルを必要とせず、HTTP POST 1 回と DataChannel だけでシグナリングが完結する。

接続は 1:1 のみであり、サーバーがシグナリングの機能を保持する。メディアの方向は `a=sendrecv` の双方向マルチトラック構成であり、クライアントとサーバーのどちら側からでも複数のトラックを追加して送受信できる。

## 設計判断

### WebSocket を使わない理由

組み込みシグナリングは WebSocket を使わず、Bootstrap フェーズの HTTP POST 1 回だけで接続を確立する。再ネゴシエーションは WebRTC 接続上の DataChannel で行うため、別途シグナリングサーバーやシグナリング用の常時接続を維持する必要がない。

### サーバーが DataChannel を作成する理由

DataChannel はサーバー側が作成し、クライアントは `ondatachannel` イベントで受信する。Bootstrap フェーズではクライアントが offer を送りサーバーが answer を返すため、クライアントが WHIP の Offerer の役割を担う。DataChannel の作成をサーバー側に任せることで、サーバーが必要な DataChannel を柔軟に制御できる。

## 用語

- **クライアント**: Bootstrap フェーズで HTTP 経由で最初に SDP offer を送信する側
- **サーバー**: Bootstrap フェーズで SDP answer を返す側。シグナリングの機能を保持する

## アーキテクチャ

```
クライアント                                    サーバー
    |                                              |
    |  [Bootstrap フェーズ]                         |
    |  POST /bootstrap                             |
    |  Content-Type: application/sdp               |
    |  Body: SDP offer                             |
    |--------------------------------------------->|
    |                                              |
    |  201 Created                                 |
    |  Content-Type: application/sdp               |
    |  Body: SDP answer                            |
    |<---------------------------------------------|
    |                                              |
    |  [WebRTC 接続確立]                            |
    |  ICE + DTLS ハンドシェイク                     |
    |<============================================>|
    |                                              |
    |  [DataChannel シグナリングフェーズ]             |
    |  DataChannel label: "signaling" (サーバー作成) |
    |<============================================>|
    |                                              |
```

## Bootstrap フェーズ

### リクエスト

クライアントは SDP offer を HTTP POST でサーバーに送信する。

- **メソッド**: `POST`
- **URL**: Bootstrap URL (設定可能、デフォルト: `http://127.0.0.1:4455/bootstrap`)
- **Content-Type**: `application/sdp`
- **Body**: SDP offer 文字列

### レスポンス

#### 成功

- **ステータスコード**: `201 Created`
- **Content-Type**: `application/sdp`
- **Body**: SDP answer 文字列

#### 既にセッションが存在する場合

- **ステータスコード**: `409 Conflict`

1:1 接続のため、既にセッションが確立されている場合は新たな接続を受け付けない。

### ICE サーバー設定

Bootstrap 設定で `iceServers` を指定できる。STUN/TURN サーバーが必要な場合に使用する。

```typescript
type BootstrapConfig = {
  bootstrapUrl: string;
  iceServers?: readonly RTCIceServer[];
};
```

### SDP の構成

Bootstrap フェーズの SDP には `m=application` (DataChannel) のみを含める。`m=audio` や `m=video` といったメディアセクションは含めない。メディアトラックの追加は全て DataChannel 経由の再ネゴシエーションで行う。

SDP に `m=application` を含めるために `alwaysNegotiateDataChannels` オプションを使用する。これは Chrome 148 以降で利用できる。

```javascript
const pc = new RTCPeerConnection({
  alwaysNegotiateDataChannels: true,
});
const offer = await pc.createOffer();
// offer の SDP には m=application のみが含まれる
```

`alwaysNegotiateDataChannels` をサポートしないブラウザでは、フォールバックとして `createDataChannel("dummy")` を offer 生成前に呼び出して SDP に `m=application` を含める。

```javascript
const pc = new RTCPeerConnection();
// ダミーの DataChannel を作成して SDP に m=application を含める
pc.createDataChannel("dummy");
const offer = await pc.createOffer();
```

`alwaysNegotiateDataChannels` と異なり、この方法では不要な DataChannel が作成される。`alwaysNegotiateDataChannels` が利用可能かどうかは、`RTCPeerConnection` を作成した後に `getConfiguration()` でオプションの存在を確認する。

```javascript
function supportsAlwaysNegotiateDataChannels() {
  const pc = new RTCPeerConnection({
    alwaysNegotiateDataChannels: true,
  });
  const config = pc.getConfiguration();
  pc.close();
  return "alwaysNegotiateDataChannels" in config;
}
```

### 処理の流れ

1. `RTCPeerConnection` を作成する
2. `createOffer()` で SDP offer を生成する
3. `setLocalDescription(offer)` で offer をセットする
4. ICE candidate の収集を待つ (host candidate のみ、タイムアウト 100ms)
5. SDP offer を Bootstrap URL に POST する
6. レスポンスの SDP answer を `setRemoteDescription()` でセットする
7. WebRTC のハンドシェイク完了を待つ (タイムアウト 5000ms)

## DataChannel シグナリングフェーズ

WebRTC 接続が確立された後、サーバーが DataChannel (`label: "signaling"`) を作成する。クライアントは `ondatachannel` イベントでこの DataChannel を受信し、以降のシグナリングに使用する。

DataChannel 上のシグナリングプロトコルには JSON を採用する。全てのメッセージは JSON 文字列である。

### メッセージ一覧

#### offer メッセージ

再ネゴシエーション用の SDP offer。クライアントとサーバーのどちらからでも送信できる。

```json
{
  "type": "offer",
  "sdp": "<SDP 文字列>"
}
```

#### answer メッセージ

再ネゴシエーションに対する SDP answer。offer を受信した側が返す。

```json
{
  "type": "answer",
  "sdp": "<SDP 文字列>"
}
```

#### disconnect メッセージ

クライアントからの切断要求。クライアントのみが送信できる。

```json
{
  "type": "disconnect"
}
```

#### close メッセージ

サーバーからの接続終了通知。サーバーのみが送信できる。

```json
{
  "type": "close",
  "code": "<close コード>",
  "reason": "<理由>"
}
```

**close コード一覧**:

| コード         | 説明                                      |
| -------------- | ----------------------------------------- |
| `unknown-type` | 不明なメッセージタイプを受信した          |
| `timeout`      | タイムアウト                              |
| `sdp-error`    | SDP の処理でエラーが発生した              |
| `srd-error`    | `setRemoteDescription` でエラーが発生した |
| `unexpected`   | 予期しないエラー                          |
| `missing-sdp`  | SDP フィールドが欠落している              |

## 再ネゴシエーションと双方向マルチトラック

全てのメディアトランシーバーは `a=sendrecv` で確立される。クライアントとサーバーのどちら側からでも複数のトラックを追加・削除できる。映像や音声のトラック数に制限はなく、再ネゴシエーションによって動的にトラックを増減する。

再ネゴシエーションはクライアントとサーバーのどちらからでも開始できる。メディアトラックの追加や変更を行う側が DataChannel 経由で offer を送信し、相手側が answer を返す。

```
offer 送信側                            answer 送信側
    |                                      |
    |  offer メッセージ (DataChannel)       |
    |------------------------------------->|
    |                                      |
    |  setRemoteDescription(offer)         |
    |  createAnswer()                      |
    |  setLocalDescription(answer)         |
    |                                      |
    |  answer メッセージ (DataChannel)      |
    |<-------------------------------------|
    |                                      |
```

## 切断

セッションの切断はクライアントから `disconnect` メッセージを DataChannel 経由で送信することで行う。サーバーから切断を開始することはない。

1. クライアントが `disconnect` メッセージを送信する
2. サーバーがセッションを終了し `close` メッセージを返す
3. クライアントが `RTCPeerConnection` を閉じる

ページ離脱時は `beforeunload` イベントで `disconnect` メッセージを送信し、`RTCPeerConnection` を閉じる。

## 接続状態

クライアントの接続状態は以下の遷移を取る。

```
idle --> bootstrapping --> connecting --> connected --> disconnecting --> closed
                |                            |
                |                            +--> closed (close メッセージ受信)
                |                            |
                |                            +--> closed (接続断)
                |
                +--> closed (Bootstrap 失敗)
                |
                +--> closed (接続タイムアウト)
```

| 状態            | 説明                                      |
| --------------- | ----------------------------------------- |
| `idle`          | 初期状態                                  |
| `bootstrapping` | HTTP POST で SDP offer/answer を交換中    |
| `connecting`    | WebRTC ハンドシェイク中                   |
| `connected`     | WebRTC 接続確立済み、DataChannel 通信可能 |
| `disconnecting` | 切断処理中                                |
| `closed`        | 接続終了                                  |
