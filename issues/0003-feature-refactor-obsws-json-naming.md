# obsws / obsdc 系の JSON フィールド命名規則を OBS の二層構造に揃える

- Priority: Medium
- Created: 2026-05-29
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-obsws-json-naming
- Polished: 2026-05-29

## 目的

hisui の OBS WebSocket 互換サーバー (`src/obsws/` 配下) と、WebRTC DataChannel 経由で同種のメッセージを流す obsdc (DataChannel label `obsdc`) 系のリクエスト・レスポンス JSON で、フィールド名の命名規則 (`camelCase` / `snake_case`) が箇所によってばらついている。

本対応では、OBS WebSocket Protocol v5 と OBS Studio 本体が既に採用している二層構造を素直に踏襲し、hisui 独自フィールドの命名規則をこの二層構造に揃える。具体的には:

- **envelope レイヤ** (Protocol 仕様で確定済みのキー、および hisui 独自 Event / Request の eventData / requestData / responseData の引数群): `camelCase`
- **settings ペイロード** (envelope 内の `inputSettings` / `outputSettings` / `streamServiceSettings` などの中身、すなわち OBS Studio が `obs_data` として扱う領域): `snake_case`
- **外部プロトコル由来の受信データ** (Sora シグナリングなど): 外部仕様の表記をそのまま維持

これにより、hisui 独自フィールドは envelope か settings ペイロードのどちらに属するかだけで命名規則が機械的に決まるようになる。

## 優先度根拠

- 命名規則のばらつきは外部 API のスペック揺れに直結し、UI 側 (devtools)・SDK 利用者・E2E テスト作者すべてに認知負荷を強いる。
- 一方、ばらつきがあっても現状の機能は動くため緊急度は High ではない。
- リネームは後方互換のない変更を含むが、hisui は正式リリース前のため互換維持コストは発生せず、コストはコード書き換えに閉じる。
- 早めに方針を固定しないと、新規エンドポイント・新規 input kind 追加時に「どっちに合わせるべきか」の判断が都度発生し、規約の根拠が崩れていく。実例として、`bwtest` フィールドは追加時に「OBS 互換のデフォルト値」というコメント付きで導入されたが、後述のフェーズ A 調査で OBS 本体側にキー自体が存在しないことが判明し、根拠が崩れている。先送りはしない。

## 現状

### OBS WebSocket Protocol v5 と OBS Studio 本体の二層構造

OBS WebSocket Protocol v5 のレスポンスは以下の形を取り、envelope と内部の `obs_data` ペイロードで命名規則が異なる。

```
{ "op": 7, "d": { "requestType": "GetInputSettings", "responseData": {
    "inputName": "...",          ← envelope: camelCase (Protocol 仕様で確定)
    "inputSettings": {            ← envelope: camelCase (Protocol 仕様で確定)
      "device_id": "...",         ← settings ペイロード: snake_case (obs_data 文化)
      "pixel_format": "NV12"      ← 同上
    }
}}}
```

この二層構造は OBS Studio 本体・OBS WebSocket Protocol が公式に採用しているもので、hisui もこれを踏襲する形が最も矛盾がない。

### envelope (camelCase 固定、変更しない)

`src/obsws/response/`, `src/obsws/state/types.rs` などのほとんどは OBS WebSocket Protocol v5 で命名が確定しており、camelCase で書かれている。例:

- `src/obsws/response/general.rs:109-126`: `obsVersion`, `obsWebSocketVersion`, `rpcVersion`, `supportedImageFormats`, `availableRequests` 等
- `src/obsws/response/input.rs:324, 429, 568-569`: `inputUuid`, `sceneItemId`, `inputMuted`, `inputVolumeDb`, `inputVolumeMul`
- `src/obsws/state/types.rs:75-81`: `inputName`, `inputKind`, `inputUuid`, `inputKindCaps`
- `src/obsws/state/types.rs:576-594`: scene item transform の `positionX`, `cropTop`, `boundsType` 等

これらは Protocol 仕様で文字列が確定しているため変更しない。

