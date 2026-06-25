# hex 色解析ロジックを共通 Color 型に集約する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-hex-color
- Polished: 2026-06-24

## 目的

`#RRGGBB` / `#RRGGBBAA` 形式の hex 色文字列を扱う関数が obsws と webrtc にまたがって 4 個別実装で散らばっており、 受理範囲・出力型・エラー戦略が箇所ごとに異なる。 共通 `Color` 型に集約して、 新しい obsws リクエストハンドラや input source、 webrtc 経路で色フィールドを追加する際に同種のパースを書き直す状態を解消する。

## 優先度根拠

機能的には動作しており即時の業務影響はないため Low とする。

## 現状

### 関数定義 (4 個)

行番号は変動するため、 着手時に関数名で grep して再特定する。

- `src/obsws/coordinator/text_overlay.rs` `parse_argb_color(s: &str) -> Result<u32, TextOverlayError>`
  - 入力: `#RRGGBB` (A=0xFF として扱う) または `#RRGGBBAA`
  - 出力: ARGB u32 (`0xAARRGGBB` レイアウト = `(a<<24) | (r<<16) | (g<<8) | b`)
  - 失敗: `TextOverlayError::InvalidColor(String)` を以下 3 文言で出し分ける
    - `#` 不在: `format!("fontColor must start with '#': {s:?}")`
    - 6/8 桁以外: `format!("fontColor must be #RRGGBB or #RRGGBBAA: {s:?}")`
    - hex 以外: `format!("fontColor must be hex: {s:?}")`
- `src/obsws/coordinator/text_overlay.rs` `argb_to_hex_string(argb: u32) -> String`
  - 入力: ARGB u32 (`0xAARRGGBB`)
  - 出力: 常に 8 桁 `#RRGGBBAA` 文字列 (alpha=0xFF でも `FF` を付与、 hex は大文字 `{:02X}`)
- `src/obsws/source/webrtc_source.rs` `pub fn parse_hex_color(color: &str) -> Option<(u8, u8, u8)>`
  - 入力: `#RRGGBB` のみ (6 桁以外は `None`、 alpha 非対応)
  - 出力: RGB タプル
  - 失敗: `None` (エラー詳細を持たない)
  - 関数自体は `pub` だが親 `pub(crate) mod source` で覆われているため、 実態は crate 内のみ可視。 `src/webrtc/p2p_session.rs` からも参照されている
- `src/obsws/state/types.rs` `validate_hex_color(color: &Option<String>) -> Result<(), ParseInputSettingsError>`
  - 内部実装は `webrtc_source::parse_hex_color` を呼んで `is_none()` を見るだけの薄いラッパ
  - 受理範囲は `#RRGGBB` のみ
  - 失敗: `ParseInputSettingsError::InvalidInputSettings(format!("invalid color format: expected #RRGGBB, got {c}"))`

### 呼び出し元

- `src/obsws/coordinator/text_overlay.rs`
  - `parse_argb_color` 2 箇所
    - Create 経路 `handle_create_text_overlay`: `Ok(v) => v` で `font_color_argb: u32` に直接代入
    - Update 経路 `handle_update_text_overlay`: `Ok(v) => Some(v)` で `font_color_argb: Option<u32>` にラップ
  - `argb_to_hex_string` 1 箇所 (`text_overlay_state_to_json` の `fontColor` レスポンス組み立て)
- `src/obsws/source/color_source.rs` `parse_hex_color` 1 箇所 (`ColorSource::run` で BT.601 経由 I420 フレームを生成)
  - 同ファイルに `const DEFAULT_COLOR: &str = "#000000"` (color 設定欠落時のデフォルト)
- `src/obsws/state/types.rs` `validate_hex_color` 2 箇所 (`from_input_settings` と `overlay_with_settings` の `color_source` 分岐)
- `src/webrtc/p2p_session.rs` `parse_hex_color` 1 箇所 (`resolve_chroma_key_config` で `background_key_color` を扱う。 同関数内で `webrtc_source::rgb_to_uv_bt601` も併用するが本 issue では触らない)

### 非対称性

