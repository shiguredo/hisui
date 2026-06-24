//! `src/color.rs` の Color 型に対する PBT。
//!
//! u32 → Color → u32 / u32 → hex → Color → u32 のラウンドトリップと、
//! `from_hex` / `from_hex_rgb` の受理範囲の境界を proptest で範囲網羅的に検証する。
//! hex 以外の文字を含むケースは挙動が分岐するため strategy で hex 文字に限定し、
//! 単体テストの責務 (`src/color.rs` の `mod tests`) と切り分けている。

use hisui::color::{Color, ColorParseError};
use proptest::prelude::*;

proptest! {
    /// 任意 u32 で `from_argb_u32` → `to_argb_u32` が元値と一致する (Color 構造体経由の往復一致)。
    #[test]
    fn argb_u32_roundtrips_via_color(argb in any::<u32>()) {
        let restored = Color::from_argb_u32(argb).to_argb_u32();
        prop_assert_eq!(restored, argb, "u32 → Color → u32 のラウンドトリップは恒等であるはず");
    }

    /// 任意 u32 で `from_argb_u32` → `to_hex_string` → `from_hex` → `to_argb_u32` が元値と一致する
    /// (hex 文字列を介した往復一致)。
    #[test]
    fn argb_u32_roundtrips_via_hex_string(argb in any::<u32>()) {
        let s = Color::from_argb_u32(argb).to_hex_string();
        let restored = Color::from_hex(&s)
            .expect("Color 自身が生成した hex は from_hex で必ず成功するはず")
            .to_argb_u32();
        prop_assert_eq!(restored, argb, "u32 → hex → Color → u32 のラウンドトリップは恒等であるはず");
    }

    /// 任意 `#RRGGBB` 文字列 (6 桁 hex) で `from_hex` と `from_hex_rgb` がともに成功し、
    /// 結果の Color が一致する (両者とも `a = 0xFF` を埋める仕様)。
    #[test]
    fn from_hex_and_from_hex_rgb_agree_on_6digit(hex6 in "[0-9A-Fa-f]{6}") {
        let s = format!("#{hex6}");
        let from_hex_result = Color::from_hex(&s)
            .expect("6 桁 hex は from_hex で成功するはず");
        let from_hex_rgb_result = Color::from_hex_rgb(&s)
            .expect("6 桁 hex は from_hex_rgb で成功するはず");
        prop_assert_eq!(
            from_hex_result, from_hex_rgb_result,
            "6 桁入力では from_hex と from_hex_rgb の結果は完全に一致するはず"
        );
        prop_assert_eq!(
            from_hex_result.a, 0xFF,
            "6 桁 from_hex の alpha は 0xFF が埋まるはず"
        );
    }

    /// 任意 `#RRGGBBAA` 文字列 (8 桁 hex) で `from_hex` は成功し、
    /// `from_hex_rgb` は必ず `InvalidLength(8)` で拒否される。
    #[test]
    fn from_hex_accepts_8digit_but_from_hex_rgb_rejects(hex8 in "[0-9A-Fa-f]{8}") {
        let s = format!("#{hex8}");
        prop_assert!(
            Color::from_hex(&s).is_ok(),
            "8 桁 hex は from_hex で成功するはず"
        );
        prop_assert_eq!(
            Color::from_hex_rgb(&s),
            Err(ColorParseError::InvalidLength(8)),
            "8 桁 hex は from_hex_rgb で InvalidLength(8) として拒否されるはず"
        );
    }

    /// `#` を含まない任意 hex 文字列 (0〜16 桁の hex 文字列) で
    /// `from_hex` / `from_hex_rgb` がともに `MissingHashPrefix` を返す。
    /// 空文字も含むため、 空文字の挙動も同時にカバーされる。
    #[test]
    fn both_parsers_reject_missing_hash(no_hash in "[0-9A-Fa-f]{0,16}") {
        prop_assert_eq!(
            Color::from_hex(&no_hash),
            Err(ColorParseError::MissingHashPrefix),
            "# 不在文字列は from_hex で MissingHashPrefix として拒否されるはず"
        );
        prop_assert_eq!(
            Color::from_hex_rgb(&no_hash),
            Err(ColorParseError::MissingHashPrefix),
            "# 不在文字列は from_hex_rgb で MissingHashPrefix として拒否されるはず"
        );
    }
}