### envelope (SCREAMING_SNAKE_CASE、変更しない)

- `src/obsws/response/general.rs:295-299`: capability flag の `MAIN`, `ACTIVATE`, `MIX_AUDIO`, `SCENE_REF`, `EPHEMERAL`
- `src/obsws/protocol.rs` などの subscription / action 定数 (`OBSWS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY` 等)

これらは OBS Studio 仕様で大文字スネークが正なので対象外。

### hisui 独自 Event / Request の envelope (camelCase 維持、変更しない)

OBS WebSocket Protocol v5 にない、hisui が拡張した Event Type / Request Type の eventData / requestData / responseData は、OBS Protocol の Event / Request 引数群と並列に扱われる envelope の一部として位置づける。よって camelCase 維持。

- 拡張 Event (`src/obsws/response/event.rs:549-`): `SoraSourceTrackPublished`, `SoraSourceTrackUnpublished`, `SoraSubscriberDisconnected`, `SoraSubscriberNotify`。eventData の主なフィールドは `subscriberName`, `connectionId`, `clientId`, `trackKind`, `trackId` 等。
- 拡張 Request: `CreateSoraSubscriber`, `AttachSoraSubscriber`, `DetachSoraSubscriber`, `GetSoraSubscriberList`, `GetSoraSubscriberSettings` 等。requestData / responseData の主なフィールドは `signalingUrls`, `channelId`, `clientId`, `bundleId`, `metadata`, `soraSdkSettings` 等。

これらは既に camelCase で書かれており、本 issue の対象外。

### settings ペイロード (snake_case に揃える対象)

OBS Studio 本体は source plugin (`video_capture_device`, `audio_capture_device`, stream service など) の settings を `obs_data` で扱い、内部は snake_case が事実上の標準。フェーズ A の調査で以下が裏付けられた (調査の詳細は本 issue 末尾「OBS Studio キー定義調査結果」を参照)。

- video_capture_device 系: linux-v4l2 が `device_id` / `pixelformat` / `framerate`、win-dshow が `video_device_id` / `video_format` / `frame_interval`、mac-avcapture が `device` / `input_format` / `frame_rate` 等。すべて snake 文化。
- audio_capture_device 系: 4 plugin 全てで `device_id` (snake_case)。
- stream service 系 (rtmp-custom.c): `server` / `key` / `use_auth` / `username` / `password` がすべて snake_case。

しかし現状の hisui 側 settings 構造体では、OBS 互換のため snake で書かれているキー (`device_id`, `pixel_format`) と、hisui 独自で camel で書かれているキー (`sampleRate`, `loopPlayback`, `inputUrl`, `backgroundKeyColor` 等) が **同一構造体内で混在** しており、これが認知負荷の主因。本 issue ではこの settings 領域を全て snake_case に揃える。

リネームの主対象構造体 (`src/obsws/state/types.rs` 配下):

- `ObswsAudioCaptureDeviceSettings`: `sampleRate` → `sample_rate`
- `ObswsMp4FileSourceSettings`: `loopPlayback` → `loop_playback`
- `ObswsRtmpInboundSettings`: `inputUrl` → `input_url`, `streamName` → `stream_name`
- `ObswsSrtInboundSettings`: `inputUrl` → `input_url`, `streamId` → `stream_id`
- `ObswsRtspSubscriberSettings`: `inputUrl` → `input_url`
- `ObswsWebRtcSourceSettings`: `trackId` → `track_id`, `backgroundKeyColor` → `background_key_color`, `backgroundKeyTolerance` → `background_key_tolerance`
- `ObswsSoraSourceInputSettings`: `videoTrackId` → `video_track_id`, `audioTrackId` → `audio_track_id`
- stream service settings (`src/obsws/coordinator/output_registry.rs:478` 周辺): `use_auth` は snake のまま、`bwtest` は削除 (詳細後述)

