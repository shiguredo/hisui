# obsws の `streamServiceSettings` JSON 出力 3 箇所を共通基盤に統合する

- Priority: Low
- Created: 2026-06-18
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-unify-stream-service-settings-emitters
- Polished:

## 目的

obsws の `streamServiceSettings` キーの中身を JSON 化するロジックが現状 src/obsws 配下の 3 箇所で独立に書かれており、共通部分の保守追従漏れリスクを抱えている。issue 0003 で `bwtest` を削除した結果、3 経路の差分が「OBS rtmp-custom.c 互換のための `use_auth` ハードコードと `key` の default 出力」だけに収束した。OBS 互換のための差分を 1 経路に閉じて、共通部分を 1 つの基盤に統合する。

## 優先度根拠

Low。

- 機能影響なし。リファクタ目的
- 緊急度なし。現状で 3 経路の JSON 出力は互換が取れている
- ただし「(2) と (3) が完全にコピペ状態」のため、片方への変更がもう片方に追従漏れする経路が成立する状態は構造的負債。issue 0041 (pipeline 残骸の死にコード削除) と並ぶ「obsws JSON 出力経路の clean-up」シリーズとして整理する価値あり

## 現状

### 対象 3 箇所

issue 0003 のレビュー中に確認:

1. `src/obsws/coordinator/output_registry.rs:465-481` の `handle_get_stream_service_settings`
   - 用途: `GetStreamServiceSettings` レスポンス (OBS WebSocket Protocol 標準 API)
   - 出力: `streamServiceType` + `streamServiceSettings` { `server` [Some なら出力], `key` [None でも `""` を出力], **`use_auth: false`** [常にハードコード] }
2. `src/obsws/coordinator/output_stream.rs:240-258` の `ObswsStreamServiceSettings::fmt`
   - 用途: `GetOutputSettings` レスポンス (hisui 汎用 output 取得)
   - 出力: `streamServiceType` + `streamServiceSettings` { `server` [Some なら出力], `key` [Some なら出力] }
3. `src/obsws/state_file.rs:1157-1176` の `ObswsStateFileStream::fmt`
   - 用途: state file 永続化
   - 出力: `streamServiceType` + `streamServiceSettings` { `server` [Some なら出力], `key` [Some なら出力] }

### 差分の整理

- (1) (2) (3) はラッパ構造が同じ (`streamServiceType` + `streamServiceSettings` object)
- (1) だけ `use_auth: false` を常にハードコード出力し、`key` の None 時に `""` を default 出力する
- (1) の差分は OBS Studio の rtmp-custom.c plugin が `use_auth` キーを必須として読むための互換要件
- (2) と (3) は出力ロジックが完全に同一だが、source struct が `ObswsStreamServiceSettings` (output_stream.rs) と `ObswsStateFileStream` (state_file.rs) で別物
- `state_file.rs:1487-1489` に `ObswsStateFileStream::to_stream_service_settings()` が既に存在しており、`ObswsStateFileStream → ObswsStreamServiceSettings` 変換は実装済み

### 既存テストの担保

- `src/obsws/session/tests.rs:3093` 周辺で `ObswsStreamServiceSettings::fmt` の出力 (`streamServiceSettings.server`) を期待するテストが存在する
- リファクタ前後の互換確認の根拠として利用可能

### 関連 docs

- `docs/obsws/json_naming.md` 章 5.3 に OBS rtmp-custom.c の `use_auth` 互換要件を記録済み
- `docs/obsws/json_naming.md` 章 4 (本ブランチ 0003 で外向き仕様レベルに再構成済み) に「`GetStreamServiceSettings` の `streamServiceSettings` は OBS rtmp-custom.c 互換のため `use_auth: false` を常に含む」と明記済み

## 設計方針

未確定。本 issue 着手時に polish-issue で詰める。検討ポイント:

- 共通基盤を関数として切り出すか (`fn write_stream_service_settings_payload(f, server, key)` 等)、`impl DisplayJson for ObswsStreamServiceSettings` のままにして (1) で wrapper を挟むか
- (3) の `ObswsStateFileStream` を `ObswsStreamServiceSettings` 自体に置き換えるか (構造体を 1 つに統合)、別物のまま `to_stream_service_settings()` 変換経由で共通基盤を呼ぶか
- (1) の `key` の `""` default が OBS Studio 互換のために本当に必須かを再確認する (rtmp-custom.c が `obs_data_get_string` の default value をどう扱うか OBS Studio ソースで裏取り)

## 完了条件

- `streamServiceType` + `streamServiceSettings` { `server` [Some なら出力] } の共通ラッパ出力ロジックがコードベース上 1 箇所にだけ存在すること
- (1) `handle_get_stream_service_settings` 固有の `use_auth: false` と `key` default は (1) に閉じていること
- (2) `ObswsStreamServiceSettings::fmt` と (3) `ObswsStateFileStream::fmt` は共通ロジックを呼ぶ形になっていること
- リファクタ前後で 3 経路の JSON 出力が完全に互換であること (`session/tests.rs:3093` 周辺の既存テスト + 必要に応じて追加テストで担保)
- `cargo check --all-features --tests --benches` / `cargo test --all` / `cargo clippy --all-targets --all-features -- -D warnings` がすべて通ること

### CHANGES.md

`## develop` に追記しない。内部リファクタで利用者から見える挙動・公開 API は一切変化しないため。

## 解決方法

1. 共通基盤の形 (関数切り出し / wrapper / 構造体統合) を決定する
2. 共通基盤に (2) `ObswsStreamServiceSettings::fmt` を寄せる
3. (3) `ObswsStateFileStream::fmt` も `to_stream_service_settings()` 経由で共通基盤を呼ぶ形に書き換える
4. (1) `handle_get_stream_service_settings` を共通基盤 + `use_auth: false` ハードコード + `key` default の追加に書き換える
5. 既存テスト (`session/tests.rs:3093` 周辺含む) がすべて通ることを確認する
6. リファクタ前後の差分を 3 経路それぞれ手で比較する (`cargo run` でローカル起動して `GetStreamServiceSettings` / `GetOutputSettings` / state file 永続化の JSON を取得して目視確認するか、追加 roundtrip テストで担保)
