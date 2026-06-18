# obsws / obsdc 系の JSON フィールド命名規則を envelope camel / settings ペイロード snake に統一する

- Priority: Medium
- Created: 2026-05-29
- Completed: 2026-06-18
- Model: Opus 4.7
- Branch: feature/change-obsws-json-naming
- Polished: 2026-06-15

## 目的

hisui の OBS WebSocket 互換サーバー (`src/obsws/` 配下) と、WebRTC DataChannel 経由で同種のメッセージを流す obsdc (DataChannel label `obsdc`、実装は `src/webrtc/p2p_session.rs`) 系のリクエスト・レスポンス JSON で、フィールド名の命名規則 (`camelCase` / `snake_case`) が箇所によってばらついている。

本対応では OBS WebSocket Protocol v5 と OBS Studio 本体が既に採用している階層構造を素直に踏襲し、hisui 独自フィールドの命名規則を以下の 3 階層で機械的に決まる形に揃える。

1. **envelope レイヤ**: Protocol 仕様で確定済みのキー、hisui 独自 Event / Request の `eventData` / `requestData` / `responseData` の引数群、envelope 境界キー (`inputSettings` / `outputSettings` / `streamServiceSettings` / `streamServiceType` / `subscriberSettings` / `soraSdkSettings`)。**camelCase** (Protocol 仕様で SCREAMING_SNAKE が指定されているものはそのまま)。
2. **settings ペイロード**: envelope 境界キーの中身、すなわち OBS Studio が `obs_data` として扱う領域、および hisui の各 settings 構造体・wrapper・variant・destination の `DisplayJson` 内のキー。**snake_case**。
3. **(例外) 外部プロトコル由来の受信データ**: Sora シグナリングなど、hisui が再構築せず外部 JSON をそのまま透過する箇所。外部仕様の表記をそのまま維持し、規約の判定対象外として明文化する。

hisui は正式リリース前のため、本対応は後方互換を維持しない破壊的変更として扱う。フォールバック読み込み・移行コードは入れない。

## 優先度根拠

- 命名規則のばらつきは外部 API のスペック揺れに直結し、UI 側 (devtools)・SDK 利用者・E2E テスト作者すべてに認知負荷を強いる。
- 一方、ばらつきがあっても現状の機能は動くため緊急度は High ではない。
- リネームは破壊的変更を含むが、hisui は正式リリース前のため互換維持コストは発生せず、コストはコード書き換えに閉じる。
- 早めに方針を固定しないと、新規 input kind / output kind / Event / Request 追加時に「どっちに合わせるべきか」の判断が都度発生し、規約の根拠が崩れていく。実例として、`bwtest` フィールドは追加時に「OBS 互換のデフォルト値」コメント付きで導入されたが、フェーズ A 調査で OBS 本体側にキー自体が存在しないことが判明し、根拠が崩れた。先送りはしない。

## 現状

### 階層構造と各レイヤ

OBS WebSocket Protocol v5 のレスポンスは以下の形を取る。

```
{ "op": 7, "d": { "requestType": "GetInputSettings", "responseData": {
    "inputName": "...",          ← envelope: camelCase (Protocol 仕様で確定)
    "inputSettings": {            ← envelope: camelCase (Protocol 仕様で確定)
      "device_id": "...",         ← settings ペイロード: snake_case (obs_data 文化)
      "pixel_format": "NV12"      ← 同上
    }
}}}
```

この階層構造は OBS Studio 本体・OBS WebSocket Protocol が公式に採用しているもので、hisui もこれを踏襲する。Protocol 仕様で SCREAMING_SNAKE が指定されているフィールド (`MAIN` / `ACTIVATE` / `MIX_AUDIO` / `SCENE_REF` / `EPHEMERAL`、`OBSWS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY` 等) もそのまま維持する。

settings ペイロード内のキーと envelope 内のキーで **同名のもの** (`videoTrackId` / `audioTrackId` / `trackId` / `signalingUrls` / `channelId` / `clientId` / `bundleId` 等) が存在する。判定基準は「envelope 境界キー (`inputSettings` / `outputSettings` / `streamServiceSettings` / `subscriberSettings` / `soraSdkSettings`) の中身として現れるか否か」で、対象 API 経路は `docs/obsws/json_naming.md` の章 4 settings ペイロードの出現箇所を真実とする。レビュー時に人間が同名キーの所在を出現箇所リストと照合して判別する。