- 受理桁数: `parse_argb_color` のみ 6/8 桁両対応、 他は 6 桁のみ
- 出力型: `u32` (ARGB) / `(u8, u8, u8)` / `()`
- 失敗表現: `Result<_, TextOverlayError>` / `Option` / `Result<_, ParseInputSettingsError>`
- alpha: `parse_argb_color` のみ保持、 他は無視

## 設計方針

### 配置場所とモジュール登録

`src/color.rs` を新規作成し、 `src/lib.rs` に `pub mod color;` を追加する。 `Color` / `ColorParseError` / 全メソッド / 全フィールドは `pub` で公開する (`crate::color::Color::...` の形で hisui crate 内の全呼び出し元から参照される)。

PBT は別 crate (`pbt`) に置く既存設計 (proptest 依存を hisui 本体に持ち込まないため) に従うため、 PBT が `use hisui::color::{Color, ColorParseError};` で参照できるよう `pub` での公開を選ぶ。 `pub(crate)` だと別 crate からの参照が成立しない。 既存の `pub mod audio;` / `pub mod media;` 等のドメイン汎用名モジュールと整合する。 将来の breaking は `shiguredo-rust` 規約 (`#[non_exhaustive]` 禁止、 「将来 variant や field を追加するときは素直に破壊的変更として扱う」) に沿って `CHANGES.md` に記載する形で運用する。

obsws 配下 (`src/obsws/color.rs`) ではなく crate root に置く理由は以下:

- 呼び出し元が obsws 内 (`text_overlay.rs` / `color_source.rs` / `state/types.rs`) だけでなく webrtc 内 (`p2p_session.rs::resolve_chroma_key_config`) にもまたがる
- パース対象の hex 文字列フォーマット (`#RRGGBB` / `#RRGGBBAA`) は OBS WebSocket 固有ではなく汎用 (CSS 標準) なため、 obsws 専用モジュールに置くと命名と責務が乖離する
- 現状すでに発生している `webrtc → obsws::source` のクロスレイヤ依存 (hex 色解析だけのために存在) を解消できる
- 既存の `src/lib.rs` には `pub mod audio;` `pub mod media;` 等のドメイン汎用名モジュールが並ぶ前例があり、 `pub mod color;` を crate root に追加するスタイルに整合する

モジュールの doc コメントで「現状は obsws と webrtc/p2p_session から使う汎用 hex 色型」と明示し、 過剰汎化と誤読されないようにする。

### Color 型

統一型 1 つで定義する。 `Rgb` / `Rgba` 2 型分離案は採用しない (呼び出し側に「alpha が要るか」の分岐が増えるため)。

```rust
/// hex 文字列由来の色値。 alpha は常に保持し、 `#RRGGBB` 入力時は 0xFF (不透明) を埋める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// `#RRGGBB` (a=0xFF として扱う) と `#RRGGBBAA` の両方を受け付ける。
    /// 空文字は `MissingHashPrefix` で拒否する。
    pub fn from_hex(s: &str) -> Result<Self, ColorParseError>;

    /// `#RRGGBB` のみ受け付ける。 6 桁以外は `InvalidLength(stripped.len())` で拒否する
    /// (8 桁を渡しても `InvalidLength(8)` で拒否される)。
    /// 空文字 / `#` 不在は `MissingHashPrefix` で拒否する (`from_hex` と同じ挙動)。
    /// 成功時は `a = 0xFF` を埋める。
    pub fn from_hex_rgb(s: &str) -> Result<Self, ColorParseError>;

    /// ARGB u32 (`0xAARRGGBB` レイアウト) から Color を構築する。 無謬な変換。
    pub const fn from_argb_u32(argb: u32) -> Self;

    /// 常に 8 桁 `#RRGGBBAA` を出力する (alpha=0xFF でも `FF` を付与、 hex は大文字)。
    pub fn to_hex_string(&self) -> String;

    /// ARGB u32 (`((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)`) を返す。
    pub const fn to_argb_u32(&self) -> u32;

    /// `(r, g, b)` 順のタプル。 alpha は捨てる。
    pub const fn rgb_tuple(&self) -> (u8, u8, u8);
}
```

### ColorParseError 型

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorParseError {
    /// `#` プレフィックス不在 (空文字もここに分類)。
    MissingHashPrefix,
    /// `#` を除いた後の文字数が 6 / 8 以外。 引数値は `#` を除いた後の長さ。
    InvalidLength(usize),
    /// hex 以外の文字が含まれる。
    InvalidHex,
}
```

`Display` / `std::error::Error` は実装しない。 呼び出し元はバリアントごとに文言整形を行う (後述)。

入力の大文字・小文字は両方受理する (`u8::from_str_radix` / `u32::from_str_radix` の挙動を維持)。 出力は大文字固定。

### 受理範囲の維持

本 issue は refactor であり、 既存の入力受理範囲を変えない。

- `fontColor` (text_overlay の create / update) は `Color::from_hex` を使う (`#RRGGBB` / `#RRGGBBAA` 両対応)
- `color_source` の `color`, `webrtc_source` の `background_key_color`, `state/types.rs::validate_hex_color` は `Color::from_hex_rgb` を使う (`#RRGGBB` のみ)