既に snake_case で書かれているキー (`device_id`, `pixel_format`, `server`, `key`, `use_auth`) はそのまま維持。1 単語のキー (`fps`, `channels`, `path`, `passphrase`) は表記揺れの影響を受けないため綴り変化なし。

### 外部プロトコル由来 (snake_case、変更しない)

- `src/obsws/coordinator/output_sora.rs:522-550`: Sora シグナリングメッセージの `event_type`, `connection_id`, `client_id`

これは Sora 仕様準拠なので対象外。

### obsdc 系の扱い

- `src/webrtc/p2p_session.rs:612-627` で `obsdc` DataChannel を作成し、obsws と同種の OBS WebSocket メッセージを WebRTC DataChannel で送受信する。
- メッセージ本体は obsws と共有 (`src/obsws/message.rs`) しているため、JSON 命名規則は obsws と共通で扱う。本 issue で決めた規約は obsdc にも自動的に適用される。
- 「obsws / obsdc」と並列に書いているが、実装上は同じ JSON コードパスを通るため追加対応はない (規約適用範囲を明文化するのみ)。

## 設計方針

### 規約 (P 案)

以下の 2 階層 + 例外 1 つで命名規則を決める。

1. **envelope レイヤ**: Protocol 仕様で確定済みのキー、および hisui 独自 Event / Request の eventData / requestData / responseData の引数群を含む。**camelCase**。Protocol 仕様で SCREAMING_SNAKE が指定されているフィールド (`MAIN`, `MIX_AUDIO` 等) は仕様準拠で SCREAMING_SNAKE のまま。
2. **settings ペイロード**: envelope 内の `inputSettings` / `outputSettings` / `streamServiceSettings` の中身。**snake_case**。OBS Studio 本体の obs_data 文化と整合する。
3. **(例外) 外部プロトコル由来の受信データ**: Sora シグナリングなど。外部仕様の表記をそのまま維持し、規約の判定対象外として明文化する。

この規約により、hisui 独自フィールドは envelope か settings ペイロードのどちらに属するかだけで命名規則が機械的に決まる。

### 規約の運用

新規 input kind / output kind / Event / Request を追加する際は、以下の 1 問で判断する。

> このフィールドは `inputSettings` / `outputSettings` / `streamServiceSettings` の中身か?
> - Yes → snake_case
> - No → camelCase (Protocol 仕様で SCREAMING_SNAKE が指定されていれば SCREAMING_SNAKE)

### 監査スクリプト

新規追加時の判断ミスを防ぐため、`scripts/audit_obsws_field_naming.sh` を追加する。

- `rg 'f\.member\("[a-zA-Z_]+"' src/obsws` と `to_member` の全フィールド名を抽出する。
- 規約上 snake であるべきキー (settings ペイロード内) で camel が混ざっていないか、規約上 camel であるべきキー (envelope) で snake が混ざっていないかをチェックする。
- 未分類フィールドを検出した場合はエラー終了する。

CI への組み込みは本 issue のスコープ外として、別途検討する。

### 規約ドキュメント

判断アルゴリズムは `docs/obsws/json_naming.md` に明文化する。本 issue 内の「OBS Studio キー定義調査結果」の表を引き写し、引用 URL も残す。

## 完了条件

- 上記規約 (P 案) に従い、`src/obsws/` 配下の settings ペイロード内のフィールドが snake_case に統一されていること。出力側 (`f.member`) と受信側 (`to_member`) が一貫した命名で揃っていること。
- envelope レイヤ (OBS WebSocket Protocol 標準キー、hisui 独自 Event / Request の eventData / requestData / responseData) が camelCase のまま維持されていること。差分が settings ペイロード内に閉じていることが確認できること。
- 外部プロトコル由来のフィールド (Sora シグナリング) が変更されていないこと。
- 既存テスト (`cargo test`) がすべて成功すること。テストデータ内の JSON 期待値 (`testdata/` 配下の固定 JSON、`tests.rs` 内の期待値) も合わせて更新されていること。
- state file 形式の扱い:
  - state file の envelope (scenes / inputs / currentProgramScene / nextInputId 等) は camel 維持。
  - state file 内部の inputSettings / outputSettings の中身は snake に倒す。
  - hisui は正式リリース前のため、フォールバック読み込みコードや移行ガイドは追加しない。既存の state file は破壊的変更扱いとする。
