# OBS WebSocket 互換機能 実装状況

## 参照仕様

- プロトコル仕様: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>
- 対象仕様バージョン: OBS WebSocket 5.x（ hisui 側の現在値: `obsWebSocketVersion = 5.7.2`, `rpcVersion = 1` ）

## 準拠バージョン

- OBS Studio: 32.1.0
- OBS WebSocket: 5.7.2
- RPC Version: 1

## 目的

- `obs-websocket` 互換機能の実装状況を 1 つのファイルで管理する
- 対応済み / 未対応 / 対象外を明確にする
- 次に実装する機能の優先順位を明確にする

## ステータス定義

- `[x]`: 実装済み
- `[ ]`: 未対応

## 更新ルール

- `obsws` 関連の機能を追加・変更したら、このファイルを同一変更内で更新する
- `実装対象外` は必ず専用セクションに記載し、理由を `NOTE` で添える
- `NOTE` は必要な項目にのみ記載する（ 全項目に強制しない ）

## 非 Request 機能

- [x] WebSocket 接続（ `obswebsocket.json` 必須 ）: クライアント接続を受け付ける
  - NOTE: subprotocol が一致しない場合は handshake を拒否する
- [x] `Hello (op=0)`: サーバー能力と認証 challenge 情報を通知する
- [x] `Identify (op=1)`: クライアント識別と認証情報の受け取りを行う
- [x] `Identified (op=2)`: 識別完了を通知する
- [x] password 認証: challenge / salt ベースで `Identify.authentication` を検証する
  - NOTE: 認証失敗時は `4009` を返す
- [x] `Request / RequestResponse (op=6/7)` 基盤: Request を受けて同期応答を返す
  - NOTE: `Identify` 後のみ `Request` を受け付ける
- [x] `Identify` 検証: `rpcVersion` の必須チェックと対応範囲チェックを行う
- [x] メッセージ検証エラーの切断: 不正 payload や未対応 opcode は close する
- [x] `Reidentify (op=3)`: 既存セッションの再設定を受け付ける
  - NOTE: 成功時は `Identified (op=2)` を返す
  - NOTE: `eventSubscriptions` は保持し、対応済みイベントの配信判定に利用する
- [x] Event 配信（ `op=5` ）基盤: サーバーイベントを push 配信する
  - NOTE: 現在は `eventSubscriptions` の General / Outputs / Scenes / Inputs ビット購読時に対応イベントを配信する
- [x] `RequestBatch (op=8/9)`: 複数 Request のバッチ処理を行う
  - NOTE: 現時点で `executionType = 1 (SerialRealtime)` のみ対応し、`haltOnFailure` を反映する

## 対応すべきイベント一覧

- [x] `StreamStateChanged`: 配信出力状態の変化を通知する
- [x] `RecordStateChanged`: 録画出力状態の変化を通知する
- [x] `CurrentProgramSceneChanged`: 現在 Program Scene の変更を通知する
- [x] `SceneCreated`: Scene 作成を通知する
- [x] `SceneRemoved`: Scene 削除を通知する
- [x] `InputCreated`: Input 作成を通知する
- [x] `InputRemoved`: Input 削除を通知する
- [x] `InputSettingsChanged`: Input 設定変更を通知する
- [x] `InputNameChanged`: Input 名変更を通知する
- [x] `CustomEvent`: カスタムイベントを通知する
- [x] `SceneItemEnableStateChanged`: Scene Item の有効状態変更を通知する
- [x] `SceneItemLockStateChanged`: Scene Item のロック状態変更を通知する
- [x] `SceneItemTransformChanged`: Scene Item の変形状態変更を通知する
- [x] `SceneItemCreated`: Scene Item の作成を通知する
- [x] `SceneItemRemoved`: Scene Item の削除を通知する
- [x] `SceneItemListReindexed`: Scene Item の並び順変更を通知する

## RequestType 実装状況

### General

- [x] `GetVersion`: サーバーのバージョン情報と対応 Request 一覧を返す
  - [x] `availableRequests`: 対応 RequestType 一覧を返す
  - [x] `supportedImageFormats`: 対応画像フォーマット一覧を返す
- [x] `GetStats`: 実行統計情報を返す
  - [ ] `cpuUsage`: CPU 使用率を返す（ 現状は `0.0` 固定 ）
  - [x] `memoryUsage`: メモリ使用量を返す
    - NOTE: 現在プロセスの最大 RSS を MB 単位で返す
  - [x] `availableDiskSpace`: 空きディスク容量を返す
    - NOTE: 現在の録画ディレクトリが属するファイルシステムの空き容量を MB 単位で返す
  - [x] `activeFps`: 現在の FPS を返す
    - NOTE: 現在アクティブな stream または record 出力の総フレーム数と稼働時間から算出する
  - [ ] `averageFrameRenderTime`: 平均レンダー時間を返す（ 現状は `0.0` 固定 ）
  - [ ] `renderSkippedFrames`: レンダーでスキップしたフレーム数を返す（ 現状は `0` 固定 ）
  - [ ] `renderTotalFrames`: レンダー総フレーム数を返す（ 現状は `0` 固定 ）
  - [x] `outputSkippedFrames`: 出力でスキップしたフレーム数を返す
    - NOTE: 現在アクティブな stream / record 出力の keyframe 待機ドロップ数を合算して返す
  - [x] `outputTotalFrames`: 出力総フレーム数を返す
    - NOTE: 現在アクティブな stream / record 出力のフレーム数を合算して返す
  - [x] `webSocketSessionIncomingMessages`: 現在セッションの受信メッセージ数を返す
  - [x] `webSocketSessionOutgoingMessages`: 現在セッションの送信メッセージ数を返す
