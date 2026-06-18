# obsws / obsdc JSON フィールド命名規則

obsws (OBS WebSocket 互換サーバー) および obsdc (WebRTC DataChannel 経由の同種メッセージ経路) のリクエスト・レスポンス JSON で用いるフィールド名の命名規則を定める。

OBS WebSocket Protocol v5 と OBS Studio 本体が公式に採用している階層構造 (Protocol envelope は camelCase、`obs_data` ペイロードは snake_case) を素直に踏襲する。

関連ドキュメント:

- [PERSISTENT_DATA.md](PERSISTENT_DATA.md): obsws の永続化対象の概要
- [STATE_FILE.md](STATE_FILE.md): state file のフォーマット詳細
- [PROTOCOL_STATUS.md](PROTOCOL_STATUS.md): OBS WebSocket Protocol 対応状況

## 1. 規約

3 階層 + 例外で命名規則を決める。

1. **envelope レイヤ**: OBS WebSocket Protocol v5 で確定済みのキー、hisui 独自 Event / Request の `eventData` / `requestData` / `responseData` の引数群、envelope 境界キー (`inputSettings` / `outputSettings` / `streamServiceSettings` / `streamServiceType` / `subscriberSettings` / `soraSdkSettings`)。**camelCase**。Protocol 仕様で SCREAMING_SNAKE が指定されているフィールド (capability flag の `MAIN` / `ACTIVATE` / `MIX_AUDIO` / `SCENE_REF` / `EPHEMERAL`、subscription / action 定数の `OBSWS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY` 等) は SCREAMING_SNAKE のまま。
2. **settings ペイロード**: envelope 境界キーの中身、すなわち OBS Studio が `obs_data` として扱う領域。**snake_case**。OBS Studio 本体の `obs_data` 文化と整合する。
3. **(例外) 外部プロトコル由来の受信データ**: Sora シグナリングなど、hisui が再構築せず外部 JSON をそのまま透過する箇所。外部仕様の表記をそのまま維持し、規約の判定対象外として明文化する。

レスポンスの形式例:

```
{ "op": 7, "d": { "requestType": "GetInputSettings", "responseData": {
    "inputName": "...",          ← envelope: camelCase (Protocol 仕様で確定)
    "inputSettings": {            ← envelope: camelCase (Protocol 仕様で確定)
      "device_id": "...",         ← settings ペイロード: snake_case (obs_data 文化)
      "pixel_format": "NV12"      ← 同上
    }
}}}
```

## 2. 判定アルゴリズム

新規 input kind / output kind / Event / Request を追加する際は、フィールドごとに以下の 1 問で判断する。

> このフィールドは envelope 境界キー (`inputSettings` / `outputSettings` / `streamServiceSettings` / `subscriberSettings` / `soraSdkSettings`) の中身か?
>
> - Yes → `snake_case` (settings ペイロード)
> - No → `camelCase` (envelope レイヤ。Protocol 仕様で SCREAMING_SNAKE が指定されていれば SCREAMING_SNAKE。本ドキュメント 3 章 envelope 例外 allow-list に必要に応じて追記する)

外部プロトコル (Sora シグナリングなど) から受け取った JSON を透過する場合は、そのプロトコルの仕様準拠で受信側を書き、本規約の対象外とする。

## 3. envelope 例外 allow-list

envelope レイヤで camelCase 維持の対象となる主なキー。新規 Event / Request を追加する際に envelope 引数が増える場合は本リストにも追記する。

### 3.1 Protocol envelope (固定キー)

OBS WebSocket Protocol v5 で命名が確定しているキー。

- `op` / `d` / `requestType` / `requestId` / `requestData` / `responseData` / `eventType` / `eventData` / `eventIntent`

### 3.2 envelope 境界キー

settings ペイロードを内包する境界として hisui が出力するキー。

- `inputSettings` / `outputSettings` / `streamServiceSettings` / `streamServiceType` / `subscriberSettings` / `soraSdkSettings`