### envelope レイヤ (camelCase 維持、変更しない)

- OBS Protocol 標準キー: `src/obsws/response/` 全般、`src/obsws/state/types.rs` の `ObswsInput*` / `ObswsScene*` / `ObswsSceneItemTransform` 等で `inputName` / `inputKind` / `inputUuid` / `sceneItemId` / `inputMuted` / `inputVolumeDb` / `inputVolumeMul` / `positionX` / `cropTop` / `boundsType` 等。
- hisui 独自 Event の `eventData` (`src/obsws/response/event.rs:549-653`): `SoraSourceTrackPublished` / `SoraSourceTrackUnpublished` / `SoraSubscriberDisconnected` / `SoraSubscriberNotify`。主なフィールドは `subscriberName` / `connectionId` / `clientId` / `trackKind` / `trackId` / `code` / `reason` / `notify`。
- hisui 独自 Request の `requestData` / `responseData`: `HisuiStartSoraSubscriber` / `HisuiStopSoraSubscriber` / `HisuiListSoraSubscribers` / `HisuiListSoraSourceTracks` / `HisuiAttachSoraSourceTrack` / `HisuiDetachSoraSourceTrack`。
- Sora subscriber list 応答 (`src/obsws/coordinator/output_sora.rs:861-907`): `subscriberName` / `active` / `settings` (子は settings ペイロード) / `connectionId` / `clientId` / `trackId` / `trackKind` / `attachedInputName`。
- envelope 境界キー: `inputSettings` / `outputSettings` / `streamServiceSettings` / `streamServiceType` / `subscriberSettings` / `soraSdkSettings`。
- `webrtc/p2p_session.rs` 経由の obsdc レスポンス: `SubscribeProgramTracks` 系の `videoTrackId` / `audioTrackId` / `trackId` (`src/webrtc/p2p_session.rs:1205-1206, 1740, 1832, 2011, 2206-2207`)。

これらは本 issue の **変更対象外** (camelCase 維持)。

### settings ペイロード (snake_case に揃える対象、本 issue のリネーム対象)

OBS Studio 本体は source / output plugin の settings を `obs_data` で扱い、内部は snake_case が事実上の標準。フェーズ A 調査で input 系 / stream service について裏付け済み (詳細は本 issue 末尾「OBS Studio キー定義調査結果」)。output 系 (HLS / DASH / RTMP outbound / Sora publisher) は hisui 独自拡張で OBS Studio に同名 plugin が無いが、本 issue の規約 (settings ペイロード内は snake) で統一する。

リネーム対象構造体と所属ファイルを以下に列挙する (フィールドの具体的なリネーム前後一覧は本 issue 末尾「リネーム対象の確定」を真とする)。

**input / source 系構造体** (`src/obsws/state/types.rs:948-1213`):
- `ObswsImageSourceSettings`, `ObswsColorSourceSettings`, `ObswsVideoCaptureDeviceSettings`, `ObswsAudioCaptureDeviceSettings`, `ObswsMp4FileSourceSettings`, `ObswsRtmpInboundSettings`, `ObswsSrtInboundSettings`, `ObswsRtspSubscriberSettings`, `ObswsWebRtcSourceSettings`, `ObswsSoraSourceInputSettings`, `ObswsSoraSubscriberSettings`