- [x] `BroadcastCustomEvent`: カスタムイベントを配信する
- [x] `Sleep`: 指定時間だけ処理を待機する
  - NOTE: `sleepMillis` は `0..=50000` のみ受理する
  - NOTE: OBS の仕様上、`Sleep` は `RequestBatch` 内でのみ利用できる。単体 Request として送った場合は `code = 206`（ `Not supported` ）になる

### Config

- [ ] `GetPersistentData`: 永続データを取得する
- [ ] `SetPersistentData`: 永続データを設定する
- [ ] `GetSceneCollectionList`: シーンコレクション一覧を取得する
- [ ] `SetCurrentSceneCollection`: 現在のシーンコレクションを切り替える
- [ ] `CreateSceneCollection`: シーンコレクションを作成する
- [ ] `GetProfileList`: プロファイル一覧を取得する
- [ ] `SetCurrentProfile`: 現在のプロファイルを切り替える
- [ ] `CreateProfile`: プロファイルを作成する
- [ ] `RemoveProfile`: プロファイルを削除する
- [ ] `GetProfileParameter`: プロファイルパラメータを取得する
- [ ] `SetProfileParameter`: プロファイルパラメータを設定する
- [ ] `GetVideoSettings`: 映像設定を取得する
- [ ] `SetVideoSettings`: 映像設定を更新する
- [x] `GetStreamServiceSettings`: 配信サービス設定を取得する
- [x] `SetStreamServiceSettings`: 配信サービス設定を更新する
  - NOTE: 現時点は `streamServiceType = "rtmp_custom"` 前提で `server` / `key` を保持する
- [x] `GetRecordDirectory`: 録画ディレクトリを取得する
- [x] `SetRecordDirectory`: 録画ディレクトリを設定する

### Sources

- [x] `GetSourceActive`: ソースのアクティブ状態を取得する
  - NOTE: 現在の Program Scene に有効な Scene Item として存在する場合に `videoActive = true` を返す
- [ ] `GetSourceScreenshot`: ソースのスクリーンショットを取得する
- [ ] `SaveSourceScreenshot`: ソースのスクリーンショットをファイル保存する

### Canvases

- [x] `GetCanvasList`: 利用可能なキャンバス一覧を返す

### Scenes

- [x] `GetSceneList`: シーン一覧を取得する
- [x] `GetGroupList`: グループ一覧を取得する
  - NOTE: 現時点では group 非対応のため空配列を返す
- [x] `GetCurrentProgramScene`: 現在の Program Scene を取得する
- [x] `SetCurrentProgramScene`: Program Scene を切り替える
- [x] `GetCurrentPreviewScene`: 現在の Preview Scene を取得する
- [x] `SetCurrentPreviewScene`: Preview Scene を切り替える
- [x] `CreateScene`: シーンを作成する
- [x] `RemoveScene`: シーンを削除する
  - NOTE: 最後の 1 Scene は削除不可
  - NOTE: 現在 Program / Preview Scene を削除した場合は残存 Scene へ自動切替する
- [x] `SetSceneName`: シーン名を変更する
  - NOTE: 現在 Program / Preview Scene を rename した場合は内部状態も同時に更新する
- [x] `GetSceneSceneTransitionOverride`: シーン遷移上書き設定を取得する
- [x] `SetSceneSceneTransitionOverride`: シーン遷移上書き設定を更新する
  - NOTE: `transitionName` / `transitionDuration` の state のみ保持し、実描画には反映しない
  - NOTE: `transitionName = null` かつ `transitionDuration = null` で override を解除する

### Inputs

- [x] `GetInputList`: 入力一覧を取得する
  - NOTE: `inputKindCaps` は現状 `0` 固定。OBS は `OBS_INPUT_CAP_AUDIO` / `OBS_INPUT_CAP_VIDEO` 等のビットフラグを返す（例: image_source = 32769）
- [x] `GetInputKindList`: 入力種別一覧を取得する
  - NOTE: hisui の inputKind 一覧は OBS と異なる。共通は `image_source` のみ。hisui 固有の種別は「hisui 独自拡張」セクション参照
- [ ] `GetSpecialInputs`: 特殊入力設定を取得する
- [x] `CreateInput`: 入力を作成する
  - NOTE: `sceneName` は既存 Scene のみ受理する（ `CreateScene` で追加可能 ）
  - NOTE: `inputKind` は `GetInputKindList` で返す値のみ受理する
  - NOTE: `sceneItemEnabled` の値に応じて Scene Item を作成し、`sceneItemEnabled` に反映する
  - NOTE: 成功時は `responseData.inputUuid` を返し、`GetInputSettings` で参照できる
- [x] `RemoveInput`: 入力を削除する
  - NOTE: `inputName` または `inputUuid` のいずれか指定で削除する
  - NOTE: 対象が存在しない場合は not found エラーを返す
- [x] `SetInputName`: 入力名を変更する
  - NOTE: `inputName` または `inputUuid` のいずれかで対象 Input を指定する
  - NOTE: 成功時は Inputs 購読中セッションへ `InputNameChanged` を配信する
- [x] `GetInputDefaultSettings`: 入力の既定設定を取得する
  - NOTE: 現在は `image_source` / `video_capture_device` / `audio_capture_device` / `mp4_file_source` の既定設定を返す
  - NOTE: hisui の `image_source` デフォルト設定は `{}` を返す。OBS は `linear_alpha` / `unload` 等のプロパティを含む