### 3.3 hisui 独自 Event の `eventData` 引数群

- `SoraSourceTrackPublished` / `SoraSourceTrackUnpublished` / `SoraSubscriberDisconnected` / `SoraSubscriberNotify`
- 主なフィールド: `subscriberName` / `connectionId` / `clientId` / `trackKind` / `trackId` / `code` / `reason` / `notify`

### 3.4 hisui 独自 Request の `requestData` / `responseData` 引数群

- `HisuiStartSoraSubscriber` / `HisuiStopSoraSubscriber` / `HisuiListSoraSubscribers` / `HisuiListSoraSourceTracks` / `HisuiAttachSoraSourceTrack` / `HisuiDetachSoraSourceTrack`
- Sora subscriber list 応答の各エントリ: `subscriberName` / `active` / `settings` (子は settings ペイロード) / `connectionId` / `clientId` / `trackId` / `trackKind` / `attachedInputName`
- obsdc DataChannel 経由の `SubscribeProgramTracks` 系応答: `videoTrackId` / `audioTrackId` / `trackId`
- `HisuiStartSoraSubscriber` の `requestData` 直下: `subscriberName` / `signalingUrls` / `channelId` / `clientId` / `bundleId` / `metadata` は envelope 引数として camelCase。state file 永続化での同名情報は settings ペイロード規約で snake_case を使うが、これは別レイヤとして 4 章で扱う。

## 4. settings ペイロードの出現箇所

settings ペイロードは以下の API 経路の `inputSettings` / `outputSettings` / `streamServiceSettings` / `subscriberSettings` / `soraSdkSettings` の中身として現れる。すべてのキーは snake_case に従う。

- `GetInputSettings` / `SetInputSettings` の `inputSettings`
- `GetOutputSettings` / `SetOutputSettings` の `outputSettings` 内 (HLS / DASH / RTMP outbound / Sora publisher / Stream service の中身)
- `GetStreamServiceSettings` の `streamServiceSettings` (OBS Studio クライアント互換のため `use_auth: false` を常に含む)
- `HisuiCreateOutput` / `HisuiStartSoraSubscriber` 等の `subscriberSettings` / `soraSdkSettings`
- `InputSettingsChanged` イベントの `inputSettings` ペイロード (obsdc DataChannel 経由を含む)
- state file の `inputs[].inputSettings`、`stream.streamServiceSettings`、`rtmpOutbound` / `sora` / `hls` / `mpegDash` セクションの中身

## 5. 非対称キー

settings ペイロード内のキーで、出現箇所が API 経路によって異なるもの。命名規則自体は snake_case で揃うが、出現箇所のルールが規約だけからは読めないため明文化する。

### 5.1 `passphrase`

対象 input kind: `srt_inbound`

- 受信: `GetInputSettings` / `SetInputSettings` の `inputSettings.passphrase` で受け取る
- 送信: `GetInputSettings` レスポンスには含めない (セキュリティ理由)
- 永続化: state file には平文で保存する (state file 自体を信頼ローカルファイルとして扱う前提)

### 5.2 `track_id`

対象 input kind: `webrtc_source`

- 制御: `HisuiAttachWebRtcVideoTrack` / `HisuiDetachWebRtcVideoTrack` で attach / detach する
- 受信: `SetInputSettings` からは変更不可 (`CreateInput` 時は無視、`SetInputSettings` overlay でも対象外)
- 送信: `GetInputSettings` レスポンスには現在値を含める
- 永続化: state file には保存しない (runtime 管理)

### 5.3 `video_track_id` / `audio_track_id`

対象 input kind: `sora_source`

- 制御: `HisuiAttachSoraSourceTrack` / `HisuiDetachSoraSourceTrack` で attach / detach する
- 受信: `SetInputSettings` からは変更不可 (`CreateInput` 時は無視、`SetInputSettings` overlay でも対象外)
- 送信: `GetInputSettings` レスポンスには現在値を含める
- 永続化: state file には保存しない (runtime 管理)
