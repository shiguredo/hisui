# OBS WebSocket 互換機能 実装状況

## 目的

- `obs-websocket` 互換機能の実装状況を 1 つのファイルで管理する
- PoC 段階でも、対応済み / 未対応 / 対象外を明確にする
- 実装時に、コードとテストの根拠を追跡できるようにする

## ステータス定義

- `未対応`: 実装されていない
- `実装済み`: 実装済みで、必要なテストがある
- `実装対象外`: 現時点の `hisui` の目的に対して対応しない

## 更新ルール

- `obsws` 関連の機能を追加・変更したら、このファイルを同一変更内で更新する
- `実装済み` にする際は、根拠となるコードとテストを併記する
- `実装対象外` にする際は、必ず理由を `実装メモ` に記載する

## 非 Request 機能

| 機能 | ステータス | 実装メモ | 根拠 |
|---|---|---|---|
| WebSocket 接続（`obswebsocket.json` 必須） | 実装済み | subprotocol が一致しない場合は handshake を拒否する | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| `Hello (op=0)` | 実装済み | `rpcVersion=1`, password 指定時は `authentication` を返す | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| `Identify (op=1)` | 実装済み | 認証あり/なしの両方に対応する | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| `Identified (op=2)` | 実装済み | `negotiatedRpcVersion=1` を返す | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| password 認証 | 実装済み | challenge/salt ベースで検証。失敗時は `4009` | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| `Request/RequestResponse (op=6/7)` 基盤 | 実装済み | Identify 後のみ Request を受け付ける | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| `Reidentify (op=3)` | 未対応 | 必要になった時点で追加する | - |
| Event 配信（`op=5`） | 未対応 | イベント購読モデルを未導入 | - |
| RequestBatch（`op=8/9`） | 未対応 | バッチ処理の要件を未定義 | - |

## RequestType 実装状況

| RequestType | カテゴリ | ステータス | 実装メモ | 根拠（コード/テスト） |
|---|---|---|---|---|
| `GetVersion` | General | 実装済み | `obsWebSocketVersion`, `rpcVersion`, `availableRequests` を返す | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| `GetStats` | General | 実装済み | PoC として `hisui` セッション/接続情報を中心に返す | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| `GetCanvasList` | Config | 実装済み | PoC として単一キャンバス（`hisui-main`）を返す | `src/subcommand_obsws.rs`, `e2e-tests/obsws/test_obsws.py` |
| `GetSceneList` | Scenes | 未対応 | シーンモデル未導入 | - |
| `GetInputList` | Inputs | 未対応 | 入力モデルと OBS の概念対応を未定義 | - |
| `GetCurrentProgramScene` | Scenes | 未対応 | シーン切替の概念未導入 | - |
| `SetCurrentProgramScene` | Scenes | 未対応 | シーン切替の概念未導入 | - |
| `StartStream` | Streaming | 実装対象外 | `hisui` は OBS の配信制御 API を持たないため対象外 | - |
| `StopStream` | Streaming | 実装対象外 | `hisui` は OBS の配信制御 API を持たないため対象外 | - |
| `StartRecord` | Recording | 実装対象外 | `hisui` の処理モデルと OBS Recording API が一致しないため対象外 | - |
| `StopRecord` | Recording | 実装対象外 | `hisui` の処理モデルと OBS Recording API が一致しないため対象外 | - |

## 未対応 Request の扱い

- 未対応 `requestType` は `RequestResponse` でエラー応答する
- 現状は `Unknown request type` を返す

## 次の候補（優先順）

1. `Reidentify (op=3)` の追加
2. `GetSceneList` / `GetInputList` の PoC 実装
3. Event 配信（`op=5`）の最小実装
4. RequestBatch（`op=8/9`）の追加