- [x] `GetInputSettings`: 入力設定を取得する
  - NOTE: OBS の `image_source` は `linear_alpha` / `unload` 等のプロパティを保持するが、hisui はこれらに非対応のため `overlay=true` でのマージ結果が異なる場合がある
- [x] `SetInputSettings`: 入力設定を更新する
  - NOTE: `overlay` 未指定時は `true` として扱う
  - NOTE: 成功時は Inputs 購読中セッションへ `InputSettingsChanged` を配信する
- [ ] `GetInputMute`: ミュート状態を取得する
- [ ] `SetInputMute`: ミュート状態を設定する
- [ ] `ToggleInputMute`: ミュート状態をトグルする
- [ ] `GetInputVolume`: 音量を取得する
- [ ] `SetInputVolume`: 音量を設定する
- [ ] `GetInputAudioBalance`: 音声バランスを取得する
- [ ] `SetInputAudioBalance`: 音声バランスを設定する
- [ ] `GetInputAudioSyncOffset`: 音声同期オフセットを取得する
- [ ] `SetInputAudioSyncOffset`: 音声同期オフセットを設定する
- [ ] `GetInputAudioMonitorType`: 音声モニター種別を取得する
- [ ] `SetInputAudioMonitorType`: 音声モニター種別を設定する
- [ ] `GetInputAudioTracks`: 音声トラック割当を取得する
- [ ] `SetInputAudioTracks`: 音声トラック割当を設定する
- [ ] `GetInputDeinterlaceMode`: デインターレースモードを取得する
- [ ] `SetInputDeinterlaceMode`: デインターレースモードを設定する
- [ ] `GetInputDeinterlaceFieldOrder`: デインターレースフィールド順を取得する
- [ ] `SetInputDeinterlaceFieldOrder`: デインターレースフィールド順を設定する

### Transitions

- [ ] 遷移種別に応じた実際の映像切り替え動作（ 例: `Fade` の補間描画 ）
- [ ] 遷移実行の時間進行（ 開始 / 進行 / 完了 ）に応じた出力制御
- [x] `GetTransitionKindList`: 遷移種別一覧を取得する
- [x] `GetSceneTransitionList`: 遷移一覧を取得する
- [x] `GetCurrentSceneTransition`: 現在の遷移情報を取得する
- [x] `SetCurrentSceneTransition`: 現在の遷移を設定する
- [x] `SetCurrentSceneTransitionDuration`: 遷移時間を設定する
- [x] `SetCurrentSceneTransitionSettings`: 遷移設定を更新する
- [x] `GetCurrentSceneTransitionCursor`: 遷移カーソル位置を取得する
- [x] `SetTBarPosition`: TBar 位置を設定する
  - NOTE: `Get/SetCurrentSceneTransition*` は API の状態保持として実装し、実描画は未対応
  - NOTE: 現時点の対応遷移は `Cut` / `Fade` のみ
  - NOTE: `transitionFixed` は `Cut=true` / `Fade=false` を返す
  - NOTE: `SetCurrentSceneTransitionDuration.transitionDuration` は `50..=20000` のみ受理する
  - NOTE: `SetCurrentSceneTransitionSettings.transitionSettings` は object のみ受理する
  - NOTE: `SetTBarPosition.position` は `0.0..=1.0` のみ受理する

### Filters

- [ ] `GetSourceFilterKindList`: フィルター種別一覧を取得する
- [ ] `GetSourceFilterList`: ソースのフィルター一覧を取得する
- [ ] `GetSourceFilterDefaultSettings`: フィルター既定設定を取得する
- [ ] `CreateSourceFilter`: ソースにフィルターを作成する
- [ ] `RemoveSourceFilter`: ソースからフィルターを削除する
- [ ] `SetSourceFilterName`: フィルター名を変更する
- [ ] `GetSourceFilter`: フィルター情報を取得する
- [ ] `SetSourceFilterIndex`: フィルター順序を設定する
- [ ] `SetSourceFilterSettings`: フィルター設定を更新する
- [ ] `SetSourceFilterEnabled`: フィルター有効状態を設定する

### Scene Items

- [x] `Scene Item` の実描画合成（ 複数 `Scene Item` の合成描画 ）
  - NOTE: `position` と `scale`（width/height）と `crop` に対応。`rotation` は未対応
- [x] `sceneItemIndex` の実描画順序への反映
- [ ] `sceneItemBlendMode` の実描画への反映
- [x] `sceneItemTransform.crop` の実描画への反映
  - NOTE: クロップはスケーリング前に適用する。I420 のクロマサブサンプリング制約のため各 crop 値は偶数に丸める
- [ ] `sceneItemTransform.rotation` の実描画への反映
- [x] `GetSceneItemList`: シーン内アイテム一覧を取得する
- [ ] `GetGroupSceneItemList`: グループ内アイテム一覧を取得する
- [x] `GetSceneItemId`: ソース名からシーンアイテム ID を取得する
  - NOTE: `searchOffset` は `0` のみ対応する
  - NOTE: `sceneItemId` の採番は全シーンで共有するグローバルカウンタを使用する。OBS はシーンごとのローカルカウンタで採番するため、同じ操作でも ID の値が異なる場合がある