**output 系構造体**:
- `ObswsRtmpOutboundSettings` (`src/obsws/coordinator/output_rtmp.rs:192`, impl `:198`)
- `ObswsHlsSettings` (`src/obsws/coordinator/output_hls.rs:247`, impl `:273`)
- `HlsVariant` (`src/obsws/coordinator/output_hls.rs:31`, impl `:53`)
- `HlsDestination` (`src/obsws/coordinator/output_hls.rs`, impl `:125` 通常 / `:163` credentials 込み)
- `ObswsDashSettings` (`src/obsws/coordinator/output_dash.rs:211`, impl `:240`)
- `DashVariant` (`src/obsws/coordinator/output_dash.rs:28`, impl `:50`)
- `DashDestination` (`src/obsws/coordinator/output_dash.rs`, impl `:89` 通常 / `:140` 付近 credentials 込み)
- `ObswsSoraPublisherSettings` (`src/obsws/coordinator/output_sora.rs:72`, impl `:81`): envelope 境界キー `soraSdkSettings` を出し、その中に settings ペイロードを入れる構造
- `ObswsStreamServiceSettings` (`src/obsws/coordinator/output_stream.rs:221`, impl `:240`): envelope 境界キー `streamServiceType` / `streamServiceSettings` を出し、`streamServiceSettings` の中に `server` / `key` (既に snake) を入れる構造。`bwtest` / `use_auth` を概念として保持しない (これらは下記 `handle_get_stream_service_settings` 側でハードコード出力)。

**stream service settings 出力経路** (`src/obsws/coordinator/output_registry.rs`):
- `handle_get_stream_service_settings` (`:469-482`): `server` / `key` / `use_auth` (`:478`) / `bwtest` (`:473`) を出力する。`bwtest` は本 issue で削除する (`ObswsStreamServiceSettings` に概念が存在しないため、`output_stream.rs` 側との同期は不要)。`use_auth` は OBS rtmp-custom.c 互換のため snake のままハードコード出力を継続する。

**state file の wrapper / receiver** (`src/obsws/state_file.rs`):
- `SrtInboundSettingsWithPassphrase` (impl `:1025`): `ObswsSrtInboundSettings` に `passphrase` を加えて永続化用に書き出す。中身は snake 化対象。
- `WebRtcSourceSettingsWithoutTrackId` (impl `:1049`): `ObswsWebRtcSourceSettings` から `trackId` を除いて永続化用に書き出す。中身は snake 化対象。
- HLS / DASH の S3 destination receiver (`:864-895` 付近): `usePathStyle` / `accessKeyId` / `secretAccessKey` / `sessionToken` / `lifetimeDays` を `to_member` で読む。snake 化追従が必要。
- variant receiver (`:339-351, :512-524` 付近): `videoBitrate` / `audioBitrate` を `to_member` で読む。snake 化追従が必要。

**受信経路** (`src/obsws/state/types.rs:181-410` 周辺):
- `parse_optional_string_setting(input_settings, "...")` / `parse_optional_i32_setting(...)` / `parse_overlay_string_setting(...)` の第 2 引数として渡される設定キー名が主たる受信経路 (settings 側)。`to_member("...")` は envelope レイヤの受信用。
- obsdc 受信 (`src/webrtc/p2p_session.rs:1670-1680` 等): `InputSettingsChanged` イベント中の inputSettings ペイロードを `to_member("backgroundKeyColor")` 等で読む。settings 内側のためリネーム対象。

**hisui 内部サブシステム連携**:
- `src/rtmp/inbound_endpoint.rs` (`:228, :249, :275`): `f.member("inputUrl", ...)` / `to_member("inputUrl")` で `inputUrl` キーを使用。snake 化対象。
- `src/srt/inbound_endpoint.rs` (`:361, :396, :420`): 同上。
- `src/rtsp/subscriber.rs` (`:39, :57, :61`): 同上。加えてエラーメッセージ (`"inputUrl scheme must be rtsp or rtsps"` 等、`:83, :1149-1199, :1504` 付近) も `input_url` 表記に追従させる。
- `src/srt/inbound_endpoint.rs` のエラーメッセージ (`format!("invalid inputUrl: {e}")` 等) も同様に追従。

### 外部プロトコル由来 (snake_case、変更しない)

`src/obsws/coordinator/output_sora.rs:521-553` 付近の Sora シグナリングメッセージ受信: `event_type` (1 箇所)、`connection_id` (2 箇所、`connection.created` / `connection.destroyed`)、`client_id` (1 箇所、`connection.created`)。Sora 仕様準拠でそのまま維持。

### obsdc 系の扱い

`src/webrtc/p2p_session.rs:602-629` で `obsdc` DataChannel を作成し、obsws と同種のメッセージを WebRTC DataChannel 経由で送受信する。メッセージ本体は obsws と共有 (`src/obsws/message.rs`) しているため、本規約は obsdc にも自動適用される。

