# hex 色解析ロジックを共通 Color 型に集約する

- Priority: Low
- Created: 2026-06-23
- Completed: 2026-06-25
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

`src/color.rs` を新規作成し、 `src/lib.rs` に `pub mod color;` を追加する。 `Color` / 全メソッド / 全フィールドは `pub` で公開する (`crate::color::Color::...` の形で hisui crate 内の全呼び出し元から参照される)。

PBT は別 crate (`pbt`) に置く既存設計 (proptest 依存を hisui 本体に持ち込まないため) に従うため、 PBT が `use hisui::color::Color;` で参照できるよう `pub` での公開を選ぶ。 `pub(crate)` だと別 crate からの参照が成立しない。 既存の `pub mod audio;` / `pub mod media;` 等のドメイン汎用名モジュールと整合する。 将来の breaking は `shiguredo-rust` 規約 (`#[non_exhaustive]` 禁止、 「将来 variant や field を追加するときは素直に破壊的変更として扱う」) に沿って `CHANGES.md` に記載する形で運用する。

obsws 配下 (`src/obsws/color.rs`) ではなく crate root に置く理由は以下:

- 呼び出し元が obsws 内 (`text_overlay.rs` / `color_source.rs` / `state/types.rs`) だけでなく webrtc 内 (`p2p_session.rs::resolve_chroma_key_config`) にもまたがる
- パース対象の hex 文字列フォーマット (`#RRGGBB` / `#RRGGBBAA`) は OBS WebSocket 固有ではなく汎用 (CSS 標準) なため、 obsws 専用モジュールに置くと命名と責務が乖離する
- 現状すでに発生している `webrtc → obsws::source` のクロスレイヤ依存 (hex 色解析だけのために存在) を解消できる
- 既存の `src/lib.rs` には `pub mod audio;` `pub mod media;` 等のドメイン汎用名モジュールが並ぶ前例があり、 `pub mod color;` を crate root に追加するスタイルに整合する

モジュールの doc コメントで「現状は obsws と webrtc/p2p_session から使う汎用 hex 色型」と明示し、 過剰汎化と誤読されないようにする。

### Color 型

統一型 1 つで定義する。 `Rgb` / `Rgba` 2 型分離案は採用しない (呼び出し側に「alpha が要るか」の分岐が増えるため)。

失敗種別を呼び出し元が区別しないため、 戻り値型は `Option<Self>` で表現する。 細かい失敗種別 (`#` 不在 / 桁数違い / hex 以外) を別文言に出し分ける呼び出し元はなく、 hex 色表記のルール自体がシンプルで「間違っている」とだけ伝われば十分。 専用エラー enum (`ColorParseError` 等) は定義しない。

```rust
/// hex 文字列由来の色値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// `#RRGGBB` (a=0xFF として扱う) と `#RRGGBBAA` の両方を受け付ける。
    /// 6 / 8 桁以外 / 空文字 / `#` 不在 / hex 以外の文字を含む場合はすべて `None` を返す。
    /// 大文字・小文字いずれの hex も受理する。
    pub fn from_hex(s: &str) -> Option<Self>;

    /// `#RRGGBB` のみ受け付ける。 6 桁以外 / 空文字 / `#` 不在 / hex 以外の文字を
    /// 含む場合はすべて `None` を返す。 成功時は `a = 0xFF` を埋める。
    pub fn from_hex_rgb(s: &str) -> Option<Self>;

    /// ARGB u32 (`0xAARRGGBB` レイアウト) から `Color` を構築する。 失敗しない変換。
    pub fn from_argb_u32(argb: u32) -> Self;

    /// 常に 8 桁 `#RRGGBBAA` を出力する (alpha=0xFF でも `FF` を付与、 hex は大文字)。
    pub fn to_hex_string(&self) -> String;

    /// ARGB u32 (`((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)`) を返す。
    pub fn to_argb_u32(&self) -> u32;

    /// `(r, g, b)` 順のタプル。 alpha は捨てる。
    pub fn to_rgb(&self) -> (u8, u8, u8);
}
```

入力の大文字・小文字は両方受理する (`u8::from_str_radix` の挙動を維持)。 出力は大文字固定。

### 受理範囲の維持

本 issue は refactor であり、 既存の入力受理範囲を変えない。

- `fontColor` (text_overlay の create / update) は `Color::from_hex` を使う (`#RRGGBB` / `#RRGGBBAA` 両対応)
- `color_source` の `color`, `webrtc_source` の `background_key_color`, `state/types.rs::validate_hex_color` は `Color::from_hex_rgb` を使う (`#RRGGBB` のみ)