- [x] `GetSceneItemSource`: シーンアイテムに紐づくソースを取得する
- [x] `CreateSceneItem`: シーンアイテムを作成する
- [x] `RemoveSceneItem`: シーンアイテムを削除する
- [x] `DuplicateSceneItem`: シーンアイテムを複製する
- [x] `GetSceneItemTransform`: シーンアイテム変形情報を取得する
- [x] `SetSceneItemTransform`: シーンアイテム変形情報を設定する
  - NOTE: `sceneItemTransform` はパッチ更新として扱い、指定フィールドのみ更新する
  - NOTE: `sourceWidth` / `sourceHeight` / `width` / `height` は更新対象外
- [x] `GetSceneItemEnabled`: シーンアイテム有効状態を取得する
- [x] `SetSceneItemEnabled`: シーンアイテム有効状態を設定する
- [x] `GetSceneItemLocked`: シーンアイテムロック状態を取得する
- [x] `SetSceneItemLocked`: シーンアイテムロック状態を設定する
- [x] `GetSceneItemIndex`: シーンアイテム順序を取得する
- [x] `SetSceneItemIndex`: シーンアイテム順序を設定する
- [x] `GetSceneItemBlendMode`: シーンアイテム合成モードを取得する
- [x] `SetSceneItemBlendMode`: シーンアイテム合成モードを設定する
  - NOTE: 現時点では blend mode 変更イベントは配信しない
  - NOTE: `Get/SetSceneItemLocked` / `Get/SetSceneItemBlendMode` は現時点で状態保持と `Event` 配信のみ対応し、実際の映像出力には反映しない
  - NOTE: `sceneItemTransform` の `rotation` および `sceneItemBlendMode` は状態保持のみで実映像出力には未反映
  - NOTE: `sceneItemTransform` の `crop` は実映像出力に反映済み

### Outputs

- [ ] `GetVirtualCamStatus`: Virtual Camera の状態を取得する
- [ ] `ToggleVirtualCam`: Virtual Camera をトグルする
- [ ] `StartVirtualCam`: Virtual Camera を開始する
- [ ] `StopVirtualCam`: Virtual Camera を停止する
- [ ] `GetReplayBufferStatus`: Replay Buffer の状態を取得する
- [ ] `ToggleReplayBuffer`: Replay Buffer をトグルする
- [ ] `StartReplayBuffer`: Replay Buffer を開始する
- [ ] `StopReplayBuffer`: Replay Buffer を停止する
- [ ] `SaveReplayBuffer`: Replay Buffer を保存する
- [ ] `GetLastReplayBufferReplay`: 最後の Replay Buffer ファイル情報を取得する
- [x] `GetOutputList`: 出力一覧を取得する
  - NOTE: `stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash` の 6 出力を返す
- [x] `GetOutputStatus`: 出力状態を取得する
  - NOTE: `outputName` は `stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash` を受理する
- [x] `ToggleOutput`: 出力をトグルする
  - NOTE: `outputName` は `stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash` を受理する
- [x] `StartOutput`: 出力を開始する
  - NOTE: `outputName` は `stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash` を受理する
- [x] `StopOutput`: 出力を停止する
  - NOTE: `outputName` は `stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash` を受理する
- [x] `GetOutputSettings`: 出力設定を取得する
  - NOTE: `outputName` は `stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash` を受理する
- [x] `SetOutputSettings`: 出力設定を更新する
  - NOTE: `outputName` は `stream` / `record` / `rtmp_outbound` / `sora` / `hls` / `mpeg_dash` を受理する
  - NOTE: `stream` は `streamServiceType` / `streamServiceSettings`、`record` は `recordDirectory` のみ更新する

### Stream

- [x] `GetStreamStatus`: 配信状態を取得する
  - [x] `outputActive`: 出力のアクティブ状態を返す
  - [x] `outputReconnecting`: 再接続状態を返す
  - [x] `outputTimecode`: 出力タイムコードを返す
  - [x] `outputDuration`: 出力継続時間を返す
  - [ ] `outputCongestion`: 出力混雑度を返す（ 現状は `0.0` 固定 ）
  - [x] `outputBytes`: 出力バイト数を返す
    - NOTE: RTMP outbound endpoint の送信バイト数を返す
  - [x] `outputSkippedFrames`: 出力スキップフレーム数を返す
    - NOTE: 接続直後の keyframe 待機中に drop した映像フレーム数を返す
  - [x] `outputTotalFrames`: 出力総フレーム数を返す
    - NOTE: stream encoder の `total_output_video_frame_count` を返す
  - NOTE: `outputCongestion` は引き続き固定値
- [x] `ToggleStream`: 配信をトグルする
  - NOTE: 現在状態に応じて `StartStream` または `StopStream` 相当の処理を内部で実行する
  - NOTE: 成功時の `responseData` には `outputActive` を返す
- [x] `StartStream`: 配信を開始する
  - NOTE: 複数映像入力に対応（`position` と `scale` と `crop` に対応。`rotation`, `blend mode` は未対応）
  - NOTE: 現時点の入力対応は `image_source` / `video_capture_device` / `audio_capture_device` / `mp4_file_source`
  - NOTE: 内部では `createPngFileSource` -> `createVideoEncoder` -> `createRtmpOutboundEndpoint` を起動する
  - NOTE: 複数映像入力時は `createVideoMixer` を追加で起動する
  - NOTE: 成功時の `responseData` には `outputActive = true` を返す
- [x] `StopStream`: 配信を停止する
  - NOTE: 内部で起動した stream 用 processor を停止する
- [ ] `SendStreamCaption`: 配信キャプションを送信する

### Record

