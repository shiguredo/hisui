# PersistentData

## 概要

OBS WebSocket 5.x の `GetPersistentData` / `SetPersistentData` リクエストにより、
クライアントが任意の JSON データをサーバー側に永続保存できる。
モバイルリモコンや自動化ボットなどが、セッションをまたいだ状態の保持に利用する。

## OBS WebSocket 仕様における realm

OBS 本家では、PersistentData のスコープを **realm** で分離している。

| realm | スコープ | 説明 |
|-------|---------|------|
| `OBS_WEBSOCKET_DATA_REALM_GLOBAL` | OBS インスタンス全体 | プロファイルを切り替えても同じデータにアクセスできる |
| `OBS_WEBSOCKET_DATA_REALM_PROFILE` | アクティブなプロファイル | プロファイル切り替えでアクセスされるデータも切り替わる |

OBS Studio では複数のプロファイルを作成し、配信設定（ビットレート、エンコーダ、配信先 URL 等）を
プロファイル単位で切り替えられる。PROFILE realm はこのプロファイルに紐づくストレージである。

```
GLOBAL realm:
 └─ slotA → 値1 （全プロファイル共通）

PROFILE realm:
 ├─ Profile「YouTube」
 │  └─ slotA → 値X
 └─ Profile「Twitch」
    └─ slotA → 値Y （プロファイルごとに独立）
```

## hisui での対応方針

hisui にはプロファイルの概念がないため、**`OBS_WEBSOCKET_DATA_REALM_GLOBAL` のみ対応する**。

`OBS_WEBSOCKET_DATA_REALM_PROFILE` が指定された場合は
`REQUEST_STATUS_INVALID_REQUEST_FIELD` (400) エラーを返す。

### 理由

- hisui にはプロファイル切り替え機能がなく、PROFILE realm を実装しても「切り替え先」が存在しない
- GLOBAL と PROFILE を同一視して透過的に処理する案もあるが、仕様との乖離を生むため採用しない
- PROFILE realm が必要になった場合は、プロファイル機能の実装と合わせて対応する

## 永続化

- `--state-file` 指定時に限り、state file の `persistentData` フィールドに永続化する
- `--state-file` 未指定時はメモリ上にのみ保持し、再起動でデータは失われる