将来 `color_source` 等で `#RRGGBBAA` も受理する変更は別途 change カテゴリの issue として起票する (本 issue では扱わない)。

### エラー文言

`fontColor` のエラー文言は呼び出し元 (`text_overlay.rs`) で 1 種類 (`fontColor must be #RRGGBB or #RRGGBBAA: ...`) に統合する。 `#` 忘れ・桁数違い・hex 以外のいずれの入力ミスでも同じ文言を返すが、 hex 色表記のルールはシンプルなので利用者は文言だけで自分で気づける。 旧 `parse_argb_color` の 3 種別出し分けは廃止する。

`validate_hex_color` (state/types.rs) は引き続き単一文言 (`invalid color format: expected #RRGGBB, got ...`) を維持する。

### 各呼び出し元の書き換え

#### text_overlay.rs Create 経路 (`handle_create_text_overlay`)

既存の 2 段 match (`parse_optional_string → parse_argb_color`) を維持し、 内側を `Color::from_hex` + `Option` 分岐に差し替える。 戻り値は `font_color_argb: u32` に直接代入する。

```rust
let font_color_argb = match parse_optional_string(request_data, "fontColor") {
    Ok(Some(s)) => match Color::from_hex(&s) {
        Some(c) => c.to_argb_u32(),
        None => {
            return self.build_text_overlay_error_result(
                request_type,
                request_id,
                TextOverlayError::InvalidColor(format!(
                    "fontColor must be #RRGGBB or #RRGGBBAA: {s:?}"
                )),
            );
        }
    },
    Ok(None) => DEFAULT_FONT_COLOR_ARGB,
    Err(e) => return self.build_error_result(...),
};
```

#### text_overlay.rs Update 経路 (`handle_update_text_overlay`)

戻り値型は `font_color_argb: Option<u32>`。 `Some(c) => Some(c.to_argb_u32())` で `Option` にラップする。

```rust
let font_color_argb = match parse_optional_string(request_data, "fontColor") {
    Ok(Some(s)) => match Color::from_hex(&s) {
        Some(c) => Some(c.to_argb_u32()),
        None => {
            return self.build_text_overlay_error_result(
                request_type,
                request_id,
                TextOverlayError::InvalidColor(format!(
                    "fontColor must be #RRGGBB or #RRGGBBAA: {s:?}"
                )),
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

旧 `use super::webrtc_source::parse_hex_color;` を削除し、 `use crate::color::Color;` を追加する。 `Option` を `.ok_or_else(...)` で `Result` に橋渡しする。

```rust
let (r, g, b) = Color::from_hex_rgb(&self.color)
    .ok_or_else(|| crate::Error::new(format!("invalid color format: {}", self.color)))?
    .to_rgb();
```

#### state/types.rs `validate_hex_color`

関数自体は残し、 中身を差し替える。 シグネチャ (`&Option<String>` 受け取り) は変更しない。

```rust
fn validate_hex_color(color: &Option<String>) -> Result<(), ParseInputSettingsError> {
    if let Some(c) = color {
        crate::color::Color::from_hex_rgb(c).ok_or_else(|| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "invalid color format: expected #RRGGBB, got {c}"
            ))
        })?;
    }
    Ok(())
}
```

#### p2p_session.rs `resolve_chroma_key_config`

関数全体の戻り値型 `Option<ChromaKeyConfig>` は変えない。 `Color::from_hex_rgb` の戻り値が `Option<Color>` なので、 そのまま `?` で `None` を伝播できる。 `rgb_to_uv_bt601` 呼び出しは維持する。

```rust
let color = background_key_color?;
let tolerance = background_key_tolerance?;
let (r, g, b) = crate::color::Color::from_hex_rgb(color)?.to_rgb();
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