- [x] `GetRecordStatus`: 録画状態を取得する
  - [x] `outputActive`: 録画出力のアクティブ状態を返す
  - [ ] `outputPaused`: 録画一時停止状態を返す
    - NOTE: 対応保留セクション参照
  - [x] `outputTimecode`: 録画タイムコードを返す
  - [x] `outputDuration`: 録画継続時間を返す
  - [x] `outputBytes`: 出力バイト数を返す
    - NOTE: 現在の録画ファイルサイズを返す
  - [x] `outputSkippedFrames`: 出力スキップフレーム数を返す
    - NOTE: keyframe 待機中に drop した映像フレーム数を返す
  - [x] `outputTotalFrames`: 出力総フレーム数を返す
    - NOTE: MP4 writer の `total_video_sample_count` を返す
  - [x] `outputPath`: 録画ファイルパスを返す
- [x] `ToggleRecord`: 録画をトグルする
  - NOTE: 現在状態に応じて `StartRecord` または `StopRecord` 相当の処理を内部で実行する
  - NOTE: 成功時の `responseData` には `outputActive` を返す
- [x] `StartRecord`: 録画を開始する
  - NOTE: 複数映像入力に対応（`position` と `scale` と `crop` に対応。`rotation`, `blend mode` は未対応）
  - NOTE: 現時点の入力対応は `image_source` / `video_capture_device` / `audio_capture_device` / `mp4_file_source`
  - NOTE: 内部では `createPngFileSource` -> `createVideoEncoder` -> `createMp4Writer` を起動する
  - NOTE: 複数映像入力時は `createVideoMixer` を追加で起動する
  - NOTE: 成功時の `responseData` には `outputActive = true` を返す
- [x] `StopRecord`: 録画を停止する
  - NOTE: 内部で起動した record 用 processor を停止する
- [ ] `ToggleRecordPause`: 録画一時停止をトグルする
  - NOTE: 対応保留セクション参照
- [ ] `PauseRecord`: 録画を一時停止する
  - NOTE: 対応保留セクション参照
- [ ] `ResumeRecord`: 録画を再開する
  - NOTE: 対応保留セクション参照
- [ ] `SplitRecordFile`: 録画ファイルを分割する
- [ ] `CreateRecordChapter`: 録画チャプターを作成する
- [ ] 配信 / 録画の encoder 共有構成
- [x] 配信 / 録画の encoder 非共有構成（ 配信用・録画用で別 encoder を生成 ）
- NOTE: encoder 共有 / 非共有の識別は obsws の request / event だけでは直接判断しにくいため、検証時は設定値・ログ・メトリクスを併用する

### Media Inputs

- [ ] `GetMediaInputStatus`: メディア入力状態を取得する
- [ ] `SetMediaInputCursor`: メディア入力カーソル位置を設定する
- [ ] `OffsetMediaInputCursor`: メディア入力カーソル位置を相対移動する
- [ ] `TriggerMediaInputAction`: メディア入力アクションを実行する

## 対応保留

実装済みだったが、OBS 側の問題により正常動作を確認できないため一時的に実装を削除して保留としている機能。
OBS 側の問題が解消された時点で再実装を検討する。

- `ToggleRecordPause` / `PauseRecord` / `ResumeRecord`: 録画の一時停止・再開
  - OBS 32.1.0 + macOS で PauseRecord は成功するが ResumeRecord が code=503 (OutputNotRunning) で失敗する
  - mkv / hybrid_mov 両フォーマットで再現し、OBS 単体実行でも同一の挙動
  - OBS 側の内部状態遷移の問題と判断し、hisui 側の実装を一旦削除

## 実装対象外

- MessagePack: WebSocket の MessagePack サブプロトコル対応
  - NOTE: 現状は `obswebsocket.json` のみを対象とする
- SceneItemSelected: Scene Item の選択状態変更を通知する
  - NOTE: OBS の GUI 操作（ユーザーがアイテムをクリックして選択）に紐づくイベントであり、hisui は headless サーバーのため選択概念が存在しない
- UI / Studio Mode 依存機能
  - `GetHotkeyList`: ホットキー一覧を取得する
  - `TriggerHotkeyByName`: 名前指定でホットキーを発火する
  - `TriggerHotkeyByKeySequence`: キーシーケンス指定でホットキーを発火する
  - `GetStudioModeEnabled`: Studio Mode の有効状態を取得する
  - `SetStudioModeEnabled`: Studio Mode の有効状態を設定する
  - `OpenInputPropertiesDialog`: 入力プロパティダイアログを開く
  - `OpenInputFiltersDialog`: 入力フィルターダイアログを開く
  - `OpenInputInteractDialog`: 入力インタラクトダイアログを開く
  - `GetMonitorList`: モニター一覧を取得する
  - `OpenVideoMixProjector`: 映像ミックスのプロジェクターを開く
  - `OpenSourceProjector`: ソースプロジェクターを開く
  - `TriggerStudioModeTransition`: Studio Mode の遷移を実行する
  - NOTE: OBS 本体の GUI 状態（ Studio Mode / Dialog / Projector ）、ホットキー設定、および OS の入力 / ディスプレイ統合に依存するため、hisui の現行アーキテクチャでは対応対象外とする
- OBS source properties 依存機能
  - `GetInputPropertiesListPropertyItems`: 入力プロパティのリスト型項目を取得する
  - `PressInputPropertiesButton`: 入力プロパティのボタンを押下する
  - NOTE: OBS source properties の動的定義と UI 操作に依存し、hisui の現行 input モデルでは自然に表現できないため、対応対象外とする