## 設計方針

### 規約

3 階層 + 例外で命名規則を決める。

1. **envelope レイヤ**: Protocol 仕様で確定済みのキー、hisui 独自 Event / Request の `eventData` / `requestData` / `responseData` の引数群、envelope 境界キー。**camelCase**。Protocol 仕様で SCREAMING_SNAKE が指定されているフィールドは SCREAMING_SNAKE のまま。
2. **settings ペイロード**: envelope 境界キーの中身 (各 settings 構造体・wrapper・variant・destination の `DisplayJson` 内)。**snake_case**。OBS Studio 本体の `obs_data` 文化と整合する。
3. **(例外) 外部プロトコル由来の受信データ**: Sora シグナリングなど。外部仕様の表記をそのまま維持し、規約の判定対象外として明文化する。

### 規約の運用

新規 input kind / output kind / Event / Request を追加する際は以下の 1 問で判断する。

> このフィールドは envelope 境界キー (`inputSettings` / `outputSettings` / `streamServiceSettings` / `subscriberSettings` / `soraSdkSettings`) の中身か?
> - Yes → snake_case (新規 API 経路を追加する場合は `docs/obsws/json_naming.md` の章 4 settings ペイロードの出現箇所に追記する)
> - No → camelCase (Protocol 仕様で SCREAMING_SNAKE が指定されていれば SCREAMING_SNAKE。`docs/obsws/json_naming.md` の章 3 envelope 例外 allow-list に必要に応じて追記する)

### 規約ドキュメント

`docs/obsws/json_naming.md` を新規作成する。章構成は 5 章:

1. 規約 (3 階層 + 例外)
2. 判定アルゴリズム (1 問)
3. envelope 例外 allow-list (拡張 Event / Request の引数群、envelope 境界キー一覧)
4. settings ペイロードの出現箇所 (snake_case を適用する API 経路リスト)
5. 非対称キー (受信のみ / 送信のみ。例: `passphrase` は state file 永続化では出すが GetInputSettings レスポンスでは隠す。`track_id` は GetInputSettings レスポンスでは出すが state file 永続化では除外する)

既存 `docs/obsws/PERSISTENT_DATA.md` / `STATE_FILE.md` 内に古い JSON サンプルがあればリネームに追従させ、相互参照リンクを `json_naming.md` 末尾に置く。

## 完了条件

- 規約に従い、`src/obsws/` / `src/webrtc/p2p_session.rs` / `src/rtmp/` / `src/srt/` / `src/rtsp/` 配下の settings ペイロード内のフィールドが snake_case に統一されていること。出力側 (`f.member`) と受信側 (`to_member` / `parse_optional_*_setting` / `parse_overlay_*_setting`) が一貫した命名で揃っていること。
- envelope レイヤ (OBS WebSocket Protocol 標準キー、hisui 独自 Event / Request の eventData / requestData / responseData、envelope 境界キー) が camelCase のまま維持されていること。
- 外部プロトコル由来のフィールド (Sora シグナリング受信) が変更されていないこと。
- 機械チェック: リネーム前後で以下の手動 grep を実行し、リネーム後にヒット 0 件 (envelope 用途で残る `signalingUrls` / `channelId` / `clientId` / `bundleId` / `trackId` / `videoTrackId` / `audioTrackId` 等を除く) であることを確認すること。envelope 用途のヒットは `docs/obsws/json_naming.md` の章 3 envelope 例外 allow-list と章 4 settings ペイロードの出現箇所を照合して人間が判別する。
  ```
  rg 'sampleRate|loopPlayback|inputUrl|streamName|streamId|backgroundKeyColor|backgroundKeyTolerance|outputUrl|videoBitrate|audioBitrate|usePathStyle|lifetimeDays|accessKeyId|secretAccessKey|sessionToken|segmentDuration|maxRetainedSegments|segmentFormat|videoCodec|audioCodec' src/ e2e-tests/ devtools/src/ testdata/
  ```
