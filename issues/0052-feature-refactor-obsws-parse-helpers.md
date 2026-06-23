# obsws ハンドラの汎用 JSON フィールド解析ヘルパーを共通モジュールに集約する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-obsws-parse-helpers
- Polished:

## 目的

obsws ハンドラ実装で使う汎用 JSON フィールド解析ヘルパー (`parse_required_string_field` / `parse_required_non_empty_string` / `parse_required_i64_field` / `parse_required_u32_field` / `parse_optional_string` / `parse_optional_i64` / `parse_optional_u32` および `RequiredFieldError` enum) を共通モジュールに集約して、 個別ハンドラからの再利用を容易にする。

## 優先度根拠

現状は機能的に動作しており、 即時の影響はない。 ただし新しい obsws リクエストハンドラを追加するたびに同種のパース関数を書きがち (もしくは text_overlay.rs から重複コピーしがち) なので、 共通化しておくことで将来の追加実装の品質と一貫性が上がる。 業務影響はないため Low。

## 現状

- 汎用 JSON フィールド解析ヘルパー 7 関数 + `RequiredFieldError` enum が `src/obsws/coordinator/text_overlay.rs:608-733` に集中している (定義箇所が text_overlay 専用モジュールであるため、 他ハンドラからの参照が不自然になる)。
- `src/obsws/coordinator.rs:1170` に **別の** `parse_required_non_empty_string_field` が定義されており、 text_overlay.rs の `parse_required_non_empty_string` と機能が重複している (関数名すら微妙に違う)。
- `text_overlay.rs::parse_optional_z` は i32 範囲チェックを含む text_overlay 固有のラッパーで、 これは集約対象外。

## 設計方針

- `src/obsws/coordinator/parse_helpers.rs` (新規、 もしくは既存の適切な場所) に汎用ヘルパー 7 関数 + `RequiredFieldError` enum を集約する。
- `coordinator.rs:1170` の `parse_required_non_empty_string_field` を新モジュール側の `parse_required_non_empty_string` に統合する (関数名は新側を採用、 呼び出し元を書き換える)。
- `text_overlay.rs::parse_optional_z` のような text_overlay 固有のラッパーは text_overlay.rs に残し、 共通ヘルパーを呼び出す形にする。
- `pub(crate)` または `pub(super)` で他のハンドラから参照可能にする。

## 完了条件

- 汎用ヘルパー 7 関数 + `RequiredFieldError` enum が共通モジュールに集約されている。
- `coordinator.rs` の `parse_required_non_empty_string_field` が削除され、 呼び出し元が共通ヘルパーに切り替わっている。
- 既存テスト全 pass (`cargo test --all-targets`)、 `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` も pass。