- vendor 拡張 request
  - `CallVendorRequest`: ベンダー拡張リクエストを実行する
  - NOTE: hisui では plugin / vendor namespace を導入する前提を取らないため、対応対象外とする
- OBS 実測で確認した Request
  - `GetSceneItemPrivateSettings`: Scene Item の private settings を取得する
  - `SetSceneItemPrivateSettings`: Scene Item の private settings を設定する
  - `GetSourcePrivateSettings`: Source の private settings を取得する
  - `SetSourcePrivateSettings`: Source の private settings を設定する
  - NOTE: これらは `protocol.md` には記載がないが、OBS 32.1.0 の `GetVersion.availableRequests` 実測で確認した。OBS 内部メタデータに依存し、hisui の現行モデルでは対応対象外とする

## 未対応 Request の扱い

- [x] 未対応 `requestType` は `RequestResponse` でエラー応答する
- [x] エラー内容は `Unknown request type` を返す

## hisui 独自拡張

OBS WebSocket 仕様には存在しない、hisui 固有の拡張機能。

### 独自 Input Kind

以下の `inputKind` は OBS WebSocket 仕様には存在しないが、hisui が独自に追加した入力種別である。
`CreateInput` / `GetInputKindList` 等の既存 Request で使用できる。

#### `rtmp_inbound`

RTMP インバウンドエンドポイント。指定した URL でリッスンし、外部から push された RTMP ストリームを映像・音声ソースとして取り込む。

**inputSettings:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputUrl` | string | 録画・配信開始時に必須 | リッスン URL（例: `rtmp://127.0.0.1:1935`） |
| `streamName` | string | - | RTMP ストリーム名 |

- NOTE: 映像・音声の両トラックを常に出力する。受信ストリームに含まれないトラックのデコーダーはアイドル状態になる

#### `srt_inbound`

SRT インバウンドエンドポイント。指定した URL でリッスンし、外部から push された SRT ストリームを映像・音声ソースとして取り込む。

**inputSettings:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputUrl` | string | 録画・配信開始時に必須 | リッスン URL（例: `srt://127.0.0.1:9000`） |
| `streamId` | string | - | SRT ストリーム ID（指定時は caller の streamid と照合する） |
| `passphrase` | string | - | SRT 暗号化パスフレーズ |

- NOTE: 映像・音声の両トラックを常に出力する。受信ストリームに含まれないトラックのデコーダーはアイドル状態になる

#### `rtsp_subscriber`

RTSP サブスクライバー。指定した RTSP URL に接続し、映像・音声ソースとして取り込む。

**inputSettings:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `inputUrl` | string | 録画・配信開始時に必須 | RTSP URL（例: `rtsp://127.0.0.1:554/stream`） |

- NOTE: 映像・音声の両トラックを常に出力する。受信ストリームに含まれないトラックのデコーダーはアイドル状態になる

### 独自 Output

以下の `outputName` は OBS WebSocket 仕様には存在しないが、hisui が独自に追加した出力種別である。
`StartOutput` / `StopOutput` / `ToggleOutput` / `GetOutputStatus` / `GetOutputSettings` / `SetOutputSettings` で使用できる。

#### `rtmp_outbound`

RTMP アウトバウンドエンドポイント。指定した URL でリッスンし、外部クライアントからの RTMP pull 接続を受け付けて映像・音声を配信する。

**outputSettings（`SetOutputSettings` / `GetOutputSettings`）:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `outputUrl` | string | StartOutput 時に必須 | リッスン URL（例: `rtmp://127.0.0.1:1936/live`） |
| `streamName` | string | - | RTMP ストリーム名 |

**フロー:**

1. `SetOutputSettings` で `outputUrl` 等を設定
2. `StartOutput` で pipeline 生成 + リスナー開始
3. `GetOutputStatus` で稼働状態確認
4. `StopOutput` で pipeline 停止

#### `sora`

Sora WebRTC Publisher。sora-rust-sdk を使い、Program 出力を Sora に SendOnly で配信する。

- `outputKind`: `sora_webrtc_output`
- SendOnly 限定（受信は行わない）
- エンコードは sora-rust-sdk 側で行う（Program 出力の raw フレームを直接供給）

**outputSettings（`SetOutputSettings` / `GetOutputSettings`）:**

outputSettings は `soraSdkSettings` オブジェクトを含む。

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `soraSdkSettings.signalingUrls` | string[] | StartOutput 時に必須（1 件以上） | シグナリング URL 一覧 |
| `soraSdkSettings.channelId` | string | StartOutput 時に必須 | チャネル ID |
| `soraSdkSettings.clientId` | string | - | クライアント ID |
| `soraSdkSettings.bundleId` | string | - | バンドル ID |
| `soraSdkSettings.metadata` | object | - | Sora に送信するメタデータ（JSON object のみ） |

**フロー:**

1. `SetOutputSettings` で `soraSdkSettings` を設定
2. `StartOutput` で Sora に SendOnly 接続を開始
3. `GetOutputStatus` で稼働状態確認
4. `StopOutput` で Sora 接続を切断

**エラー条件:**

- `signalingUrls` が空、または `channelId` が未設定の場合: `StartOutput` が失敗
- `metadata` が JSON object 以外の場合: `SetOutputSettings` が失敗
- 二重開始: `StartOutput` が `OUTPUT_RUNNING` エラーを返す
- 未起動停止: `StopOutput` が `OUTPUT_NOT_RUNNING` エラーを返す

#### `hls`

