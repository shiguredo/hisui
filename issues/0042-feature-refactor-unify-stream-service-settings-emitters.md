# obsws の `streamServiceSettings` JSON 出力 3 箇所を共通基盤に統合する

- Priority: Low
- Created: 2026-06-18
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-unify-stream-service-settings-emitters
- Polished: 2026-06-19

## 目的

obsws の `streamServiceSettings` キーの中身を JSON 化するロジックが現状 `src/obsws/` 配下の 3 箇所で独立に書かれており、共通部分の保守追従漏れリスクを抱えている。closed issue 0003 (`feature/change-obsws-json-naming`) で `bwtest` を削除し、`docs/obsws/json_naming.md` に外向き仕様を確定させた結果、3 経路の差分が「OBS Studio クライアント互換のための `use_auth: false` ハードコードと `key` の常時出力 (None 時 `""`)」だけに収束した。OBS 互換のための差分を 1 経路に閉じて、共通部分を 1 つの基盤に統合する。

## 優先度根拠

Low。挙動・公開 API 不変のリファクタで緊急度はないが、(2) と (3) は `impl DisplayJson` の 18 行が改行・インデントまで含めてバイト一致のコピペで、片方への変更がもう片方に追従漏れする経路が成立する構造的負債。新規 `streamServiceSettings` 出力経路を追加する予定が現状無いため Low に留める。

## 現状

### 対象 3 箇所

`streamServiceSettings` キーの中身を JSON 化している箇所:

1. `src/obsws/coordinator/output_registry.rs:465-481` の `handle_get_stream_service_settings`
   - 用途: `GetStreamServiceSettings` レスポンス (OBS WebSocket Protocol 標準 API)
   - 出力: `streamServiceType` + `streamServiceSettings { [Some なら server], "key" (常時出力、None 時 `""`), "use_auth": false (常にハードコード) }`
   - `ObswsStreamServiceSettings` を経由せず、直接 `f.member` で envelope を組み立てる
2. `src/obsws/coordinator/output_stream.rs:240-259` の `impl nojson::DisplayJson for ObswsStreamServiceSettings`
   - 用途: `GetOutputSettings` レスポンス (hisui 汎用 output 取得)
   - 出力: `streamServiceType` + `streamServiceSettings { [Some なら server], [Some なら key] }`
3. `src/obsws/state_file.rs:1157-1176` の `impl nojson::DisplayJson for ObswsStateFileStream`
   - 用途: state file 永続化
   - 出力: (2) と同一形

### 差分の整理

- (1) (2) (3) はラッパ構造が同じ (`streamServiceType` + `streamServiceSettings` object)
- (1) だけ次の 2 つの OBS 互換固有差分を持つ:
  - `use_auth: false` を常にハードコード出力する (OBS Studio の rtmp-custom.c plugin が `use_auth` キーを必須として読むための互換要件)
  - `key` を常時出力する (Some 時はそのまま、None 時は `""`)。(2) と (3) は `if let Some(key)` で None 時はキー自体を省略する点が異なる
- (2) と (3) は出力ロジックが完全に同一 (`impl DisplayJson` の 18 行がバイト一致)。さらにフィールド構成 (`stream_service_type: String / server: Option<String> / key: Option<String>`) も型定義レベル (`output_stream.rs:221-225` と `state_file.rs:81-85`) で完全一致
- `src/obsws/state_file.rs:1488-1494` に `ObswsStateFileStream::to_stream_service_settings()` が既に存在しており、`ObswsStateFileStream → ObswsStreamServiceSettings` 変換は実装済み

### 関連 docs

- `docs/obsws/json_naming.md` 章 4 (line 78) に「`GetStreamServiceSettings` の `streamServiceSettings` は OBS Studio クライアント互換のため `use_auth: false` を常に含む」と明記済み

### 既存テストの担保

- `src/obsws/session/tests.rs:3093` 周辺で `ObswsStreamServiceSettings::fmt` の出力 (`streamServiceSettings.server`) を期待するテストが存在する。リファクタ後の (2) 経路の bit 互換確認の根拠として利用可能
- `src/obsws/state_file.rs::tests` の `parse_full_state_file` 等で `ObswsStateFileStream` を含む固定 JSON 比較テストが存在する。リファクタ後の (3) 経路の bit 互換確認の根拠として利用可能

## 設計方針

### 1. 共通基盤の形: 関数切り出し（採用）

`src/obsws/coordinator/output_stream.rs` に共通ヘルパ関数を追加する。命名は既存の `output_dash.rs::DashDestination::fmt_with_credentials` (`:129`) / `output_hls.rs::HlsDestination::fmt_with_credentials` (`:165`) の「`fmt_` 接頭辞 + `nojson::JsonFormatter` を受ける」パターンに揃える。`obs_compat: bool` は OBS 互換要件が現状 1 種類しかないため enum 化は見送る (互換要件が 2 種類以上に増えた段階で別 issue として再検討)。