将来 `color_source` 等で `#RRGGBBAA` も受理する変更は別途 change カテゴリの issue として起票する (本 issue では扱わない)。

### エラー文言の整形

`text_overlay.rs` 側だけが `ColorParseError` の 3 バリアントを別文言に出し分ける (`fontColor must ...` 系)。 `validate_hex_color` 側は単一文言 (`expected #RRGGBB, got ...`) で出すため分岐不要。 したがって整形ヘルパは `text_overlay.rs` 内 private fn として配置する。 現状は `fontColor` 専用だが、 将来 `backgroundColor` 等の hex 色フィールドが追加されたときに同じヘルパを再利用できるよう `field` を引数化しておく:

```rust
/// `Color::from_hex` 失敗時に既存 `parse_argb_color` 互換の文言を組み立てる。
fn format_color_parse_error(field: &str, input: &str, e: ColorParseError) -> String {
    match e {
        ColorParseError::MissingHashPrefix => format!("{field} must start with '#': {input:?}"),
        ColorParseError::InvalidLength(_) => format!("{field} must be #RRGGBB or #RRGGBBAA: {input:?}"),
        ColorParseError::InvalidHex => format!("{field} must be hex: {input:?}"),
    }
}
```

### 各呼び出し元の書き換え

#### text_overlay.rs Create 経路 (`handle_create_text_overlay`)

既存の 2 段 match (`parse_optional_string → parse_argb_color`) を維持し、 内側だけ差し替える。 戻り値は `font_color_argb: u32` に直接代入する。

```rust
let font_color_argb = match parse_optional_string(request_data, "fontColor") {
    Ok(Some(s)) => match Color::from_hex(&s) {
        Ok(c) => c.to_argb_u32(),
        Err(e) => {
            return self.build_text_overlay_error_result(
                request_type,
                request_id,
                TextOverlayError::InvalidColor(format_color_parse_error("fontColor", &s, e)),
            );
        }
    },
    Ok(None) => DEFAULT_FONT_COLOR_ARGB,
    Err(e) => return self.build_error_result(...),
};
```

#### text_overlay.rs Update 経路 (`handle_update_text_overlay`)

戻り値型は `font_color_argb: Option<u32>`。 `Ok(c) => Some(c.to_argb_u32())` で `Option` にラップする。

```rust
let font_color_argb = match parse_optional_string(request_data, "fontColor") {
    Ok(Some(s)) => match Color::from_hex(&s) {
        Ok(c) => Some(c.to_argb_u32()),
        Err(e) => {
            return self.build_text_overlay_error_result(
                request_type,
                request_id,
                TextOverlayError::InvalidColor(format_color_parse_error("fontColor", &s, e)),
            );
        }
    },
    Ok(None) => None,
    Err(e) => return self.build_error_result(...),
};
```

#### text_overlay.rs `text_overlay_state_to_json`

```rust
f.member("fontColor", Color::from_argb_u32(spec.font_color_argb).to_hex_string())?;
```

#### color_source.rs `ColorSource::run`

旧 `use super::webrtc_source::parse_hex_color;` を削除し、 `use crate::color::Color;` を追加する。