- 既存テスト (`cargo test`) と e2e テスト (`cd e2e-tests && uv run pytest obsws/`) がすべて成功すること。テストデータ内の JSON 期待値も更新されていること:
  - `testdata/` 配下の固定 JSON
  - `src/obsws/state/tests.rs` / `src/obsws/session/tests.rs` / `src/obsws/response/tests.rs` の固定 JSON 期待値
  - `src/obsws/state_file.rs` の `#[cfg(test)]` 内固定 JSON。`assert!(json_text.contains("trackId"))` 等のアサーションを `track_id` に追従させる。
  - `src/obsws/message.rs` 内のテストモジュール
  - `e2e-tests/obsws/` 配下の Python テスト全体 (`test_output.py` / `test_state_file.py` / `test_request_batch.py` / `helpers.py` / `conftest.py` / `test_input.py` 等。実装時に `ls e2e-tests/obsws/*.py` の全件を対象とする)
- state file 形式の扱い:
  - state file の envelope (`scenes` / `inputs` / `currentProgramScene` / `nextInputId` 等) は camel 維持。
  - state file 内部の wrapper 構造体 (`SrtInboundSettingsWithPassphrase` / `WebRtcSourceSettingsWithoutTrackId`) と HLS / DASH の S3 destination / variant 受信経路の中身を snake に倒す。
  - hisui は正式リリース前のため、フォールバック読み込みコードや移行ガイドは追加しない。既存の state file は破壊的変更扱いとする。
- 未リリース機能のため `CHANGES.md` の `## develop` セクションには新規 `[CHANGE]` エントリを追加しない。代わりに、既存 `[ADD]` エントリ内で本対応のリネーム対象キーを引用している箇所を snake_case 表記に追従させる (HLS / DASH エントリの `segmentDuration` / `maxRetainedSegments` / `videoCodec` / `audioCodec` / `lifetimeDays` / `segmentFormat` 等が該当)。規約ドキュメント (`docs/obsws/json_naming.md`) も内部開発者向けで CHANGES.md には載せない。
- `bwtest` フィールドが `GetStreamServiceSettings` 応答 (`src/obsws/coordinator/output_registry.rs:473`) から削除されていること。`use_auth` のハードコード出力 (`:478`) は OBS rtmp-custom.c 互換のため維持されていること。
- 規約ドキュメント `docs/obsws/json_naming.md` が新規作成され、章 3 envelope 例外 allow-list と章 4 settings ペイロードの出現箇所がリネーム後のキー名と整合していること。
- OBS Studio クライアントとの互換確認は本 issue の範囲外。obsws JSON 命名規約の整理が本 issue の責務であり、OBS Studio 本体は「外部 obs-websocket サーバーに接続するクライアント機能」を提供しないため hisui との直接疎通経路も存在しない。特定 plugin との `obs_data` キー対照表や疎通確認は別途実施する。

## 解決方法

### 実装ステップ

1. **issue ファイル名のリネーム**: `git mv issues/0003-feature-refactor-obsws-json-naming.md issues/0003-feature-change-obsws-json-naming.md`。実装ブランチ `feature/change-obsws-json-naming` への切り替えと同じコミットで実施。
2. **規約ドキュメントの先行作成**: `docs/obsws/json_naming.md` を新規作成 (5 章構成)。
3. **settings ペイロード内の snake_case リネーム (本対応の中核)**: 出力側 (`f.member`) と受信側 (`to_member` / `parse_optional_*_setting` / `parse_overlay_*_setting`) を同時に書き換える。対象は本 issue 末尾「リネーム対象の確定」を真とし、最終的な網羅性は完了条件の手動 grep で担保する。
4. **`bwtest` の削除**: `src/obsws/coordinator/output_registry.rs:473` の `f.member("bwtest", false)?;` を削除。`use_auth` のハードコード出力 (`:478`) は維持。
5. **state file の snake 化**: `SrtInboundSettingsWithPassphrase` (`state_file.rs:1025`) / `WebRtcSourceSettingsWithoutTrackId` (`state_file.rs:1049`) の中身、`state_file.rs:864-895` 付近の HLS / DASH S3 destination receiver、`state_file.rs:339-351, :512-524` 付近の variant receiver の中身を snake 化。envelope (scenes / inputs / currentProgramScene 等) は触らない。`#[cfg(test)]` 内のアサーションも追従。
6. **output_stream.rs の同期**: `ObswsStreamServiceSettings::fmt` (`output_stream.rs:240-258`) は `bwtest` / `use_auth` を概念として持たないため `bwtest` 削除対象外。`streamServiceSettings` 受信経路の中身 (server / key 等) は既に snake、追加作業なし。
7. **hisui 内部サブシステムの同期**: `src/rtmp/inbound_endpoint.rs` (`:228, :249, :275`)、`src/srt/inbound_endpoint.rs` (`:361, :396, :420`)、`src/rtsp/subscriber.rs` (`:39, :57, :61`) の `f.member` / `to_member` キー `inputUrl` を `input_url` に追従させる。各ファイルのエラーメッセージ (`"inputUrl scheme must be rtsp or rtsps"` 等) も snake 表記に追従させる。
8. **devtools 側の同期**: `devtools/src/components/obsdc/ObsDcSourcePanel.tsx` ほか `devtools/src/` 内の literal 文字列 (`settingsKey="..."` 等) を snake 化。TypeScript 側の型定義は実コード上存在しない (`inputSettings` は `Record<string, string>` として扱われている) ため型変更は不要。
9. **テストの更新**: 完了条件で列挙したテストファイル全体を網羅して更新する。
10. **CHANGES.md の追従**: `## develop` セクションの既存 `[ADD]` エントリ内で snake 化対象のキー名を引用している箇所を snake_case に書き換える。未リリース機能のため新規 `[CHANGE]` エントリは追加しない。