シグネチャ案:

```rust
/// `streamServiceType` + `streamServiceSettings { server, key }` の envelope を JSON に書き出す。
///
/// `obs_compat: true` の場合、`docs/obsws/json_naming.md` 章 4 の OBS Studio クライアント互換要件に従い
/// `key` を常時出力 (None 時 `""`) し `use_auth: false` をハードコード出力する。
pub(crate) fn fmt_stream_service_envelope(
    f: &mut nojson::JsonFormatter<'_, '_>,
    stream_service_type: &str,
    server: Option<&str>,
    key: Option<&str>,
    obs_compat: bool,
) -> std::fmt::Result {
    f.member("streamServiceType", stream_service_type)?;
    f.member(
        "streamServiceSettings",
        nojson::object(|f| {
            if let Some(server) = server {
                f.member("server", server)?;
            }
            if obs_compat {
                f.member("key", key.unwrap_or(""))?;
                f.member("use_auth", false)?;
            } else if let Some(key) = key {
                f.member("key", key)?;
            }
            Ok(())
        }),
    )
}
```

呼び出し側の nesting context が経路ごとに異なるため、呼び出しパターンを 2 種類使い分ける:

```rust
// (1) handle_get_stream_service_settings: build_request_response_success のクロージャ内で
// 既に response 直下の object formatter が開いているため直接呼ぶ。
// `true` 引数には call site で `// OBS Studio クライアント互換` のコメントを付ける。
fmt_stream_service_envelope(
    f,
    &settings.stream_service_type,
    settings.server.as_deref(),
    settings.key.as_deref(),
    true, // OBS Studio クライアント互換
)?

// (2) ObswsStreamServiceSettings::fmt / (3) ObswsStateFileStream::fmt: 新規 object を開く必要があるため
// nojson::object で包む。
nojson::object(|f| {
    fmt_stream_service_envelope(
        f,
        &self.stream_service_type,
        self.server.as_deref(),
        self.key.as_deref(),
        false,
    )
})
.fmt(f)
```

### 2. (3) `ObswsStateFileStream` は別物のまま維持

`ObswsStateFileStream` は state file 入力解析の `TryFrom<RawJsonValue>` (`state_file.rs:154`) と各種テストフィクスチャ (`:1612` / `:1650` / `:1736`) を持ち、`ObswsStreamServiceSettings` は output 経路で `update_from_json` / `parse_from_json` (`output_stream.rs:265, :314`) を持つ。両者は責務 (state file 入力解析 / output JSON 構築) が異なり、構造体統合は state file の JSON 出力フォーマットを意図せず変える可能性があるため本 issue では実施しない。(3) の `fmt` は新設の `fmt_stream_service_envelope` を直接呼ぶ形に書き換える。

### 3. (1) の OBS 互換差分

`use_auth: false` ハードコードと `key` の常時出力は `docs/obsws/json_naming.md` 章 4 で明記された外向き互換要件のため現状維持。本 issue では `obs_compat: true` フラグ (§1 で enum 化見送りを決定済み) で共通基盤に明示的に伝える形に整理し、(1) の呼び出し箇所に互換差分を閉じ込める。`use_auth` / `key` default の OBS Studio ソース裏取りは `json_naming.md` の確定要件を真とするため本 issue では行わない。

### 4. 対象外スコープ

本 issue で扱わない:

- **読み取り側 3 経路** (`output_stream.rs::update_from_json` (`:265`) / `parse_from_json` (`:314`)、`state_file.rs::TryFrom for ObswsStateFileStream` (`:154`))。本 issue は出力側 (`impl DisplayJson` / `f.member`) の統合のみを扱う
- **`handle_set_stream_service_settings` (Set 側)** (`output_registry.rs:410-442`)。`parse_set_stream_service_settings_fields` 経由で `server` / `key` のみを読み `use_auth` を読まない現状仕様を変えない。Get/Set の対称性は維持
- **`key` default `""` や `use_auth: false` の OBS 互換要件再評価**。`docs/obsws/json_naming.md` 章 4 で確定済み
- **構造体統合 (`ObswsStateFileStream` を `ObswsStreamServiceSettings` で置き換え)**。state file 永続化フォーマットへの影響可能性があり別 issue 扱い (`feature/change-` 系、CHANGES.md 記載対象)

## 完了条件

- `fmt_stream_service_envelope` 関数 (または同等の共通ヘルパ) が `src/obsws/coordinator/output_stream.rs` に追加されていること
- `streamServiceType` + `streamServiceSettings { server, key }` envelope の出力ロジックがコードベース上 1 箇所だけに存在すること
- (1) `handle_get_stream_service_settings` 固有の `use_auth: false` と `key` 常時出力は共通基盤の `obs_compat: true` フラグ経由で表現され、(1) の呼び出し箇所だけに差分が閉じていること
- (2) `ObswsStreamServiceSettings::fmt` と (3) `ObswsStateFileStream::fmt` は共通ヘルパを呼ぶ形になっていること
- リファクタ前後で 3 経路の JSON 出力がバイト互換であること:
  - 既存テスト維持: `src/obsws/session/tests.rs:3093` 周辺の `streamServiceSettings.server` を assert するテストが通ること
  - 既存テスト維持: `src/obsws/state_file.rs::tests` の `parse_full_state_file` 等で `streamServiceSettings` を含む固定 JSON 比較が、共通ヘルパ経由後の (3) `ObswsStateFileStream::fmt` 出力でも通ること
  - 追加 (1) 経路 (`src/obsws/session/tests.rs` に既存の `create_coordinator_handle` を使う形で追加。`handle_get_stream_service_settings` は `ObswsCoordinator` のメソッドで、メソッドだけを単独で呼ぶには full coordinator initializer が必要なため、既存 session テストの基盤を流用する) 2 件:
    - `handle_get_stream_service_settings_emits_use_auth_when_key_none`: `key=None`, `server=Some` で出力 JSON の `streamServiceSettings` に `"key": ""` と `"use_auth": false` が含まれること
    - `handle_get_stream_service_settings_emits_use_auth_when_key_some`: `key=Some("k")`, `server=None` で出力 JSON の `streamServiceSettings` に `"key": "k"` と `"use_auth": false` が含まれ、`server` キーが省略されていること (`obs_compat: true` でも `key=Some` 時に else 分岐を誤って削っていないかの回帰検知)
  - 追加 (2) 経路 (`src/obsws/coordinator/output_stream.rs` に新規 `#[cfg(test)] mod tests` モジュールを追加) 1 件:
    - `obsws_stream_service_settings_fmt_omits_obs_compat_keys`: `ObswsStreamServiceSettings { server: None, key: None, .. }` を `nojson` でシリアライズしたとき、`streamServiceSettings` 内に `use_auth` キーも `key` キーも含まれないこと (`obs_compat: false` 経路で OBS 互換差分が漏れ出ていないかの回帰検知)
  - 追加テストの assert 手段は既存 `session/tests.rs:3093` 周辺の慣習 (`nojson::RawJson::parse(text.text()) → to_path_member(...)`) に揃える