ラウンドトリップなど proptest で範囲網羅できるプロパティを書く。 関数名は既存 `pbt/tests/prop_text_overlay.rs` の説明的命名スタイル (`<対象>_<挙動>_<条件>`) に揃える。

- `argb_u32_roundtrips_via_color` 任意 u32 で `from_argb_u32 → to_argb_u32` が元値と一致する
- `argb_u32_roundtrips_via_hex_string` 任意 u32 で `from_argb_u32 → to_hex_string → from_hex → to_argb_u32` が元値と一致する
- `to_hex_string_matches_uppercase_8digit_pattern` 任意 u32 → `from_argb_u32 → to_hex_string` が `#` + 8 桁大文字 hex の構造を持つ
- `from_hex_and_from_hex_rgb_agree_on_6digit` 任意 `#RRGGBB` 文字列 (`[0-9A-Fa-f]{6}`) で `from_hex` と `from_hex_rgb` がともに成功し、 結果の `Color` が一致する (両者とも `a = 0xFF`)
- `from_hex_accepts_8digit_but_from_hex_rgb_rejects` 任意 `#RRGGBBAA` 文字列 (`[0-9A-Fa-f]{8}`) で `from_hex` は `Some`、 `from_hex_rgb` は `None` を返す
- `from_hex_rejects_non_6_or_8_lengths` 任意の 6 / 8 桁以外の hex (`[0-9A-Fa-f]{1,5}|[0-9A-Fa-f]{7}|[0-9A-Fa-f]{9,16}`) で `from_hex` は `None` を返す
- `from_hex_rgb_rejects_non_6_lengths` 任意の 6 桁以外の hex (`[0-9A-Fa-f]{1,5}|[0-9A-Fa-f]{7,16}`) で `from_hex_rgb` は `None` を返す
- `both_parsers_reject_missing_hash` `#` を含まない任意 hex 文字列 (`[0-9A-Fa-f]{0,16}`) で `from_hex` / `from_hex_rgb` がともに `None` を返す

### 単体テスト (`src/color.rs` の `mod tests`)

PBT で実現しにくいエラーパス・境界値のみを書く。

- `from_hex_rejects_non_hex_chars` (`#GGGGGG` が `None`)
- `from_hex_rejects_wrong_length` (3 桁 `#FFF` と 9 桁 `#FFFFFFFFF` がいずれも `None`)

### 既存テストの扱い

- `text_overlay.rs` の `parse_argb_color_handles_*` / `_rejects_*` / `argb_to_hex_string_roundtrip` は新規 PBT + 単体テストでカバーされるため削除する (旧関数自体が削除されるため)
- `webrtc_source.rs::test_parse_hex_color` も同様に削除する
- `color_source.rs` の `color_source_emits_i420_frames` / `build_record_source_plan_uses_*_color` は間接的に `Color::from_hex_rgb` を経由する形で残す
- `obsws/session/tests.rs::hisui_create_text_overlay_rejects_invalid_color` (status code を assert する統合テスト) はそのまま残す
- `state/types.rs::validate_hex_color` および `p2p_session.rs::resolve_chroma_key_config` には現状直接ユニットテストがない (統合経路でのみカバー) 状態を変えない

## 後方互換 (非ゴール)

- state file に保存される `color` / `background_key_color` の文字列フォーマット (`#RRGGBB`) は変更しない
- obsws レスポンスの `fontColor` フィールドは引き続き常に 8 桁 `#RRGGBBAA` 形式・大文字で出力する
- `fontColor` の不正入力時のエラー文言は `fontColor must be #RRGGBB or #RRGGBBAA: ...` の 1 種類に統合する (旧 `parse_argb_color` の 3 種別出し分け文言は廃止)。 `expected #RRGGBB, got ...` (`color_source` 等の検証エラー文言) は維持する
- `TextOverlaySpec::font_color_argb: u32` のフィールド型は変更しない (mixer / layer / validate への波及を避ける)

## 完了条件