- リネーム対象と削除対象を CHANGES.md の `## develop` に `[CHANGE]` として列挙する。
- `bwtest` フィールドが `GetStreamServiceSettings` 応答から削除されていること (フェーズ A 末尾で確定済み)。
- リネーム後、OBS Studio 本体クライアント (公式アプリ) を hisui server に繋げて、`video_capture_device` / `audio_capture_device` / stream service settings の往復が正しく動作することを確認する。確認結果は本 issue のコメントに記録する。
- 規約ドキュメント `docs/obsws/json_naming.md` が新規作成されていること。
- 監査スクリプト `scripts/audit_obsws_field_naming.sh` が追加され、ローカルから実行可能な状態であること。

## 解決方法

### 実装ステップ

1. **規約ドキュメントの先行作成**: `docs/obsws/json_naming.md` を新規作成し、P 案の規約と判断アルゴリズム、フェーズ A の調査結果表を載せる。これ以降の実装の参照源とする。
2. **監査スクリプトの先行作成**: `scripts/audit_obsws_field_naming.sh` を追加し、規約違反を検出するロジックを実装する。リネーム前に走らせて現状の違反を列挙し、リネーム後に走らせて違反ゼロを確認する。
3. **settings ペイロード内の snake_case リネーム**: 出力側 (`f.member`) と受信側 (`to_member`) を同時に書き換える。対象構造体:
   - `ObswsAudioCaptureDeviceSettings`: `sampleRate` → `sample_rate`
   - `ObswsMp4FileSourceSettings`: `loopPlayback` → `loop_playback`
   - `ObswsRtmpInboundSettings`: `inputUrl` → `input_url`, `streamName` → `stream_name`
   - `ObswsSrtInboundSettings`: `inputUrl` → `input_url`, `streamId` → `stream_id`
   - `ObswsRtspSubscriberSettings`: `inputUrl` → `input_url`
   - `ObswsWebRtcSourceSettings`: `trackId` → `track_id`, `backgroundKeyColor` → `background_key_color`, `backgroundKeyTolerance` → `background_key_tolerance`
   - `ObswsSoraSourceInputSettings`: `videoTrackId` → `video_track_id`, `audioTrackId` → `audio_track_id`
4. **`bwtest` の削除**: `src/obsws/coordinator/output_registry.rs:473` の `f.member("bwtest", false)?;` を削除する。
5. **state file の inputSettings / outputSettings 内部のみ snake 化**: `src/obsws/state_file.rs` で input/output settings の保存・読み込みパスが snake key を使うようにする。state file envelope (scenes / inputs / currentProgramScene 等) は触らない。
6. **devtools 側の同期**: `devtools/src/components/obsdc/ObsDcSourcePanel.tsx` ほか、devtools で settings フィールドを読み書きしている箇所を snake_case 化。TypeScript 側の型定義も合わせて更新する。
7. **テストの更新**: `src/obsws/{state,session,response}/tests.rs` の固定 JSON 期待値、`testdata/` 配下の固定 JSON を snake 化。e2e-tests 配下に同種の固定 JSON があれば併せて更新する。
8. **CHANGES.md の更新**: `## develop` セクションに以下を `[CHANGE]` として列挙する。
   - settings ペイロード内のフィールドを snake_case に統一 (具体的なリネーム前後を列挙)
   - `bwtest` フィールドを `GetStreamServiceSettings` 応答から削除
9. **OBS Studio 本体疎通テスト**: 公式 OBS Studio を立ち上げて hisui server に接続し、`video_capture_device` / `audio_capture_device` / stream service settings の Get / Set が想定通りに動くことを確認し、結果を本 issue のコメントに記録する。

### コミット分割

