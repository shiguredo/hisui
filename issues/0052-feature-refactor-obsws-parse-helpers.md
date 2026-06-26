# obsws の汎用 JSON フィールド解析ヘルパーを共通モジュールに集約する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-obsws-parse-helpers
- Polished: 2026-06-26

## 目的

`src/obsws/coordinator/text_overlay.rs` に閉じている汎用 JSON フィールド解析ヘルパー群を、 共通モジュール `src/obsws/coordinator/parse_helpers.rs` に集約して他ハンドラから参照可能な形に整える。 obsws の外部応答仕様 (応答 JSON のキー名・キー順・値、 および `RequestBatchResult` の各フィールド値) は変化しない内部リファクタとして扱う。

## 優先度根拠

機能的には現状で動作しているが、 同種のパース関数は既に複数ファイルに散らばっており (`text_overlay.rs` / `coordinator.rs` / `output_registry.rs` / `response.rs`)、 新しい obsws リクエストハンドラを追加するたびに重複が増殖する状況 (Broken Windows)。 業務影響はないため Low。

## 現状

### 集約対象 (text_overlay.rs から parse_helpers.rs に移動)

`src/obsws/coordinator/text_overlay.rs` に以下が集中している (行番号は 2026-06-25 時点、 いずれもモジュール内 private)。

汎用 JSON フィールド解析ヘルパー 7 関数:

- `parse_required_string_field` (l.533) — 必須文字列、 空文字も valid 値として透過
- `parse_required_non_empty_string` (l.553) — 必須文字列、 空文字は `Missing` 扱い (識別子向け)
- `parse_required_i64_field` (l.565) — 必須 i64
- `parse_required_u32_field` (l.584) — 必須 u32 (i64 経由 + 範囲チェック)
- `parse_optional_string` (l.603) — オプション文字列 (null は `Err` = 明示的指定を拒否)
- `parse_optional_i64` (l.623) — オプション i64
- `parse_optional_u32` (l.657) — オプション u32

関連型・メソッド:

- `RequiredFieldError` enum (l.24-30、 バリアントは `Missing` / `Invalid(String)`)
- `ObswsCoordinator::map_required_field_error` メソッド (l.441-462、 `RequiredFieldError` を `CommandResult` に変換)。 呼び出し箇所は **7 箇所** で全て text_overlay.rs 内: Create で 5 箇所 (l.66/77/81/85/90)、 Update で 1 箇所 (l.188)、 Remove で 1 箇所 (l.301)。 既存 doc comment「5 必須フィールド分の match を簡素化」 (l.440) は Update / Remove を見落とした古い記述

関連単体テスト (集約と同時に移動):

- `#[cfg(test)] mod tests` 内の `#[test]` 関数 11 件 (l.678/686/697/709/720/737/789/799/816/825/839)
- 補助関数 `assert_missing` (l.773) / `assert_invalid` (l.780) — `RequiredFieldError` 系でのみ使用

### 集約対象外 (text_overlay.rs に残す)

- `parse_optional_z` (l.648-654) — 内部で `parse_optional_i64` を呼ぶ i64 → i32 範囲変換ラッパー (現状 z フィールドでのみ使用)。 集約後は import 経由で `parse_optional_i64` をそのまま呼ぶ
- `parse_optional_z` 関連 `#[test]` 2 件 (l.748 `parse_optional_z_rejects_out_of_i32_range` / l.762 `parse_optional_z_accepts_i32_max`、 `assert_missing` / `assert_invalid` を使わず内部で `parse_optional_z` のみ呼ぶ)
- 補助関数 `parse_owned_json` (l.672、 3 行) — 移動側 / 残置側の両方のテストで使用

### 本 issue のスコープ外として残す類似ヘルパー