- `src/color.rs` が新規作成され、 `Color` と上記 API が定義されている
- `src/lib.rs` に `pub mod color;` が追加されている
- 上記「各呼び出し元の書き換え」のとおり、 obsws / webrtc 配下の全呼び出し元 (`text_overlay.rs` の Create / Update / state_to_json / `color_source.rs` / `state/types.rs` / `p2p_session.rs`) が `Color` 経由に切り替わっている
- 旧 `pub fn parse_hex_color` (`webrtc_source.rs`)、 `parse_argb_color` / `argb_to_hex_string` (`text_overlay.rs`) が削除されている
- `validate_hex_color` (`state/types.rs`) は関数として残り、 内部が `Color::from_hex_rgb` 経由に置き換わっている
- `pbt/tests/prop_color.rs` が新規作成され、 上記 PBT プロパティを含む
- `grep -rEn 'parse_hex_color|parse_argb_color|argb_to_hex_string' src/ pbt/ tests/ examples/ fuzz/` で `src/color.rs` および `pbt/tests/prop_color.rs` 以外に出現がない
- 既存の受理範囲・拒否範囲が変わっていない。 `fontColor` のエラー文言は 1 種類 (`fontColor must be #RRGGBB or #RRGGBBAA: ...`) に統合され、 `expected #RRGGBB` (`validate_hex_color` 経路) は維持される
- `cargo test --all-targets` がすべて通る
- `cargo fmt --check` および `cargo clippy --all-targets -- -D warnings` がすべて通る

## 関連

- open `issues/0052-feature-refactor-obsws-parse-helpers.md` (JSON フィールド解析ヘルパー集約) と同じ `text_overlay.rs` を編集するが、 対象範囲が異なる (0052 は JSON ヘルパ群、 本 issue は hex 色関連) のでマージ順序は問わない

## CHANGES.md について

内部リファクタであり外部から観測可能な挙動 (state file フォーマット、 obsws レスポンス、 受理範囲) は変えないため `CHANGES.md` には記載しない (`shiguredo-changelog` 規約準拠)。 `fontColor` のエラー文言は 1 種類に統合するが、 主要キーワード `fontColor must` は維持されておりクライアント側の文言文字列パースは想定外のため非互換扱いしない。

## 解決方法

`src/color.rs` を新規作成し共通 `Color` 型を集約した。

実装の主な内訳:

- 配置: `src/color.rs` を crate root に置き、 `src/lib.rs` に `pub mod color;` を追加した。 PBT (`pbt/tests/prop_color.rs`) が別 crate から `use hisui::color::Color;` で参照する必要があるため `pub` で公開する方針に正規化した
- API: `Color::from_hex` / `from_hex_rgb` / `from_argb_u32` / `to_hex_string` / `to_argb_u32` / `to_rgb` を実装した。 戻り値型は `Option<Self>` で統一し、 失敗種別を呼び出し元が区別しない実態に合わせた (専用エラー enum は導入しない)。 `from_hex_rgb` は 6 桁チェック後に `from_hex` に委譲する形で実装重複を解消した
- 旧関数の置き換え: `parse_argb_color` / `argb_to_hex_string` (`text_overlay.rs`) と `pub fn parse_hex_color` (`webrtc_source.rs`) を削除し、 全呼び出し元 (text_overlay の Create / Update / state_to_json、 `color_source.rs`、 `state/types.rs::validate_hex_color` の内部、 `p2p_session.rs::resolve_chroma_key_config`) を `Color` 経由に書き換えた
- 文言整形: `fontColor` のエラー文言を 1 種類 (`fontColor must be #RRGGBB or #RRGGBBAA: ...`) に統合し、 旧 `parse_argb_color` の 3 種別出し分けは廃止した。 `validate_hex_color` 側は単一文言 (`expected #RRGGBB, got ...`) を維持した
- テスト: `pbt/tests/prop_color.rs` を新規作成し、 ARGB ラウンドトリップ / 6 桁・8 桁の受理 / 6 桁以外と 6/8 桁以外の `None` / `#` 不在の `None` / 出力フォーマット (`#` + 8 桁大文字 hex) を網羅した
- 受理範囲は変えていない (`fontColor` は 6/8 桁、 `color_source` の `color` / `webrtc_source` の `background_key_color` は 6 桁のみ)。 既存 state file フォーマット・obsws レスポンスを維持している

`cargo test --workspace --all-targets` / `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` がすべて通ることを確認した。