shiguredo-git 規約に従い、以下の単位でコミットを分割する。リネーム差分は巨大になるが blame の汚染範囲を 1 コミットに閉じることでレビューと将来の調査負荷を抑える。

1. `docs: obsws の JSON 命名規則を docs/obsws/json_naming.md に明文化する`
2. `feat: obsws フィールド命名規則の監査スクリプトを追加する`
3. `change: obsws settings ペイロード内のフィールドを snake_case に統一する` (リネームのみで他のロジック変更を混ぜない)
4. `change: obsws GetStreamServiceSettings 応答から未使用の bwtest フィールドを削除する`
5. (devtools 修正があれば) `change: devtools を obsws の snake_case 規約に揃える`

### 留意事項

- hisui は正式リリース前のため、受信側 (`message.rs` 等の `to_member`) で旧名と新名の両方を試すフォールバックコードは入れない。state file の移行コードも入れない。
- リネーム差分は巨大になるが、blame の汚染範囲を 1 コミットに閉じる。
- 規約根拠コメントは構造体単位で書き散らさず、必要に応じて `docs/obsws/json_naming.md` への参照を `src/obsws/state/types.rs` 冒頭に 1 行入れる程度に留める。混在問題は構造的に解消するため、コード内の補足コメントは不要。

## OBS Studio キー定義調査結果 (2026-06-15 追記、フェーズ A)

obs-studio リポジトリ (master) の plugin 実コードを直接確認し、hisui の `src/obsws/state/types.rs` と `src/obsws/coordinator/output_registry.rs` で snake_case のまま使っているキーが OBS Studio 本体側で実際にどう定義されているかを照合した。本調査の主目的は当初「リネーム対象を確定する」ことだったが、結果として OBS Studio 本体が `obs_data` ペイロードで snake_case を主体としていることが裏付けられ、本 issue の規約 (P 案: settings ペイロード内は snake) を採用する根拠の補強にもなった。

### video_capture_device 系

OBS Studio 側で plugin (OS) ごとにキー名が大きく分かれており、hisui の 1 構造体で完全に対応するのは元から不可能であることが判明。共通して言えるのは「全 plugin が snake_case 文化」という点のみ。

| hisui 側キー | mac-avcapture | linux-v4l2 | win-dshow | 規約適用 (P 案) |
| --- | --- | --- | --- | --- |
| `device_id` | `device` / `device_name` | `device_id` | `video_device_id` / `audio_device_id` | settings ペイロード内のため snake_case 維持。既に snake のため綴り変化なし。linux-v4l2 互換も担保 |
| `pixel_format` | `input_format` + `video_range` + `color_space` | `pixelformat` (アンダースコア無し) | `video_format` | settings ペイロード内のため snake_case 維持。既に snake のため綴り変化なし |
| `fps` | `frame_rate` | `framerate` | `frame_interval` | settings ペイロード内のため snake 規約適用。`fps` は 1 単語のため綴り変化なし |

引用 (代表): `linux-v4l2/v4l2-input.c#L670` (`device_id`), `#L577` (`pixelformat`), `#L581` (`framerate`)。

### audio_capture_device 系

| hisui 側キー | OBS 側の有無 | 規約適用 (P 案) |
| --- | --- | --- |
| `device_id` | mac-audio.c / pulse-input.c / win-wasapi.cpp / alsa-input.c の 4 plugin 全てで `device_id` (snake_case) を確認 | settings ペイロード内のため snake 維持。OBS Studio 互換も担保 |
| `sampleRate` | OBS 側にキーが存在しない (4 plugin で `sample_rate` / `samplerate` / `rate` 等の `obs_data_get_int` 呼び出しを検索したが 0 件)。OBS Studio はサンプリングレートをグローバル設定で決める仕様 | settings ペイロード内のため snake 規約適用 → `sample_rate` にリネーム |
| `channels` | 同上、OBS 側に存在しない | settings ペイロード内のため snake 規約適用。1 単語のため綴り変化なし |

引用 (代表): `mac-capture/mac-audio.c#L724` (`device_id`)。

### stream service settings

