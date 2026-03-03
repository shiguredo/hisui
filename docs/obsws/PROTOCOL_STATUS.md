# OBS WebSocket 互換機能 実装状況

## 参照仕様

- プロトコル仕様: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>
- 対象仕様バージョン: OBS WebSocket 5.x（ hisui 側の現在値: `obsWebSocketVersion = 5.0.0`, `rpcVersion = 1` ）

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
- [ ] `Reidentify (op=3)`: 既存セッションの再設定を受け付ける
- [ ] Event 配信（ `op=5` ）: サーバーイベントを push 配信する
- [ ] `RequestBatch (op=8/9)`: 複数 Request のバッチ処理を行う

## RequestType 実装状況

### General

- [x] `GetVersion`: サーバーのバージョン情報と対応 Request 一覧を返す
  - [x] `availableRequests`: 対応 RequestType 一覧を返す
  - [x] `supportedImageFormats`: 対応画像フォーマット一覧を返す
- [x] `GetStats`: 実行統計情報を返す
  - [ ] `cpuUsage`: CPU 使用率を返す（ 現状は `0.0` 固定 ）
  - [ ] `memoryUsage`: メモリ使用量を返す（ 現状は `0.0` 固定 ）
  - [ ] `availableDiskSpace`: 空きディスク容量を返す（ 現状は `0.0` 固定 ）
  - [ ] `activeFps`: 現在の FPS を返す（ 現状は `0.0` 固定 ）
  - [ ] `averageFrameRenderTime`: 平均レンダー時間を返す（ 現状は `0.0` 固定 ）
  - [ ] `renderSkippedFrames`: レンダーでスキップしたフレーム数を返す（ 現状は `0` 固定 ）
  - [ ] `renderTotalFrames`: レンダー総フレーム数を返す（ 現状は `0` 固定 ）
  - [ ] `outputSkippedFrames`: 出力でスキップしたフレーム数を返す（ 現状は `0` 固定 ）
  - [ ] `outputTotalFrames`: 出力総フレーム数を返す（ 現状は `0` 固定 ）
  - [x] `webSocketSessionIncomingMessages`: 現在セッションの受信メッセージ数を返す
  - [x] `webSocketSessionOutgoingMessages`: 現在セッションの送信メッセージ数を返す
- [ ] `BroadcastCustomEvent`: カスタムイベントを配信する
- [ ] `CallVendorRequest`: ベンダー拡張リクエストを実行する
- [ ] `GetHotkeyList`: ホットキー一覧を取得する
- [ ] `TriggerHotkeyByName`: 名前指定でホットキーを発火する
- [ ] `TriggerHotkeyByKeySequence`: キーシーケンス指定でホットキーを発火する
- [ ] `Sleep`: 指定時間だけ処理を待機する

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
- [ ] `GetStreamServiceSettings`: 配信サービス設定を取得する
- [ ] `SetStreamServiceSettings`: 配信サービス設定を更新する
- [ ] `GetRecordDirectory`: 録画ディレクトリを取得する
- [ ] `SetRecordDirectory`: 録画ディレクトリを設定する

### Sources

- [ ] `GetSourceActive`: ソースのアクティブ状態を取得する
- [ ] `GetSourceScreenshot`: ソースのスクリーンショットを取得する
- [ ] `SaveSourceScreenshot`: ソースのスクリーンショットをファイル保存する

### Canvases

- [ ] `GetCanvasList`: 利用可能なキャンバス一覧を返す

### Scenes

- [ ] `GetSceneList`: シーン一覧を取得する
- [ ] `GetGroupList`: グループ一覧を取得する
- [ ] `GetCurrentProgramScene`: 現在の Program Scene を取得する
- [ ] `SetCurrentProgramScene`: Program Scene を切り替える
- [ ] `GetCurrentPreviewScene`: 現在の Preview Scene を取得する
- [ ] `SetCurrentPreviewScene`: Preview Scene を切り替える
- [ ] `CreateScene`: シーンを作成する
- [ ] `RemoveScene`: シーンを削除する
- [ ] `SetSceneName`: シーン名を変更する
- [ ] `GetSceneSceneTransitionOverride`: シーン遷移上書き設定を取得する
- [ ] `SetSceneSceneTransitionOverride`: シーン遷移上書き設定を更新する

### Inputs

- [ ] `GetInputList`: 入力一覧を取得する
- [ ] `GetInputKindList`: 入力種別一覧を取得する
- [ ] `GetSpecialInputs`: 特殊入力設定を取得する
- [ ] `CreateInput`: 入力を作成する
- [ ] `RemoveInput`: 入力を削除する
- [ ] `SetInputName`: 入力名を変更する
- [ ] `GetInputDefaultSettings`: 入力の既定設定を取得する
- [ ] `GetInputSettings`: 入力設定を取得する
- [ ] `SetInputSettings`: 入力設定を更新する
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
- [ ] `GetInputPropertiesListPropertyItems`: リスト型プロパティ項目を取得する
- [ ] `PressInputPropertiesButton`: 入力プロパティのボタンを押下する