```rust
let (r, g, b) = Color::from_hex_rgb(&self.color)
    .map_err(|_| crate::Error::new(format!("invalid color format: {}", self.color)))?
    .rgb_tuple();
```

#### state/types.rs `validate_hex_color`

関数自体は残し、 中身を差し替える。 シグネチャ (`&Option<String>` 受け取り) は変更しない (呼び出し元 2 箇所のスタイルを維持するため)。

```rust
fn validate_hex_color(color: &Option<String>) -> Result<(), ParseInputSettingsError> {
    if let Some(c) = color {
        crate::color::Color::from_hex_rgb(c).map_err(|_| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "invalid color format: expected #RRGGBB, got {c}"
            ))
        })?;
    }
    Ok(())
}
```

#### p2p_session.rs `resolve_chroma_key_config`

関数全体の戻り値型 `Option<ChromaKeyConfig>` は変えない。 `Color::from_hex_rgb` の `Result` を `.ok()?` で `Option` セマンティクスに合わせる。 `rgb_to_uv_bt601` 呼び出しは維持する。

```rust
let color = background_key_color?;
let tolerance = background_key_tolerance?;
let (r, g, b) = crate::color::Color::from_hex_rgb(color).ok()?.rgb_tuple();
let (key_u, key_v) = crate::obsws::source::webrtc_source::rgb_to_uv_bt601(r, g, b);
```

### スコープ外

以下は本 issue では扱わない。

- `webrtc_source::rgb_to_uv_bt601` および `webrtc_source::apply_chroma_key`: RGB → YUV 変換と alpha plane 生成であり、 hex 解析とは別責務のため `color.rs` には移さず `webrtc_source.rs` に残す
- `crate::video::rgb_to_yuv_bt601_int` と `webrtc_source::rgb_to_uv_bt601` の重複: BT.601 変換 2 系統共存の解消は別 issue
- `TextOverlaySpec::font_color_argb: u32` のフィールド型変更: mixer / layer / validate まで波及するため触らない
- `color_source.rs` の `const DEFAULT_COLOR: &str = "#000000"` は `String` のまま維持する (`Color::BLACK` 等の定数化は将来検討)

## テスト方針

### PBT (`pbt/tests/prop_color.rs` を新規作成)

ラウンドトリップなど proptest で範囲網羅できるプロパティを書く。 関数名は既存 `pbt/tests/prop_text_overlay.rs` の説明的命名スタイル (`<対象>_<挙動>_<条件>`) に揃える。 strategy は `[0-9A-Fa-f]{6}` / `[0-9A-Fa-f]{8}` のように hex 文字限定で生成する (hex 以外の文字列は単体テストの責務)。

- `argb_u32_roundtrips_via_color` 任意 u32 で `from_argb_u32 → to_argb_u32` が元値と一致する
- `argb_u32_roundtrips_via_hex_string` 任意 u32 で `from_argb_u32 → to_hex_string → from_hex → to_argb_u32` が元値と一致する
- `from_hex_and_from_hex_rgb_agree_on_6digit` 任意 `#RRGGBB` 文字列 (strategy `[0-9A-Fa-f]{6}` を `#` プレフィックス付きで生成) で `from_hex` と `from_hex_rgb` がともに成功し、 結果の `Color` が一致する (両者とも `a = 0xFF`)
- `from_hex_accepts_8digit_but_from_hex_rgb_rejects` 任意 `#RRGGBBAA` 文字列 (strategy `[0-9A-Fa-f]{8}` を `#` プレフィックス付きで生成) で `from_hex` は成功、 `from_hex_rgb` は必ず `Err(ColorParseError::InvalidLength(8))` を返す
- `both_parsers_reject_missing_hash` `#` を含まない任意 hex 文字列 (strategy `[0-9A-Fa-f]{0,16}`) で `from_hex` と `from_hex_rgb` がともに `Err(ColorParseError::MissingHashPrefix)` を返す

### 単体テスト (`src/color.rs` の `mod tests`)

PBT で実現しにくいエラーパス・境界値のみを書く。