| hisui 側キー | OBS rtmp-services 側 | 規約適用 (P 案) |
| --- | --- | --- |
| `server` | rtmp-common.c / rtmp-custom.c で snake_case | settings ペイロード内 + OBS 互換のため snake 維持 |
| `key` | 同上 | 同上 |
| `use_auth` | rtmp-custom.c のみ snake_case で定義 (公式サービス側にはなし) | settings ペイロード内 + OBS (カスタム RTMP) 互換のため snake 維持 |
| `bwtest` | OBS rtmp-services 配下に obs_data キーとして存在しない (rtmp-common.c / rtmp-custom.c / services.json を `bwtest` 文字列で grep して 0 件)。由来は本調査では特定できなかった | 削除 (詳細は後述) |

引用 (代表): `rtmp-custom.c#L27` (`use_auth`)。

### リネーム対象の確定

settings ペイロード内で実際に綴りが変わる (camel → snake) のは以下のみ。それ以外は既に snake のため変化なし、または 1 単語のため綴り変化なし。

- `ObswsAudioCaptureDeviceSettings.sampleRate` → `sample_rate`
- `ObswsMp4FileSourceSettings.loopPlayback` → `loop_playback`
- `ObswsRtmpInboundSettings.inputUrl` → `input_url`
- `ObswsRtmpInboundSettings.streamName` → `stream_name`
- `ObswsSrtInboundSettings.inputUrl` → `input_url`
- `ObswsSrtInboundSettings.streamId` → `stream_id`
- `ObswsRtspSubscriberSettings.inputUrl` → `input_url`
- `ObswsWebRtcSourceSettings.trackId` → `track_id`
- `ObswsWebRtcSourceSettings.backgroundKeyColor` → `background_key_color`
- `ObswsWebRtcSourceSettings.backgroundKeyTolerance` → `background_key_tolerance`
- `ObswsSoraSourceInputSettings.videoTrackId` → `video_track_id`
- `ObswsSoraSourceInputSettings.audioTrackId` → `audio_track_id`
- (削除) `output_registry.rs:473` の `bwtest`

### 残課題: `bwtest`

#### 経緯調査の結果

- 追加 commit: `f82b2f44` (`feat: OBS WebSocket 互換性を向上させるフィールドと挙動を修正する`) で `use_auth` と同時に追加された。当時の直書きコメントは `// OBS 互換のデフォルト値を含める`。追加時の意図は OBS Studio 互換だった。
- 本フェーズ A の調査で OBS Studio 本体の rtmp-services 配下 (rtmp-common.c / rtmp-custom.c / services.json) に `bwtest` というキーは存在しないことが確定した。追加当時の互換性根拠は現状では裏付けられない。
- リポジトリ全体で `bwtest` が現れるのは `src/obsws/coordinator/output_registry.rs:473` の出力箇所 1 件のみ。e2e テスト (`e2e-tests/`) / devtools / 受信側 (`to_member`) / 永続化される state file のいずれにも存在せず、内部消費者ゼロ。

#### 結論

`GetStreamServiceSettings` 応答から `bwtest` フィールド自体を削除する。理由:

- 追加時の根拠 ("OBS 互換") が現行 OBS Studio で裏付けられない。
- 値が常に `false` 固定で、hisui の他のレイヤから設定する経路もない。
- 内部消費者がゼロのため、削除の影響範囲は「未知の外部クライアントが期待していた場合」のみ。hisui の OBS WebSocket 互換サーバーが想定する主たるクライアントは OBS Studio 本体 / その派生で、本体側はそもそもこのキーを読まないことが確定済み。

破壊的変更扱いとして CHANGES.md の `## develop` に `[CHANGE]` で 1 行記載する。

#### 対比: `use_auth` (削除しない)

同じ commit で追加された `use_auth` (同じく常時 `false` 出力) は OBS rtmp-custom.c で実在キー (`obs_data_get_bool(settings, "use_auth")`) として参照されているため削除しない。settings ペイロード内かつ OBS 互換のため snake_case 維持。