### コミット分割

shiguredo-git 規約 (`{SEQ} {TITLE}` 形式) に従う。リネーム差分は巨大化するが blame 汚染範囲を限定するため論理単位で分ける。

1. `0003 obsws の issue ファイル名と命名規則ドキュメントを準備する` (issue ファイル `git mv` + `docs/obsws/json_naming.md` 新規作成)
2. `0003 obsws / obsdc の settings ペイロード内のフィールドを snake_case に統一する` (`src/obsws/` / `src/webrtc/p2p_session.rs` / `src/rtmp/` / `src/srt/` / `src/rtsp/` / 各種テスト)
3. `0003 obsws の state file 永続化フォーマットを規約変更に追従させる` (`src/obsws/state_file.rs` の wrapper / receiver、テストデータ)
4. `0003 devtools の obsdc 関連 settings を snake_case 規約に揃える`
5. `0003 obsws GetStreamServiceSettings 応答から未使用の bwtest フィールドを削除する`
6. `0003 CHANGES.md 内の未リリース obsws エントリを snake_case 規約に追従させる`

差分が更に巨大化する場合はコミット 2 を input 系・output 系・state file・devtools に追加分割することを検討する。

### 留意事項

- hisui は正式リリース前のため、受信側 (`to_member` / `parse_optional_*_setting` 等) で旧名と新名の両方を試すフォールバックコードは入れない。state file の移行コードも入れない。
- 章 5 「非対称キー」の対象候補 (`docs/obsws/json_naming.md` 執筆時の参考):
  - `passphrase` (`srt_inbound`): state file 永続化では出力、GetInputSettings レスポンスでは隠す
  - `track_id` (`webrtc_source`): GetInputSettings レスポンスでは出力、state file 永続化では除外

## OBS Studio キー定義調査結果 (2026-06-15 追記、フェーズ A)

obs-studio リポジトリ (master ブランチ) の plugin 実コードを直接確認し、hisui の input / stream service 系の settings ペイロード内キーが OBS Studio 本体側で実際にどう定義されているかを照合した。output 系 (HLS / DASH / RTMP outbound / Sora publisher) は hisui 独自拡張で OBS Studio 本体に対応 plugin が無いため、本フェーズの調査対象外 (output 系のキーは本 issue の規約に従って snake で統一する)。

引用 URL は将来の master 浮動に備えて `docs/obsws/json_naming.md` 側で commit hash pinned 形式 (`https://github.com/obsproject/obs-studio/blob/<commit-hash>/...#L<行>`) に正規化する。本 issue 内は短縮表記で記録する。

### video_capture_device 系

OBS Studio 側で plugin (OS) ごとにキー名が大きく分かれており、hisui の 1 構造体で完全に対応するのは元から不可能。共通して言えるのは「全 plugin が snake_case 文化」という点のみ。