- `from_hex_accepts_lowercase` (大文字小文字混在の `#abCD12` が受理されることを明示確認)
- `from_hex_rejects_empty` (`""` が `MissingHashPrefix`)
- `from_hex_rejects_non_hex_chars` (`#GGGGGG` が `InvalidHex`)
- `from_hex_rejects_wrong_length` (3 桁 `#FFF` と 9 桁 `#FFFFFFFFF` がそれぞれ `InvalidLength(3)` / `InvalidLength(9)`)
- `to_hex_string_always_8_digits_uppercase` (`a=0xFF` でも `FF` が付き、 hex は大文字。 代表色 `#FF0000FF` / `#00FF00FF` / `#0000FFFF` / `#000000FF` / `#FFFFFFFF` を列挙)

### 既存テストの扱い

- `text_overlay.rs` の `parse_argb_color_handles_*` / `_rejects_*` / `argb_to_hex_string_roundtrip` は新規 PBT + 単体テストでカバーされるため削除する (旧関数自体が削除されるため)
- `webrtc_source.rs::test_parse_hex_color` も同様に削除する
- `color_source.rs` の `color_source_emits_i420_frames` / `build_record_source_plan_uses_*_color` は間接的に `Color::from_hex_rgb` を経由する形で残す
- `obsws/session/tests.rs::hisui_create_text_overlay_rejects_invalid_color` (status code を assert する統合テスト) はそのまま残す
- `state/types.rs::validate_hex_color` および `p2p_session.rs::resolve_chroma_key_config` には現状直接ユニットテストがない (統合経路でのみカバー) 状態を変えない

## 後方互換 (非ゴール)

- state file に保存される `color` / `background_key_color` の文字列フォーマット (`#RRGGBB`) は変更しない
- obsws レスポンスの `fontColor` フィールドは引き続き常に 8 桁 `#RRGGBBAA` 形式・大文字で出力する
- 既存エラー応答の comment 文言 (`fontColor must start with '#'` / `fontColor must be #RRGGBB or #RRGGBBAA` / `fontColor must be hex` / `expected #RRGGBB, got ...`) は維持する
- `TextOverlaySpec::font_color_argb: u32` のフィールド型は変更しない (mixer / layer / validate への波及を避ける)

## 完了条件

- `src/color.rs` が新規作成され、 `Color` / `ColorParseError` と上記 API が定義されている
- `src/lib.rs` に `pub mod color;` が追加されている
- 上記「各呼び出し元の書き換え」のとおり、 obsws / webrtc 配下の全呼び出し元 (`text_overlay.rs` の Create / Update / state_to_json / `color_source.rs` / `state/types.rs` / `p2p_session.rs`) が `Color` 経由に切り替わっている
- 旧 `pub fn parse_hex_color` (`webrtc_source.rs`)、 `parse_argb_color` / `argb_to_hex_string` (`text_overlay.rs`) が削除されている
- `validate_hex_color` (`state/types.rs`) は関数として残り、 内部が `Color::from_hex_rgb` 経由に置き換わっている
- `pbt/tests/prop_color.rs` が新規作成され、 上記 PBT プロパティを含む
- `grep -rEn 'parse_hex_color|parse_argb_color|argb_to_hex_string' src/ pbt/ tests/ examples/ fuzz/` で `src/color.rs` および `pbt/tests/prop_color.rs` 以外に出現がない
- 既存の受理範囲・拒否範囲・主要エラー文言キーワード (`fontColor must` / `expected #RRGGBB`) が変わっていない
- `cargo test --all-targets` がすべて通る
- `cargo fmt --check` および `cargo clippy --all-targets -- -D warnings` がすべて通る

## 関連

- open `issues/0052-feature-refactor-obsws-parse-helpers.md` (JSON フィールド解析ヘルパー集約) と同じ `text_overlay.rs` を編集するが、 対象範囲が異なる (0052 は JSON ヘルパ群、 本 issue は hex 色関連) のでマージ順序は問わない

## CHANGES.md について

内部リファクタであり外部から観測可能な挙動 (state file フォーマット、 obsws レスポンス、 受理範囲、 主要エラー文言キーワード) は変えないため `CHANGES.md` には記載しない (`shiguredo-changelog` 規約準拠)。
