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
2. **settings ペイロード**: envelope 境界キーの中身、すなわち OBS Studio が `obs_data` として扱う領域、および hisui の各 settings 構造体・wrapper・variant・destination の `DisplayJson` 内のキー。**snake_case**。OBS Studio 本体の `obs_data` 文化と整合する。
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

> このフィールドは settings ペイロード構造体・wrapper・variant・destination の `DisplayJson` 内に書くか?
>
> - Yes → `snake_case` (本ドキュメント 4 章 settings ペイロード allow-list に該当構造体が含まれていなければ追記する)
> - No → `camelCase` (Protocol 仕様で SCREAMING_SNAKE が指定されていれば SCREAMING_SNAKE。本ドキュメント 3 章 envelope 例外 allow-list に必要に応じて追記する)

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

`src/obsws/response/event.rs:549-` 以降の拡張 Event:

- `SoraSourceTrackPublished` / `SoraSourceTrackUnpublished` / `SoraSubscriberDisconnected` / `SoraSubscriberNotify`
- 主なフィールド: `subscriberName` / `connectionId` / `clientId` / `trackKind` / `trackId` / `code` / `reason` / `notify`

### 3.4 hisui 独自 Request の `requestData` / `responseData` 引数群

- `HisuiStartSoraSubscriber` / `HisuiStopSoraSubscriber` / `HisuiListSoraSubscribers` / `HisuiListSoraSourceTracks` / `HisuiAttachSoraSourceTrack` / `HisuiDetachSoraSourceTrack`
- Sora subscriber list 応答 (`src/obsws/coordinator/output_sora.rs:861-907`): `subscriberName` / `active` / `settings` (子は settings ペイロード) / `connectionId` / `clientId` / `trackId` / `trackKind` / `attachedInputName`
- obsdc レスポンス (`src/webrtc/p2p_session.rs:1205-1206, 1740, 1832, 2011, 2206-2207`): `SubscribeProgramTracks` 系の `videoTrackId` / `audioTrackId` / `trackId`
- `HisuiStartSoraSubscriber` の `requestData` 直下 (`src/obsws/coordinator/output_sora.rs:632-720` の `handle_start_sora_subscriber`): `subscriberName` / `signalingUrls` / `channelId` / `clientId` / `bundleId` / `metadata` は envelope 引数として camelCase。`ObswsSoraSubscriberSettings::fmt` (state file 永続化) は settings ペイロード規約で snake_case を使うが、これは別レイヤなので独立して扱う。

## 4. settings ペイロード allow-list

settings ペイロードで snake_case 必須となる構造体一覧。新規 input kind / output kind を追加する際は本リストにも追記する。

### 4.1 input / source 系 (`src/obsws/state/types.rs`)

- `ObswsImageSourceSettings` / `ObswsColorSourceSettings`
- `ObswsVideoCaptureDeviceSettings` / `ObswsAudioCaptureDeviceSettings`
- `ObswsMp4FileSourceSettings`
- `ObswsRtmpInboundSettings` / `ObswsSrtInboundSettings` / `ObswsRtspSubscriberSettings`
- `ObswsWebRtcSourceSettings`
- `ObswsSoraSourceInputSettings` / `ObswsSoraSubscriberSettings`

### 4.2 output 系

- `ObswsRtmpOutboundSettings` (`src/obsws/coordinator/output_rtmp.rs`)
- `ObswsHlsSettings` / `HlsVariant` / `HlsDestination` (`src/obsws/coordinator/output_hls.rs`)
- `ObswsDashSettings` / `DashVariant` / `DashDestination` (`src/obsws/coordinator/output_dash.rs`)
- `ObswsSoraPublisherSettings` (`src/obsws/coordinator/output_sora.rs`)
- `ObswsStreamServiceSettings` (`src/obsws/coordinator/output_stream.rs`): envelope 境界キー `streamServiceType` / `streamServiceSettings` を出すラッパ。内側に `server` / `key` (既に snake) を持つ。`bwtest` / `use_auth` を概念として保持しない (これらは `handle_get_stream_service_settings` 側でハードコード出力)。

### 4.3 stream service settings 出力経路

- `src/obsws/coordinator/output_registry.rs` の `handle_get_stream_service_settings`: `server` / `key` / `use_auth` を出力する (`bwtest` 削除の経緯は章 5.3 参照)。

### 4.4 state file の wrapper / receiver (`src/obsws/state_file.rs`)

- `SrtInboundSettingsWithPassphrase`: `ObswsSrtInboundSettings` に `passphrase` を加えて永続化用に書き出す。
- `WebRtcSourceSettingsWithoutTrackId`: `ObswsWebRtcSourceSettings` から `track_id` を除いて永続化用に書き出す。
- HLS / DASH の S3 destination receiver (`state_file.rs:864-895` 付近): `use_path_style` / `access_key_id` / `secret_access_key` / `session_token` / `lifetime_days` を `to_member` で読む。
- variant receiver (`state_file.rs:339-351, :512-524` 付近): `video_bitrate` / `audio_bitrate` を `to_member` で読む。

### 4.5 受信経路 (`src/obsws/state/types.rs:181-410` 周辺)

settings 側の文字列キーを読む主たる関数:

- `parse_optional_string_setting(input_settings, "...")`
- `parse_optional_i32_setting(input_settings, "...")`
- `parse_overlay_string_setting(input_settings, "...")`