| hisui 側キー | mac-avcapture | linux-v4l2 | win-dshow | OBS 互換 |
| --- | --- | --- | --- | --- |
| `device_id` | `device` / `device_name` | `device_id` | `video_device_id` / `audio_device_id` | linux-v4l2 のみ成立 (疎通対象) |
| `pixel_format` | `input_format` + `video_range` + `color_space` | `pixelformat` | `video_format` | 名称不一致で不成立 (疎通範囲外) |
| `fps` | `frame_rate` | `framerate` | `frame_interval` | 名称不一致で不成立 (疎通範囲外) |

引用 (代表): `linux-v4l2/v4l2-input.c#L670` (`device_id`), `#L577` (`pixelformat`), `#L581` (`framerate`)。

### audio_capture_device 系

| hisui 側キー | OBS 側の有無 | OBS 互換 |
| --- | --- | --- |
| `device_id` | mac-audio.c / pulse-input.c / win-wasapi.cpp / alsa-input.c の 4 plugin 全てで `device_id` (snake_case) | 4 plugin で成立 (疎通対象) |
| `sampleRate` (→ `sample_rate`) | OBS 側にキーが存在しない (4 plugin で `sample_rate` / `samplerate` / `rate` 等の `obs_data_get_int` 呼び出しを検索したが 0 件)。OBS Studio はサンプリングレートをグローバル設定で決める仕様 | 不成立 (疎通範囲外、hisui 独自) |
| `channels` | 同上、OBS 側に存在しない | 不成立 (疎通範囲外、hisui 独自) |

引用 (代表): `mac-capture/mac-audio.c#L724` (`device_id`)。

### stream service settings

| hisui 側キー | OBS rtmp-services 側 | OBS 互換 |
| --- | --- | --- |
| `server` | rtmp-common.c / rtmp-custom.c で snake_case | 成立 (疎通対象) |
| `key` | 同上 | 成立 (疎通対象) |
| `use_auth` | rtmp-custom.c のみ snake_case | カスタム RTMP のみ成立 (疎通対象) |
| `bwtest` | OBS rtmp-services 配下に obs_data キーとして存在しない (rtmp-common.c / rtmp-custom.c / services.json を grep して 0 件) | 削除。経緯: commit `f82b2f44` で `use_auth` と同時に `// OBS 互換のデフォルト値を含める` コメント付きで追加されたが、現行 OBS 本体側にキーが無いため根拠が崩れた。リポジトリ全体で出力 1 箇所のみ (`output_registry.rs:473`)、内部消費者ゼロ |

引用 (代表): `rtmp-custom.c#L27` (`use_auth`)。

### リネーム対象の確定 (本 issue 内の真実)

settings ペイロード内で **実際に綴りが変わる (camel → snake) フィールド** を列挙する。それ以外は既に snake のため変化なし、または 1 単語のため綴り変化なし。最終的な網羅性は完了条件の手動 grep で担保する。

**input / source 系** (`src/obsws/state/types.rs`):

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
- `ObswsSoraSubscriberSettings.signalingUrls` → `signaling_urls`
- `ObswsSoraSubscriberSettings.channelId` → `channel_id`
- `ObswsSoraSubscriberSettings.clientId` → `client_id`
- `ObswsSoraSubscriberSettings.bundleId` → `bundle_id`

**output 系**:

