//! `src/color.rs` の Color 型に対する PBT。
//!
//! u32 → Color → u32 / u32 → hex → Color → u32 のラウンドトリップと、
//! `from_hex` / `from_hex_rgb` の受理範囲の境界を proptest で範囲網羅的に検証する。

use hisui::color::{Color, ColorParseError};
use proptest::prelude::*;

proptest! {
    #[test]
    fn argb_u32_roundtrips_via_color(argb in any::<u32>()) {
        let restored = Color::from_argb_u32(argb).to_argb_u32();
        prop_assert_eq!(restored, argb, "u32 → Color → u32 のラウンドトリップは恒等であるはず");
    }

    #[test]
    fn argb_u32_roundtrips_via_hex_string(argb in any::<u32>()) {
        let s = Color::from_argb_u32(argb).to_hex_string();
        let restored = Color::from_hex(&s)
            .expect("Color 自身が生成した hex は from_hex で必ず成功するはず")
            .to_argb_u32();
        prop_assert_eq!(restored, argb, "u32 → hex → Color → u32 のラウンドトリップは恒等であるはず");
    }

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

    #[test]
    fn from_hex_accepts_8digit_but_from_hex_rgb_rejects(hex8 in "[0-9A-Fa-f]{8}") {
        let s = format!("#{hex8}");
        prop_assert!(
            Color::from_hex(&s).is_ok(),
            "8 桁 hex は from_hex で成功するはず"
        );
        prop_assert_eq!(
            Color::from_hex_rgb(&s),
            Err(ColorParseError::InvalidLength { actual: 8 }),
            "8 桁 hex は from_hex_rgb で actual=8 として拒否されるはず"
        );
    }

    // 上限 16 桁は 8 桁 (`from_hex` の受理上限) の倍までを「`#` 不在」として網羅する意図。
    // 下限 0 桁で空文字の挙動も同時にカバーされる。
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