### Transitions

- [ ] `GetTransitionKindList`: 遷移種別一覧を取得する
- [ ] `GetSceneTransitionList`: 遷移一覧を取得する
- [ ] `GetCurrentSceneTransition`: 現在の遷移情報を取得する
- [ ] `SetCurrentSceneTransition`: 現在の遷移を設定する
- [ ] `SetCurrentSceneTransitionDuration`: 遷移時間を設定する
- [ ] `SetCurrentSceneTransitionSettings`: 遷移設定を更新する
- [ ] `GetCurrentSceneTransitionCursor`: 遷移カーソル位置を取得する
- [ ] `TriggerStudioModeTransition`: Studio Mode の遷移を実行する
- [ ] `SetTBarPosition`: TBar 位置を設定する

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

- [ ] `GetSceneItemList`: シーン内アイテム一覧を取得する
- [ ] `GetGroupSceneItemList`: グループ内アイテム一覧を取得する
- [ ] `GetSceneItemId`: ソース名からシーンアイテム ID を取得する
- [ ] `GetSceneItemSource`: シーンアイテムに紐づくソースを取得する
- [ ] `CreateSceneItem`: シーンアイテムを作成する
- [ ] `RemoveSceneItem`: シーンアイテムを削除する
- [ ] `DuplicateSceneItem`: シーンアイテムを複製する
- [ ] `GetSceneItemTransform`: シーンアイテム変形情報を取得する
- [ ] `SetSceneItemTransform`: シーンアイテム変形情報を設定する
- [ ] `GetSceneItemEnabled`: シーンアイテム有効状態を取得する
- [ ] `SetSceneItemEnabled`: シーンアイテム有効状態を設定する
- [ ] `GetSceneItemLocked`: シーンアイテムロック状態を取得する
- [ ] `SetSceneItemLocked`: シーンアイテムロック状態を設定する
- [ ] `GetSceneItemIndex`: シーンアイテム順序を取得する
- [ ] `SetSceneItemIndex`: シーンアイテム順序を設定する
- [ ] `GetSceneItemBlendMode`: シーンアイテム合成モードを取得する
- [ ] `SetSceneItemBlendMode`: シーンアイテム合成モードを設定する

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
- [ ] `GetOutputList`: 出力一覧を取得する
- [ ] `GetOutputStatus`: 出力状態を取得する
- [ ] `ToggleOutput`: 出力をトグルする
- [ ] `StartOutput`: 出力を開始する
- [ ] `StopOutput`: 出力を停止する
- [ ] `GetOutputSettings`: 出力設定を取得する
- [ ] `SetOutputSettings`: 出力設定を更新する

### Stream

- [ ] `GetStreamStatus`: 配信状態を取得する
- [ ] `ToggleStream`: 配信をトグルする
- [ ] `StartStream`: 配信を開始する
- [ ] `StopStream`: 配信を停止する
- [ ] `SendStreamCaption`: 配信キャプションを送信する

### Record

- [ ] `GetRecordStatus`: 録画状態を取得する
- [ ] `ToggleRecord`: 録画をトグルする
- [ ] `StartRecord`: 録画を開始する
- [ ] `StopRecord`: 録画を停止する
- [ ] `ToggleRecordPause`: 録画一時停止をトグルする
- [ ] `PauseRecord`: 録画を一時停止する
- [ ] `ResumeRecord`: 録画を再開する
- [ ] `SplitRecordFile`: 録画ファイルを分割する
- [ ] `CreateRecordChapter`: 録画チャプターを作成する

### Media Inputs

- [ ] `GetMediaInputStatus`: メディア入力状態を取得する
- [ ] `SetMediaInputCursor`: メディア入力カーソル位置を設定する
- [ ] `OffsetMediaInputCursor`: メディア入力カーソル位置を相対移動する
- [ ] `TriggerMediaInputAction`: メディア入力アクションを実行する

### UI

- [ ] `GetStudioModeEnabled`: Studio Mode の有効状態を取得する
- [ ] `SetStudioModeEnabled`: Studio Mode の有効状態を設定する
- [ ] `OpenInputPropertiesDialog`: 入力プロパティダイアログを開く
- [ ] `OpenInputFiltersDialog`: 入力フィルターダイアログを開く
- [ ] `OpenInputInteractDialog`: 入力インタラクトダイアログを開く
- [ ] `GetMonitorList`: モニター一覧を取得する
- [ ] `OpenVideoMixProjector`: 映像ミックスのプロジェクターを開く
- [ ] `OpenSourceProjector`: ソースプロジェクターを開く

## 実装対象外

- MessagePack: WebSocket の MessagePack サブプロトコル対応
  - NOTE: 現状は `obswebsocket.json` のみを対象とする

## 未対応 Request の扱い

- [x] 未対応 `requestType` は `RequestResponse` でエラー応答する
- [x] エラー内容は `Unknown request type` を返す