- `ObswsRtmpOutboundSettings.outputUrl` → `output_url` (`output_rtmp.rs:202`)
- `ObswsRtmpOutboundSettings.streamName` → `stream_name` (`output_rtmp.rs:205`)
- `ObswsHlsSettings.segmentDuration` → `segment_duration` (`output_hls.rs:279`)
- `ObswsHlsSettings.maxRetainedSegments` → `max_retained_segments` (`output_hls.rs:280`)
- `ObswsHlsSettings.segmentFormat` → `segment_format` (`output_hls.rs:281`)
- `HlsVariant.videoBitrate` → `video_bitrate` (`output_hls.rs:56`)
- `HlsVariant.audioBitrate` → `audio_bitrate` (`output_hls.rs:57`)
- `HlsDestination::S3.usePathStyle` → `use_path_style` (`output_hls.rs:152, 190`)
- `HlsDestination::S3.lifetimeDays` → `lifetime_days` (`output_hls.rs:154, 203`)
- `HlsDestination::S3.accessKeyId` → `access_key_id` (`output_hls.rs:194`)
- `HlsDestination::S3.secretAccessKey` → `secret_access_key` (`output_hls.rs:195`)
- `HlsDestination::S3.sessionToken` → `session_token` (`output_hls.rs:197`)
- `ObswsDashSettings.segmentDuration` → `segment_duration` (`output_dash.rs:246`)
- `ObswsDashSettings.maxRetainedSegments` → `max_retained_segments` (`output_dash.rs:247`)
- `ObswsDashSettings.videoCodec` → `video_codec` (`output_dash.rs:257`)
- `ObswsDashSettings.audioCodec` → `audio_codec` (`output_dash.rs:258`)
- `DashVariant.videoBitrate` → `video_bitrate` (`output_dash.rs:53`)
- `DashVariant.audioBitrate` → `audio_bitrate` (`output_dash.rs:54`)
- `DashDestination::S3.usePathStyle` → `use_path_style` (`output_dash.rs:116, 154`)
- `DashDestination::S3.lifetimeDays` → `lifetime_days` (`output_dash.rs:118, 167`)
- `DashDestination::S3.accessKeyId` → `access_key_id` (`output_dash.rs:158`)
- `DashDestination::S3.secretAccessKey` → `secret_access_key` (`output_dash.rs:159`)
- `DashDestination::S3.sessionToken` → `session_token` (`output_dash.rs:161`)
- `ObswsSoraPublisherSettings.signalingUrls` → `signaling_urls` (`output_sora.rs:88`、envelope 境界キー `soraSdkSettings` の中身)
- `ObswsSoraPublisherSettings.channelId` → `channel_id` (`output_sora.rs:91`)
- `ObswsSoraPublisherSettings.clientId` → `client_id` (`output_sora.rs:94`)
- `ObswsSoraPublisherSettings.bundleId` → `bundle_id` (`output_sora.rs:97`)

**hisui 内部サブシステム連携**:

- `src/rtmp/inbound_endpoint.rs.inputUrl` → `input_url` (`:228, :249, :275`)
- `src/srt/inbound_endpoint.rs.inputUrl` → `input_url` (`:361, :396, :420`)
- `src/rtsp/subscriber.rs.inputUrl` → `input_url` (`:39, :57, :61`)

**1 単語のため綴り変化なし** (確認のみ、リネーム不要): `metadata` (`ObswsSoraSubscriberSettings.metadata` / `ObswsSoraPublisherSettings.metadata` の `soraSdkSettings` 内) / `bucket` / `prefix` / `region` / `endpoint` / `directory` / `width` / `height` / `destination` / `type` / `path` / `passphrase` / `channels` / `fps`。

**stream service**:

- (削除) `output_registry.rs:473` の `bwtest`
- (変更なし、snake 維持) `output_registry.rs:478` の `use_auth`、`output_registry.rs` 内の `server` / `key`

**state file の wrapper / receiver**:

- `SrtInboundSettingsWithPassphrase` (`state_file.rs:1025`): 中身は `ObswsSrtInboundSettings` 同等 (`inputUrl` → `input_url`, `streamId` → `stream_id`)。`passphrase` は綴り変化なし
- `WebRtcSourceSettingsWithoutTrackId` (`state_file.rs:1049`): 中身は `ObswsWebRtcSourceSettings` 同等で `trackId` を除外。残る `backgroundKeyColor` / `backgroundKeyTolerance` を snake にリネーム
- HLS / DASH の S3 destination receiver (`state_file.rs:864-895` 付近): `usePathStyle` / `lifetimeDays` / `accessKeyId` / `secretAccessKey` / `sessionToken` を snake にリネーム
- variant receiver (`state_file.rs:339-351, :512-524` 付近): `videoBitrate` / `audioBitrate` を snake にリネーム
- `streamServiceSettings` 受信経路 (`state_file.rs:161-228` 付近): 境界キー `streamServiceType` / `streamServiceSettings` は envelope 扱いで camel 維持。中身 (`server` / `key`) は既に snake、追加作業なし
