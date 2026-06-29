# obsws のパースヘルパーを共通モジュールに集約しエラー応答メソッドを整理する

- Priority: Low
- Created: 2026-06-23
- Completed: 2026-06-26
- Model: Opus 4.7
- Branch: feature/refactor-obsws-parse-helpers
- Polished: 2026-06-26

## 目的

`src/obsws/coordinator/text_overlay.rs` に閉じている汎用 JSON フィールド解析ヘルパー群を、 共通モジュール `src/obsws/coordinator/parse_helpers.rs` に集約して他ハンドラから参照可能な形に整える。 あわせて、 必須 / オプション両系統のエラー応答生成メソッド (`ObswsCoordinator::build_required_field_error_result` / `build_invalid_field_error_result`) の整理、 テスト整理 (`matches!` 化 + 責務外 assert 削除)、 doc 整理 (規約明文化 + 重複圧縮) も同 issue で扱う。 obsws の外部応答仕様 (応答 JSON / `RequestBatchResult` の値) は変化しない内部リファクタとして扱う。

## 優先度根拠

機能的には現状で動作しているが、 同種のパース関数は既に複数ファイルに散らばっており (`text_overlay.rs` / `coordinator.rs` / `output_registry.rs` / `response.rs`)、 新しい obsws リクエストハンドラを追加するたびに重複が増殖する状況 (Broken Windows)。 業務影響はないため Low。

## 現状

### 集約対象 (text_overlay.rs から parse_helpers.rs に移動)

`src/obsws/coordinator/text_overlay.rs` に汎用 JSON フィールド解析ヘルパー 7 関数 + `RequiredFieldError` enum + `ObswsCoordinator::map_required_field_error` メソッドが集中している。

汎用 7 関数:

- `parse_required_string_field` — 必須文字列、 空文字も valid 値として透過
- `parse_required_non_empty_string` — 必須文字列、 空文字は `Missing` 扱い (識別子向け)
- `parse_required_i64_field` — 必須 i64
- `parse_required_u32_field` — 必須 u32 (i64 経由 + 範囲チェック)
- `parse_optional_string` — オプション文字列 (null は `Err` = 明示的指定を拒否)
- `parse_optional_i64` — オプション i64
- `parse_optional_u32` — オプション u32

関連型・メソッド:

- `RequiredFieldError` enum (バリアントは `Missing` / `Invalid(String)`)
- `ObswsCoordinator::map_required_field_error` メソッド (`RequiredFieldError` を `CommandResult` に変換)。 text_overlay.rs 内 7 箇所で呼び出し (Create 5 / Update 1 / Remove 1)
- 関連単体テスト 11 件 (7 関数を直接テスト) + 補助関数 `assert_missing` / `assert_invalid` (RequiredFieldError バリアント判別用)

### 集約対象外 (text_overlay.rs に残す)

- `parse_optional_z` — 内部で `parse_optional_i64` を呼ぶ i64 → i32 範囲変換ラッパー (現状 z フィールドでのみ使用)
- `parse_optional_z` 関連 `#[test]` 2 件 (`parse_optional_z_rejects_out_of_i32_range` / `parse_optional_z_accepts_i32_max`)
- 補助関数 `parse_owned_json` — 移動側 / 残置側の両方のテストで使用

### 本 issue のスコープ外として残す類似ヘルパー