HLS ライブ出力。H.264 + AAC の MPEG-TS または fragmented MP4 セグメントを生成し、M3U8 プレイリストで管理する。
`variants` に複数のバリアントを指定すると adaptive bitrate (ABR) 出力に対応する。

- `outputKind`: `hls_output`
- ローカルファイルシステムまたは S3 互換オブジェクトストレージへの出力に対応
- 停止時に生成ファイル（playlist.m3u8 + 全セグメント）を自動削除する
- `outputBytes` / `outputSkippedFrames` / `outputTotalFrames` は現時点では 0 固定
- `variants` が 1 要素の場合は non-ABR（出力先直下にセグメントとプレイリストを出力）
- `variants` が 2 要素以上の場合は ABR（出力先直下にマスタープレイリスト、`variant_N/` サブディレクトリ/prefix にバリアントごとのセグメントとプレイリストを出力）

**outputSettings（`SetOutputSettings` / `GetOutputSettings`）:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `destination` | object | StartOutput 時に必須 | 出力先の設定（下記参照） |
| `segmentDuration` | number | - | セグメントの目標尺（秒）。デフォルト: 2.0 |
| `maxRetainedSegments` | number | - | プレイリストに保持するセグメント数。デフォルト: 6 |
| `segmentFormat` | string | - | セグメントフォーマット。`"mpegts"` (デフォルト) または `"fmp4"` |
| `variants` | array | - | バリアント定義の配列。デフォルト: 1 要素（2Mbps video, 128kbps audio） |

**`destination` (filesystem):**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `type` | string | 必須 | `"filesystem"` |
| `directory` | string | 必須 | セグメントとプレイリストの出力先ディレクトリ |

**`destination` (s3):**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `type` | string | 必須 | `"s3"` |
| `bucket` | string | 必須 | S3 バケット名 |
| `prefix` | string | - | オブジェクトキーの prefix。デフォルト: 空文字列 |
| `region` | string | 必須 | AWS リージョン |
| `endpoint` | string | - | カスタムエンドポイント URL（MinIO 等） |
| `usePathStyle` | boolean | - | パススタイルの URL を使用する。デフォルト: `false` |
| `credentials` | object | 必須 | 認証情報（下記参照） |
| `lifetimeDays` | number | - | オブジェクトのライフタイム（日数） |

> **`lifetimeDays` に関する注意:**
>
> - `lifetimeDays` を指定すると、hisui は Output 開始時に対象バケットの lifecycle 設定を更新する
> - この更新は既存ルールとの競合を安全に解決しないため、他の lifecycle ルールへ影響する可能性がある
> - 既存 lifecycle を運用している共有バケットでの利用は非推奨

**`credentials`:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `accessKeyId` | string | 必須 | アクセスキー ID |
| `secretAccessKey` | string | 必須 | シークレットアクセスキー |
| `sessionToken` | string | - | セッショントークン（一時認証情報用） |

**`variants` の各要素:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `videoBitrate` | number | 必須 | ビデオビットレート (bps) |
| `audioBitrate` | number | 必須 | オーディオビットレート (bps) |
| `width` | number | - | ビデオ幅（偶数）。省略時はキャンバスサイズを使用 |
| `height` | number | - | ビデオ高さ（偶数）。省略時はキャンバスサイズを使用 |

**ディレクトリ構成（filesystem, ABR）:**

```
<directory>/
├── playlist.m3u8          # マスタープレイリスト
├── variant_0/
│   ├── playlist.m3u8      # メディアプレイリスト
│   └── segment*.ts        # セグメントファイル
├── variant_1/
│   ├── playlist.m3u8
│   └── segment*.ts
```

**S3 オブジェクト構成（ABR）:**

```
<prefix>/playlist.m3u8          # マスタープレイリスト
<prefix>/variant_0/playlist.m3u8
<prefix>/variant_0/segment*.ts
<prefix>/variant_1/playlist.m3u8
<prefix>/variant_1/segment*.ts
```

**`GetOutputStatus` の `outputPath`:**

- filesystem: `<directory>/playlist.m3u8`
- s3: `s3://<bucket>/<prefix>/playlist.m3u8`

**フロー:**

1. `SetOutputSettings` で `destination` / `variants` 等を設定
2. `StartOutput` で pipeline 生成 + HLS セグメント書き出し開始
3. `GetOutputStatus` で稼働状態確認（`outputPath` にプレイリストパスを返す）
4. `StopOutput` で pipeline 停止 + 生成ファイル削除

**エラー条件:**

- `destination` が未設定の場合: `StartOutput` が失敗
- `destination.type` が `"filesystem"` でも `"s3"` でもない場合: `SetOutputSettings` が失敗
- `segmentDuration` が 0 以下の場合: `SetOutputSettings` が失敗
- `maxRetainedSegments` が 0 の場合: `SetOutputSettings` が失敗
- `variants` が空の場合: `SetOutputSettings` が失敗
- `variants[].videoBitrate` が 0 の場合: `SetOutputSettings` が失敗
- `variants[].audioBitrate` が 0 の場合: `SetOutputSettings` が失敗
- `variants[].width` / `height` が 0 または奇数の場合: `SetOutputSettings` が失敗
- `variants[].width` / `height` の片方だけ指定された場合: `SetOutputSettings` が失敗
- 二重開始: `StartOutput` が `OUTPUT_RUNNING` エラーを返す
- 未起動停止: `StopOutput` が `OUTPUT_NOT_RUNNING` エラーを返す

#### `mpeg_dash`

MPEG-DASH ライブ出力。H.264 + AAC の fragmented MP4 セグメントを生成し、MPD マニフェストで管理する。