`to_member("...")` は envelope レイヤの受信用。settings 側受信ではない。

### 4.6 obsdc 経由の settings 受信 (`src/webrtc/p2p_session.rs`)

`InputSettingsChanged` イベント中の inputSettings ペイロードを `to_member("...")` で読む箇所がある (例: `:1670-1680` 付近の `background_key_color` 等)。settings 内側のため snake 規約。

### 4.7 hisui 内部サブシステム連携

obsws/obsdc から渡された settings を受け取る側。エラーメッセージや構造体の DisplayJson / 受信キーで `input_url` 系の表記を持つ。

- `src/rtmp/inbound_endpoint.rs`
- `src/srt/inbound_endpoint.rs`
- `src/rtsp/subscriber.rs`

## 5. OBS Studio キー定義対照表

OBS Studio リポジトリ ([https://github.com/obsproject/obs-studio](https://github.com/obsproject/obs-studio)) の plugin 実コードを直接確認した結果を以下に記す。output 系 (HLS / DASH / RTMP outbound / Sora publisher) は hisui 独自拡張で OBS Studio に対応 plugin が無いため、本表の対象外。

### 5.1 video_capture_device 系

OBS Studio 側で plugin (OS) ごとにキー名が大きく分かれており、hisui の 1 構造体で完全に対応するのは元から不可能。共通して言えるのは「全 plugin が snake_case 文化」という点のみ。

| hisui 側キー | mac-avcapture | linux-v4l2 | win-dshow | OBS 互換 |
| --- | --- | --- | --- | --- |
| `device_id` | `device` / `device_name` | `device_id` | `video_device_id` / `audio_device_id` | linux-v4l2 のみ成立 |
| `pixel_format` | `input_format` + `video_range` + `color_space` | `pixelformat` | `video_format` | 名称不一致で不成立 |
| `fps` | `frame_rate` | `framerate` | `frame_interval` | 名称不一致で不成立 |

### 5.2 audio_capture_device 系

| hisui 側キー | OBS 側の有無 | OBS 互換 |
| --- | --- | --- |
| `device_id` | mac-audio.c / pulse-input.c / win-wasapi.cpp / alsa-input.c の 4 plugin 全てで `device_id` (snake_case) | 4 plugin で成立 |
| `sample_rate` (旧 `sampleRate`) | OBS 側にキー存在せず (4 plugin で `sample_rate` / `samplerate` / `rate` 等の `obs_data_get_int` 呼び出し 0 件)。OBS Studio はサンプリングレートをグローバル設定で決める仕様 | hisui 独自、不成立 |
| `channels` | 同上、OBS 側に存在しない | hisui 独自、不成立 |

### 5.3 stream service settings

| hisui 側キー | OBS rtmp-services 側 | OBS 互換 |
| --- | --- | --- |
| `server` | rtmp-common.c / rtmp-custom.c で snake_case | 成立 |
| `key` | 同上 | 成立 |
| `use_auth` | rtmp-custom.c のみ snake_case | カスタム RTMP のみ成立 |
| ~~`bwtest`~~ | OBS rtmp-services 配下に obs_data キーとして存在せず。削除済み | (削除) |

## 6. 引用 URL

OBS Studio 本体の引用箇所。master が将来移動する可能性があるため、本ドキュメント執筆時に確認した master 最新 commit を基準とする。実装時に最新の commit hash で固定する。

調査基準: `https://github.com/obsproject/obs-studio` master ブランチ (2026-06-15 時点)。

- `plugins/linux-v4l2/v4l2-input.c#L670` (`device_id`), `#L577` (`pixelformat`), `#L581` (`framerate`)
- `plugins/mac-capture/mac-audio.c#L724` (`device_id`)
- `plugins/rtmp-services/rtmp-custom.c#L27` (`use_auth`)

## 7. 非対称キー

settings ペイロード内のキーのうち、受信のみ・送信のみといった非対称な扱いがあるものをここに集約する。命名規則自体は snake_case で揃うが、出現箇所のルールが規約だけからは読めないため明文化する。

### 7.1 `passphrase`

- 構造体: `ObswsSrtInboundSettings`
- 受信時: `parse_optional_string_setting(input_settings, "passphrase")` で読む。
- 送信時:
  - GetInputSettings レスポンス (`ObswsSrtInboundSettings::fmt`): セキュリティ上の理由により出力しない (`src/obsws/state/types.rs:1103` のコメント参照)。
  - state file 永続化 (`SrtInboundSettingsWithPassphrase::fmt`): 永続化対象のため出力する。
- 命名: `passphrase` は 1 単語のため snake / camel 表記揺れの影響を受けない。

### 7.2 `track_id` (旧 `trackId`)

- 構造体: `ObswsWebRtcSourceSettings`
- 受信時: `parse_optional_string_setting(input_settings, "track_id")` で読む。
- 送信時:
  - GetInputSettings レスポンス (`ObswsWebRtcSourceSettings::fmt`): 出力する。
  - state file 永続化 (`WebRtcSourceSettingsWithoutTrackId::fmt`): 永続化対象から除外する。

なお Sora source の `video_track_id` / `audio_track_id` も同様に Attach / Detach Request 側で制御するが、構造体上は単純な settings フィールドなので非対称扱いではない。
