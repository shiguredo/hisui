# obsws 内に散らばる hex 色解析ロジックを共通 Color 型に集約する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-obsws-hex-color
- Polished:

## 目的

obsws 周辺で `#RRGGBB` / `#RRGGBBAA` 形式の hex 色文字列の解析・検証・フォーマット処理が 3 箇所に散らばっており、 出力型もバラバラになっている。 共通 Color 型と変換 API に集約して保守性を上げる。

## 優先度根拠

機能的には動作しており即時の影響はない。 ただし新しい obsws リクエストハンドラや input source で色フィールドを追加するたびに同種のパースを書く / コピペする状態のため、 集約しておくことで将来の追加実装の品質と一貫性が上がる。 業務影響はないため Low。

## 現状

3 箇所に同種の hex 色解析処理が存在する:

- `src/obsws/coordinator/text_overlay.rs:580` `parse_argb_color(s: &str) -> Result<u32, TextOverlayError>`: `#RRGGBB` / `#RRGGBBAA` → ARGB u32 (`0xAARRGGBB`)
- `src/obsws/coordinator/text_overlay.rs:570` `argb_to_hex_string(argb: u32) -> String`: ARGB u32 → `#RRGGBBAA` 文字列
- `src/obsws/source/webrtc_source.rs:72` `parse_hex_color(color: &str) -> Option<(u8, u8, u8)>`: `#RRGGBB` → RGB タプル (ALPHA 非対応)
- `src/obsws/state/types.rs:186, 320` `validate_hex_color(&color)?`: `#RRGGBB` の検証のみ (値変換は別経路)

入力形式 (`#RRGGBB` の hex) は同じだが、 出力型 (`u32` / `(u8,u8,u8)` / 検証のみ) と ALPHA 対応の有無が違うため共通化されていない。

## 設計方針

- `src/obsws/color.rs` (新規、 もしくは適切な共通モジュール) に色型を集約する。
- 統一型 `Color` を 1 つ定義 (alpha は `Option<u8>` で省略可、 default は不透明白 等) する案か、 `Rgb` / `Rgba` の 2 型を別々に持つ案かは実装時に決定する。
- パース API (`from_hex(s: &str) -> Result<Color, ColorParseError>`) と フォーマット API (`to_hex_string(&self) -> String`、 ALPHA の有無で出力切替) を提供する。
- 既存 3 箇所の呼び出し元 (`parse_argb_color` / `argb_to_hex_string` / `parse_hex_color` / `validate_hex_color`) を新型経由に切り替える。

## 完了条件

- 共通 Color 型が定義され、 既存 3 箇所が新型経由に切り替わっている。
- 既存テスト全 pass (`cargo test --all-targets`)、 `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` も pass。