obsws 配下には他にも類似パースヘルパーが分散している。 本 issue では集約対象外とし、 一部は別 issue で扱う (詳細は「## 関連」参照)。

- `coordinator.rs:1170 parse_required_non_empty_string_field` (`Option<String>` ベース、 機能重複)
- `output_registry.rs:829 parse_required_string` (`Option<String>` ベース、 機能重複)
- `output_registry.rs:845 parse_optional_string_strict` (null セマンティクスが矛盾)
- `response.rs:913 parse_required_i64_field_for_session` (session 系の別経路)

## 設計方針

### 配置場所と可視性

- `src/obsws/coordinator/parse_helpers.rs` を新規作成する
- `src/obsws/coordinator.rs` の `mod` 並びはアルファベット順 (`handle` / `input` / `output*` / `scene*` / `text_overlay`) のため、 `mod output_stream;` と `mod scene;` の間に `mod parse_helpers;` を挿入する
- 7 関数 / `RequiredFieldError` enum / `build_required_field_error_response` フリー関数 (後述) はすべて `pub(super)` とする。 参照元は `coordinator` モジュール内 (`coordinator.rs` 本体 + `coordinator/*.rs` 配下) を想定。 obsws 全体 (`obsws/response.rs` 等) に公開する必要が生じた時点で `pub(crate)` への昇格を別途判断する

### `map_required_field_error` のフリー関数化

`impl ObswsCoordinator::map_required_field_error` メソッド (l.441-462) を、 `parse_helpers.rs` の `pub(super)` フリー関数 `build_required_field_error_response` に置き換える。

シグネチャ:

```rust
pub(super) fn build_required_field_error_response(
    request_type: &str,
    request_id: &str,
    field_name: &str,
    error: RequiredFieldError,
) -> nojson::RawJsonOwned;
```

内部実装は `crate::obsws::response::build_request_response_error` (response.rs:1296、 既に `pub`) を直接呼ぶ。 parse_helpers.rs 側で以下の追加 import が必要 (`build_request_response_error` は修飾呼び出しでも可):

```rust
use crate::obsws::protocol::{REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD};
```

text_overlay.rs 内の呼び出し 7 箇所は以下の形に置き換える:

```rust
let response = build_required_field_error_response(request_type, request_id, "x", e);
return self.build_result_from_response(response, Vec::new());
```

**メソッド形式維持 (`impl ObswsCoordinator` を parse_helpers.rs に分割) 案を採らない理由**: パースヘルパー本体は `&self` を参照しない純粋関数で、 メソッド化は `build_error_result` 経由の `&self` 参照のためだけに残っている。 他ハンドラから使う際に `coordinator.build_required_field_error_response(...)` の形になり、 純粋関数の薄いヘルパー集という性質が見えにくくなる。

### 経路置換の意味的等価性 (外部応答仕様不変の根拠)

旧 `map_required_field_error` は `self.build_error_result` 経由で `RequestBatchResult` を引数値から直接構築する経路。 新 `build_required_field_error_response` は `build_request_response_error` で応答 JSON を生成し、 `build_result_from_response` で再パースして `RequestBatchResult` を組み立てる経路。 両経路で生成される `CommandResult` の各要素は完全一致する:

- 応答 JSON: 両者とも `build_request_response_error` を経由するためバイト互換 (`op` / `d.requestType` / `d.requestId` / `d.requestStatus.{result,code,comment}`)
- `RequestBatchResult`: `build_request_response_error` は常に `request_type` / `request_id` / `code` / `comment` を含む有効な JSON を生成し、 `build_result_from_response` 内の `parse_request_response_for_batch_result` は必ず引数値と等しい値を取り出すため、 fallback 分岐 (parse 失敗時の空フィールドフォールバック) には入らない
- `events`: いずれも `Vec::new()` で空

### テスト

text_overlay.rs:668-849 から「集約対象」項目の `#[test]` 11 件 + 補助関数 `assert_missing` / `assert_invalid` を parse_helpers.rs::tests に移す。 補助関数 `parse_owned_json` は parse_helpers.rs::tests / text_overlay.rs::tests の両方に同名・同シグネチャ・同実装の 3 行関数として独立定義する (同モジュール内の `#[cfg(test)] mod tests` は外部から `use` できないため、 重複定義以外に選択肢がない。 `src/obsws/coordinator/test_helpers.rs` を新設する案も検討したが、 3 行関数のためモジュール分割を増やすコストが上回ると判断)。

PBT 化は本 issue のスコープ外 (関連 5 で扱う)。

### doc comment の更新指針

集約先 (parse_helpers.rs) の汎用文脈に合わせて以下のみ更新する:

- `parse_required_string_field` の「`text` のように『空文字が valid 値』のフィールド向け」 (l.532) を汎用表現 (例: 「空文字を valid 値として透過するフィールド向け」) に書き換える
- `build_required_field_error_response` には新 doc comment を付ける (旧 `map_required_field_error` の「5 必須フィールド」「`CommandResult`」言及は引き継がない)

他 6 関数 / `RequiredFieldError` enum / 補助関数の doc comment は現状のまま移動する。

## 完了条件

### コード変更

- `src/obsws/coordinator/parse_helpers.rs` が新規作成され、 7 関数 / `RequiredFieldError` enum / `build_required_field_error_response` が `pub(super)` で配置されている
- parse_helpers.rs 内で `use crate::obsws::protocol::{REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD};` および `build_request_response_error` への参照 (use または修飾呼び出し) が追加されている
- `src/obsws/coordinator.rs` の `mod output_stream;` と `mod scene;` の間に `mod parse_helpers;` が追加されている
- `text_overlay.rs` から旧定義 (7 関数 / `RequiredFieldError` enum / `ObswsCoordinator::map_required_field_error` メソッド) が削除されている
- `text_overlay.rs` 冒頭の use 文から `REQUEST_STATUS_MISSING_REQUEST_FIELD` の import が削除されている (集約後 text_overlay.rs では `REQUEST_STATUS_INVALID_REQUEST_FIELD` のみ参照される。 削除しないと `cargo clippy -- -D warnings` が `unused_imports` で fail する)
- `text_overlay.rs` 冒頭で `super::parse_helpers` 配下の **8 シンボル** (7 関数 + `build_required_field_error_response`) を `use` で取り込む。 これらはハンドラ本体 (Create / Update / Remove) が個々の関数を呼ぶために必要で、 同じ `use` で `parse_optional_z` 内部の `parse_optional_i64(request_data, "z")?` 呼び出しも修飾なしで解決される。 形式は明示列挙 / `use super::parse_helpers::*;` のいずれでも可、 `cargo fmt` 通過後の改行はそれに従う
- `text_overlay.rs` の `self.map_required_field_error(...)` 呼び出し 7 箇所 (現状節で列挙) が `build_required_field_error_response(...)` + `self.build_result_from_response(response, Vec::new())` の組み合わせに置き換わっている
- 関連単体テスト (11 件 + 補助関数 `assert_missing` / `assert_invalid`) が parse_helpers.rs::tests に移されており、 `parse_owned_json` は両モジュールの `tests` に同一実装で独立定義されている

### 静的検査・テスト

- `cargo fmt --all --check` pass
- `cargo check --workspace --all-features --tests --benches` pass
- `cargo check --workspace --no-default-features` pass
- `cargo test --workspace --all-targets` pass
- `cargo test --workspace --no-default-features` pass
- `cargo clippy --workspace --all-targets -- -D warnings` pass
- `cargo clippy --workspace --no-default-features -- -D warnings` pass

### 客観的検証 (grep)

- 集約完了: `grep -rnE 'enum RequiredFieldError' src/obsws/` が parse_helpers.rs の 1 件のみ
- フリー関数の追加: `grep -rnE 'fn build_required_field_error_response' src/obsws/coordinator/` が parse_helpers.rs の 1 件のみ
- 旧定義の完全削除: `grep -nE 'fn (parse_required_string_field|parse_required_non_empty_string|parse_required_i64_field|parse_required_u32_field|parse_optional_string|parse_optional_i64|parse_optional_u32|map_required_field_error)' src/obsws/coordinator/text_overlay.rs` が 0 件
- `mod` 追加: `grep -nE '^mod parse_helpers;' src/obsws/coordinator.rs` が 1 件
- `parse_optional_z` の所在: `grep -rnE 'fn parse_optional_z' src/obsws/coordinator/` が text_overlay.rs の 1 件のみ (parse_helpers.rs には現れない)

### CHANGES.md

本 issue は内部リファクタで、 obsws の外部応答仕様 (応答 JSON / `RequestBatchResult` の値) は変化しない。 公開 API・state file 永続化フォーマットにも影響しない。 よって `CHANGES.md` への記載は行わない。

## 関連

関連 1, 2, 5 は本 issue が merge 済みの develop から派生ブランチを切る前提。 関連 3, 4 は別 issue で意味論を整理するもので本 issue とは独立。 open 0046 issue (`feature/refactor-clarify-processor-validation-boundary`) は対象ファイル群 (`src/rtmp/` / `src/srt/` / `src/rtsp/` / `src/obsws/state/types.rs` / `src/obsws/coordinator/output_rtmp.rs` / `src/obsws/coordinator/output_stream.rs`) が異なるため本 issue と並行進行可能。

1. **重複統合** (依存: 本 issue、 順序: 関連 2 の前): `coordinator.rs:1170 parse_required_non_empty_string_field` / `output_registry.rs:829 parse_required_string` を新ヘルパー `parse_required_non_empty_string` に置き換える。 `Option<String>` ベース → `RequiredFieldError` ベースへの移行で一部リクエストの外部応答が変化 (`MISSING_REQUEST_FIELD` → `INVALID_REQUEST_FIELD`) するため、 別 issue 起票時に対象範囲・カテゴリ・`CHANGES.md` 扱いを polish フェーズで詰める
2. **関数名 `_field` サフィックスの統一** (依存: 本 issue、 順序: 関連 1 の後): カテゴリ `refactor`
3. **`output_registry.rs:845 parse_optional_string_strict` の扱い**: null セマンティクスが text_overlay 流と矛盾するため別 issue で意味論を整理する
4. **`response.rs:913 parse_required_i64_field_for_session` の扱い**: session 系の `nojson::JsonParseError` ベースエラーパイプラインに乗っているため集約対象外として残すのが現実的だが別 issue で正式に判断する
5. **PBT 化** (依存: 本 issue、 `pub(super)` → `pub` への可視性昇格を併せて行う): `parse_helpers.rs` の 7 関数 (数値境界系を中心に) を `pbt/tests/prop_obsws_parse_helpers.rs` で PBT 化する。 pbt は外部 crate からの参照になるため、 関数本体の可視性に加えて到達経路上のモジュール (`obsws` / `obsws::coordinator`) の公開状況も併せて確認する