- `outputKind`: `mpeg_dash_output`
- ローカルファイルシステムまたは S3 互換オブジェクトストレージへの出力に対応
- 停止時に生成ファイル（manifest.mpd + init.mp4 + 全セグメント）を自動削除する
- `outputBytes` / `outputSkippedFrames` / `outputTotalFrames` は現時点では 0 固定
- `variants` が 1 要素の場合は non-ABR（出力先直下にセグメントとマニフェストを出力）
- `variants` が 2 要素以上の場合は ABR（出力先直下に結合 MPD、`variant_N/` サブディレクトリ/prefix にバリアントごとのセグメントを出力）

**outputSettings（`SetOutputSettings` / `GetOutputSettings`）:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `destination` | object | StartOutput 時に必須 | 出力先の設定（下記参照） |
| `segmentDuration` | number | - | セグメントの目標尺（秒）。デフォルト: 2.0 |
| `maxRetainedSegments` | number | - | マニフェストに保持するセグメント数。デフォルト: 6 |
| `variants` | array | - | バリアント定義の配列。デフォルト: 1 要素（2Mbps video, 128kbps audio） |

**`destination` (filesystem):**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `type` | string | 必須 | `"filesystem"` |
| `directory` | string | 必須 | セグメントとマニフェストの出力先ディレクトリ |

**`destination` (s3):**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `type` | string | 必須 | `"s3"` |
| `bucket` | string | 必須 | S3 バケット名 |
| `prefix` | string | - | オブジェクトキーの prefix。デフォルト: 空文字列 |
| `region` | string | 必須 | AWS リージョン |
| `endpoint` | string | - | カスタムエンドポイント URL（MinIO 等） |
| `usePathStyle` | boolean | - | パススタイルの URL を使用する。デフォルト: `false` |
| `credentials` | object | 必須 | 認証情報（下記参照） |
| `lifetimeDays` | number | - | オブジェクトのライフタイム（日数） |

> **`lifetimeDays` に関する注意:**
>
> - `lifetimeDays` を指定すると、hisui は Output 開始時に対象バケットの lifecycle 設定を更新する
> - この更新は既存ルールとの競合を安全に解決しないため、他の lifecycle ルールへ影響する可能性がある
> - 既存 lifecycle を運用している共有バケットでの利用は非推奨

**`credentials`:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `accessKeyId` | string | 必須 | アクセスキー ID |
| `secretAccessKey` | string | 必須 | シークレットアクセスキー |
| `sessionToken` | string | - | セッショントークン（一時認証情報用） |

**`variants` の各要素:**

| フィールド | 型 | 必須 | 説明 |
|-----------|-----|------|------|
| `videoBitrate` | number | 必須 | ビデオビットレート (bps) |
| `audioBitrate` | number | 必須 | オーディオビットレート (bps) |
| `width` | number | - | ビデオ幅（偶数）。省略時はキャンバスサイズを使用 |
| `height` | number | - | ビデオ高さ（偶数）。省略時はキャンバスサイズを使用 |

**ディレクトリ構成（filesystem, non-ABR）:**

```
<directory>/
├── manifest.mpd
├── init.mp4
└── segment-*.m4s
```

**ディレクトリ構成（filesystem, ABR）:**

```
<directory>/
├── manifest.mpd          # 結合 MPD（全バリアントの Representation を含む）
├── variant_0/
│   ├── init.mp4
│   └── segment-*.m4s
├── variant_1/
│   ├── init.mp4
│   └── segment-*.m4s
```

**S3 オブジェクト構成（non-ABR）:**

```
<prefix>/manifest.mpd
<prefix>/init.mp4
<prefix>/segment-*.m4s
```

**S3 オブジェクト構成（ABR）:**

```
<prefix>/manifest.mpd
<prefix>/variant_0/init.mp4
<prefix>/variant_0/segment-*.m4s
<prefix>/variant_1/init.mp4
<prefix>/variant_1/segment-*.m4s
```

**`GetOutputStatus` の `outputPath`:**

- filesystem: `<directory>/manifest.mpd`
- s3: `s3://<bucket>/<prefix>/manifest.mpd`

**フロー:**

1. `SetOutputSettings` で `destination` / `variants` 等を設定
2. `StartOutput` で pipeline 生成 + MPEG-DASH セグメント書き出し開始
3. `GetOutputStatus` で稼働状態確認（`outputPath` にマニフェストパスを返す）
4. `StopOutput` で pipeline 停止 + 生成ファイル削除

**エラー条件:**

- `destination` が未設定の場合: `StartOutput` が失敗
- `destination.type` が `"filesystem"` でも `"s3"` でもない場合: `SetOutputSettings` が失敗
- `segmentDuration` が 0 以下の場合: `SetOutputSettings` が失敗
- `maxRetainedSegments` が 0 の場合: `SetOutputSettings` が失敗
- `variants` が空の場合: `SetOutputSettings` が失敗
- `variants[].videoBitrate` が 0 の場合: `SetOutputSettings` が失敗
- `variants[].audioBitrate` が 0 の場合: `SetOutputSettings` が失敗
- `variants[].width` / `height` が 0 または奇数の場合: `SetOutputSettings` が失敗
- `variants[].width` / `height` の片方だけ指定された場合: `SetOutputSettings` が失敗
- 二重開始: `StartOutput` が `OUTPUT_RUNNING` エラーを返す
- 未起動停止: `StopOutput` が `OUTPUT_NOT_RUNNING` エラーを返す