- 挙動不変が要件のため PBT / fuzzing / golden file テストは導入しない (固定出力 assert で担保する)
- `cargo check --all-features --tests --benches` / `cargo test --all` / `cargo clippy --all-targets --all-features -- -D warnings` がすべて通ること

### CHANGES.md

`## develop` に追記しない。内部リファクタで利用者から見える挙動・公開 API・state file の永続化フォーマットは一切変化しないため。

## 解決方法

実装ステップ:

1. `src/obsws/coordinator/output_stream.rs` に `fmt_stream_service_envelope` 関数を追加する
2. (2) `ObswsStreamServiceSettings::fmt` を共通ヘルパ呼び出しに書き換える (`obs_compat: false`)
3. (3) `state_file.rs::ObswsStateFileStream::fmt` を共通ヘルパ呼び出しに書き換える (`obs_compat: false`)
4. (1) `output_registry.rs::handle_get_stream_service_settings` を共通ヘルパ呼び出しに書き換える (`obs_compat: true`)
5. 完了条件節に列挙した追加テスト 3 件 ((1) 経路 2 件 + (2) 経路 1 件) を実装する
6. 既存テスト (`session/tests.rs:3093` 周辺、`state_file.rs::tests`) が通ることを確認する

### コミット分割

`shiguredo-git` 規約 (`{SEQ} {TITLE}` 形式) に従い、blame 汚染範囲を限定するため論理単位で分ける。ヘルパ関数を単独コミットで導入すると呼び出し元 0 で `cargo clippy --deny warnings` の dead_code 警告が出るため、ヘルパ追加と (2) の差し替えを 1 コミットに同梱する:

1. 実装ステップ 1 + 2: `0042 obsws stream service settings の共通ヘルパ fmt_stream_service_envelope を導入し ObswsStreamServiceSettings::fmt を寄せる`
2. 実装ステップ 3: `0042 ObswsStateFileStream::fmt を共通ヘルパに寄せる`
3. 実装ステップ 4 + 5 + 6: `0042 handle_get_stream_service_settings を共通ヘルパ + obs_compat フラグに書き換える`

## 関連

- closed issue 0003 (`feature/change-obsws-json-naming`): 本 issue の前提を整える (`bwtest` 削除と `docs/obsws/json_naming.md` 章 4 への外向き互換要件記録)
- open issue 0046 (`feature/refactor-clarify-processor-validation-boundary`): 同じ `src/obsws/coordinator/output_stream.rs` を触る予定だが、対象は line 359 付近の `start_stream_processors` で本 issue の line 240-259 とは衝突しない
