# OBS WebSocket 互換機能 実装状況

## 参照仕様

- プロトコル仕様: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>
- 対象仕様バージョン: OBS WebSocket 5.x（ hisui 側の現在値: `obsWebSocketVersion = 5.0.0`, `rpcVersion = 1` ）

## 目的

- `obs-websocket` 互換機能の実装状況を 1 つのファイルで管理する
- PoC 段階でも、対応済み / 未対応 / 対象外を明確にする
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

- [x] `GetVersion`（ General ）: サーバーのバージョン情報と対応 Request 一覧を返す
- [x] `GetStats`（ General ）: 実行統計情報を返す
  - [ ] `cpuUsage`: CPU 使用率を返す（ 現状は `0.0` 固定 ）
  - [ ] `memoryUsage`: メモリ使用量を返す（ 現状は `0.0` 固定 ）
  - [ ] `availableDiskSpace`: 空きディスク容量を返す（ 現状は `0.0` 固定 ）
  - [ ] `activeFps`: 現在の FPS を返す（ 現状は `0.0` 固定 ）
  - [ ] `averageFrameRenderTime`: 平均レンダー時間を返す（ 現状は `0.0` 固定 ）
  - [x] `renderSkippedFrames`: レンダーでスキップしたフレーム数を返す
  - [x] `renderTotalFrames`: レンダー総フレーム数を返す
  - [x] `outputSkippedFrames`: 出力でスキップしたフレーム数を返す
  - [x] `outputTotalFrames`: 出力総フレーム数を返す
  - [x] `webSocketSessionIncomingMessages`: 現在セッションの受信メッセージ数を返す
  - [x] `webSocketSessionOutgoingMessages`: 現在セッションの送信メッセージ数を返す
  - [x] `hisuiSessionUptimeSec`: 現在セッションの稼働秒数を返す
  - [x] `hisuiServerUptimeSec`: `obsws` サーバー全体の稼働秒数を返す
  - [x] `hisuiCurrentConnections`: 現在接続中のクライアント数を返す
  - [x] `hisuiTotalConnections`: 累計接続数を返す
  - [x] `hisuiTotalRequests`: 累計 Request 処理数を返す
  - [x] `hisuiFailedRequests`: 累計 Request 失敗数を返す
  - NOTE: OBS 互換の統計項目のうち一部は PoC として固定値を返している
- [x] `GetCanvasList`（ Config ）: 利用可能なキャンバス一覧を返す
  - NOTE: PoC として単一キャンバス（ `hisui-main` ）を返す
- [ ] `GetSceneList`（ Scenes ）: シーン一覧を返す
- [ ] `GetInputList`（ Inputs ）: 入力一覧を返す
- [ ] `GetCurrentProgramScene`（ Scenes ）: 現在の Program Scene を返す
- [ ] `SetCurrentProgramScene`（ Scenes ）: Program Scene を切り替える
- [ ] `StartStream`（ Streaming ）: 配信開始を制御する
- [ ] `StopStream`（ Streaming ）: 配信停止を制御する
- [ ] `StartRecord`（ Recording ）: 録画開始を制御する
- [ ] `StopRecord`（ Recording ）: 録画停止を制御する

## 実装対象外

- [ ] MessagePack: WebSocket の MessagePack サブプロトコル対応
  - NOTE: 現状は `obswebsocket.json` のみを対象とする

## 未対応 Request の扱い

- [x] 未対応 `requestType` は `RequestResponse` でエラー応答する
- [x] エラー内容は `Unknown request type` を返す