obsws 配下には他にも類似パースヘルパーが分散している。 本 issue では集約対象外とし、 一部は別 issue で扱う (詳細は「## 関連」 参照)。

- `coordinator.rs` の `parse_required_non_empty_string_field` (`Option<String>` ベースで型違反を `None` に潰す別実装)
- `output_registry.rs:829 parse_required_string` (同上)
- `output_registry.rs:845 parse_optional_string_strict` (null セマンティクスが矛盾)
- `response.rs:913 parse_required_i64_field_for_session` (session 系の別経路)

## 設計方針

### 配置場所と可視性

- `src/obsws/coordinator/parse_helpers.rs` を新規作成する
- `src/obsws/coordinator.rs` の `mod` 並びはアルファベット順 (`handle` / `input` / `output*` / `scene*` / `text_overlay`) のため、 `mod output_stream;` と `mod scene;` の間に `mod parse_helpers;` を挿入する
- 7 関数および `RequiredFieldError` enum はすべて `pub(super)` とする。 メソッド (`build_required_field_error_result` / `build_invalid_field_error_result`) は `impl ObswsCoordinator` 内の private (可視性指定なし、 `coordinator` モジュール内からのみ呼べる) として coordinator.rs 側に置く
- 参照元は `coordinator` モジュール内 (`coordinator.rs` 本体 + `coordinator/*.rs` 配下) を想定。 obsws 全体 (`obsws/response.rs` 等) に公開する必要が生じた時点で `pub(crate)` への昇格を別途判断する

### メソッド整理 (`build_required_field_error_result` のリネーム + `build_invalid_field_error_result` の新設)

`map_required_field_error` メソッドを以下 2 つに整理する。 いずれも `build_error_result` を経由する旧経路を維持し、 外部応答仕様 (`CommandResult` の各値) は変化しない。

1. **`build_required_field_error_result` メソッド** (`map_required_field_error` のリネーム + 引数型を `parse_helpers::RequiredFieldError` に変更): `Missing` → `MISSING_REQUEST_FIELD` + `"Missing or empty {field_name} field"`、 `Invalid(message)` → `INVALID_REQUEST_FIELD` + `message` をそれぞれ `self.build_error_result(...)` で組み立てる
2. **`build_invalid_field_error_result` メソッド** (新設、 薄いラッパー): `self.build_error_result(request_type, request_id, REQUEST_STATUS_INVALID_REQUEST_FIELD, message)` 相当。 `parse_optional_*` の Err (`String`) を `INVALID_REQUEST_FIELD` で返す形式が text_overlay.rs 内に多数あるため、 メソッド化で 1 行化する

両メソッドの内部経路は `build_error_result` 経由で、 `RequestBatchResult` を引数値から直接組み立てる (JSON 再パース不要)。 不採用案の詳細は「## 解決方法 §「設計方針からの主な乖離点」」 参照。

### text_overlay.rs 内の置き換え

- 旧 `self.map_required_field_error(...)` 呼び出し (Create 5 / Update 1 / Remove 1 = 計 7 箇所) を `self.build_required_field_error_result(...)` の 1 行呼び出しに置換
- `parse_optional_*` Err 経路 (Create 3: fontColor / fontName / z、 Update 7: fontColor / text / x / y / fontSize / fontName / z = 計 10 箇所。 z の 2 箇所は集約対象外の `parse_optional_z` 経由だが Err 型が共通の `String` のため同じ統一対象とする) を `self.build_invalid_field_error_result(...)` の 1 行呼び出しに統一
- Update ハンドラの旧 `let invalid = |e: String| -> CommandResult { ... }` クロージャを削除
- 冒頭 use 文から `REQUEST_STATUS_MISSING_REQUEST_FIELD` を削除 (`text_overlay_error_status_code` 内で参照される `REQUEST_STATUS_INVALID_REQUEST_FIELD` は残す)
- 冒頭で `use super::parse_helpers::{...};` として 7 関数を取り込む (メソッドは `&self` 経由で呼ぶため import 不要)

### テスト整理

- 7 関数を直接テストする `#[test]` 11 件を `parse_helpers.rs::tests` に移動する
- 補助関数 `assert_missing` / `assert_invalid` は廃止し、 各 `#[test]` 内で `assert!(matches!(..., Err(RequiredFieldError::Missing)))` の形にインライン展開する
- `parse_required_string_field_classifies_failures` / `parse_required_i64_field_classifies_type_mismatch_as_invalid` / `parse_required_u32_field_rejects_negative` のテストは関数名と挙動を一致させ、 責務外の assert (Ok 値の往復確認など) を削除する (`parse_optional_*` 系の「missing / null / 値」 をまとめて検証するテストは『分類失敗』 ではないためそのまま残す)
- 補助関数 `parse_owned_json` は両モジュールの `tests` に同実装で独立定義する (`#[cfg(test)] mod tests` を跨いだ共有手段がないため)
- 集約対象外の `parse_optional_z` 関連 `#[test]` 2 件 (text_overlay.rs に残置) は本「テスト整理」 規範の対象外

### doc 整理

- 関数 doc の重複説明 (`Missing` / `Invalid` の振り分けなど) は `RequiredFieldError` 型 doc に集約する
- `RequiredFieldError::Invalid` のメッセージ規約 (`field '{name}' must be ...` 形式でフィールド名を含む) を `RequiredFieldError` 型 doc に明記する (応答 comment にそのまま使われるため生成元の `parse_required_*` 関数側で組み立てる規約)
- parse_helpers.rs モジュール冒頭 doc に「`RequiredFieldError` から `CommandResult` への変換は `ObswsCoordinator::build_required_field_error_result` が担う」 と記す
- テスト関数 doc のうち関数名翻訳に過ぎないものは削除し、 WHY を含むもののみ残す
- `parse_required_string_field` の旧 doc 「`text` のように『空文字が valid 値』 のフィールド向け」 を汎用表現 「空文字を valid 値として透過するフィールド向け」 に書き換える
- text_overlay.rs ハンドラ内コメント「// text は空文字も valid 値として扱う (バイト数 / 行数の上限は validate_text で確認する)。」 は `validate_text` が存在しない誤参照のため、 「ミキサー側で検証する」 に文言修正する

## 完了条件

### コード変更

- `src/obsws/coordinator/parse_helpers.rs` が新規作成され、 7 関数 + `RequiredFieldError` enum が `pub(super)` で配置されている
- `src/obsws/coordinator.rs` の `mod output_stream;` と `mod scene;` の間に `mod parse_helpers;` が追加されている
- `coordinator.rs` に `ObswsCoordinator::build_required_field_error_result` メソッド (`map_required_field_error` のリネーム) と `build_invalid_field_error_result` メソッド (新設) が追加されている
- `text_overlay.rs` から旧定義 (7 関数 / `RequiredFieldError` enum / `map_required_field_error` メソッド) が削除されている
- `text_overlay.rs` の `self.map_required_field_error(...)` 呼び出しがすべて `self.build_required_field_error_result(...)` に、 `parse_optional_*` Err 経路がすべて `self.build_invalid_field_error_result(...)` に置き換わっている (内訳は「## 設計方針 §「text_overlay.rs 内の置き換え」」 参照)
- Update ハンドラの旧 `let invalid = |e: String|` クロージャが削除されている
- 関連単体テスト 11 件が `parse_helpers.rs::tests` に移されており、 `assert_missing` / `assert_invalid` 廃止 + `matches!` 展開 + 「分類失敗」 テストの責務外 assert 削除が反映されている
- 補助関数 `parse_owned_json` が両モジュールの `tests` に独立定義され、 doc コメントで複製の旨が明記されている
- 「## 設計方針 §「doc 整理」」 のとおり doc / コメントが更新されている

### 静的検査・テスト

`cargo fmt --all --check` / `cargo check --workspace --tests --benches` および `--no-default-features` / `cargo test --workspace --all-targets` および `--no-default-features` / `cargo clippy --workspace --all-targets -- -D warnings` および `--no-default-features` がすべて pass。

### CHANGES.md

本 issue は内部リファクタで、 obsws の外部応答仕様 (応答 JSON / `RequestBatchResult` の値) は変化しない (新メソッドはいずれも旧 `build_error_result` 経由で同一の `CommandResult` を組み立てる)。 公開 API・state file 永続化フォーマットにも影響しない。 よって `CHANGES.md` への記載は行わない。

## 解決方法

ブランチ `feature/refactor-obsws-parse-helpers` 上で実装した。 主な内訳:

- `src/obsws/coordinator/parse_helpers.rs` を新規作成し、 7 関数 + `RequiredFieldError` enum を `pub(super)` で集約した
- `ObswsCoordinator` に `build_required_field_error_result` (旧 `map_required_field_error` のリネーム + 引数型を `parse_helpers::RequiredFieldError` に変更) と `build_invalid_field_error_result` (新設、 薄いラッパー) のメソッドを追加した
- `text_overlay.rs` の旧 `self.map_required_field_error(...)` 呼び出しと `parse_optional_*` Err 経路を新メソッド呼び出しに統一し、 Update ハンドラの `invalid` クロージャを削除した
- テストヘルパー `assert_missing` / `assert_invalid` を廃止して `assert!(matches!(...))` に展開し、 `parse_required_*` 系の責務外 assert を削除した
- 関数 doc / モジュール doc / `RequiredFieldError::Invalid` メッセージ規約 (`field '{name}' must be ...` 形式) を整理した

### 設計方針からの主な乖離点

初版実装ではフリー関数案 (`build_required_field_error_response`、 `build_request_response_error` 経由で応答 JSON を生成し `build_result_from_response` で再パースする経路) を採用したが、 review-diff-code レビューで「呼び出し側 7 箇所が 4 行に膨張」 「再パースのオーバーヘッド」 の指摘を受けて、 メソッド形式 (引数値から `RequestBatchResult` を直接組み立てる旧 `map_required_field_error` の経路を維持) に方針転換した。

## 関連

1. **重複統合** (依存: 本 issue、 順序: 関連 2 の前): `coordinator.rs` の `parse_required_non_empty_string_field` / `output_registry.rs:829 parse_required_string` を新ヘルパー `parse_required_non_empty_string` に置き換える。 統合に伴い型違反応答が `MISSING_REQUEST_FIELD` から `INVALID_REQUEST_FIELD` に厳密化されるため、 カテゴリと `CHANGES.md` 扱いは別 issue 起票時に確定する
2. **関数名 `_field` サフィックスの統一** (依存: 本 issue、 順序: 関連 1 の後): カテゴリ `refactor`
3. **`output_registry.rs:845 parse_optional_string_strict` の扱い**: null セマンティクスが text_overlay 流と矛盾するため別 issue で意味論を整理する
4. **`response.rs:913 parse_required_i64_field_for_session` の扱い**: session 系の `nojson::JsonParseError` ベースエラーパイプラインに乗っているため集約対象外として残すのが現実的だが別 issue で正式に判断する
5. **PBT 化** (依存: 本 issue、 可視性昇格を併せて行う): `parse_helpers.rs` の数値境界系 4 関数 (`parse_required_i64_field` / `parse_required_u32_field` / `parse_optional_i64` / `parse_optional_u32`) を `pbt/tests/prop_obsws_parse_helpers.rs` で PBT 化する。 文字列系 3 関数 (`parse_required_string_field` / `parse_required_non_empty_string` / `parse_optional_string`) は単体テストのまま、 メソッド (`build_*_error_result`) は `&self` を要するため PBT 対象外
