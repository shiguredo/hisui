//! `src/mixer/video/text_overlay/` のバリデーション関数と `TextOverlaySpec` 構造体に対する PBT。
//!
//! text 長境界 / 行数境界 / fontSize 境界 / 改行のみ / Unicode 制御文字 /
//! フォント不在文字 / 空文字 / x = i64 境界の各境界値ケースを proptest で
//! 範囲網羅的に検証する。
//!
//! 純粋関数 (`validate_text` / `validate_font_size`) のみを対象にし、 ファイル I/O
//! を伴う `validate_font_name_and_resolve_path` や `TextOverlayLayer` の状態遷移は
//! スコープ外。 これらは `src/mixer/video/text_overlay/layer.rs` の単体テストと
//! `src/obsws/session/tests.rs` の obsws 経由テストでカバーされている。

use hisui::mixer::video::text_overlay::validate::{validate_font_size, validate_text};
use hisui::mixer::video::text_overlay::{
    TEXT_MAX_BYTES, TEXT_MAX_LINES, TextOverlayError, TextOverlaySpec,
};
use proptest::prelude::*;

/// テストで使う仮想 canvas の高さ。`validate_font_size` の上限境界として使う。
const TEST_CANVAS_HEIGHT: usize = 1080;

proptest! {
    /// `text` のバイト数が上限以下なら必ず `Ok`。
    /// ASCII 1 バイト文字で組み立てるので、バイト数 = 文字数 = 行数 1 となり、
    /// 行数制限には抵触しない。
    #[test]
    fn validate_text_accepts_bytes_within_limit(len in 0usize..=TEXT_MAX_BYTES) {
        let text = "a".repeat(len);
        prop_assert!(
            validate_text(&text).is_ok(),
            "len={len} はバイト数上限以下のため Ok のはず"
        );
    }

    /// `text` のバイト数が上限を超えると必ず `InvalidText`。
    /// 上限 +1 から上限 *2 までを対象にする (これ以上大きくしてもケースは同じ)。
    #[test]
    fn validate_text_rejects_bytes_exceeding_limit(
        len in (TEXT_MAX_BYTES + 1)..=(TEXT_MAX_BYTES * 2)
    ) {
        let text = "a".repeat(len);
        prop_assert!(matches!(
            validate_text(&text),
            Err(TextOverlayError::InvalidText(_))
        ));
    }

    /// 行数が上限以下なら必ず `Ok`。
    /// 1 行あたり "x" を 1 文字置いて改行で結合するので、行数だけ変動させる。
    #[test]
    fn validate_text_accepts_lines_within_limit(lines in 1usize..=TEXT_MAX_LINES) {
        let text = vec!["x"; lines].join("\n");
        prop_assert!(
            validate_text(&text).is_ok(),
            "lines={lines} は行数上限以下のため Ok のはず"
        );
    }

    /// 行数上限を超えると必ず `InvalidText`。
    #[test]
    fn validate_text_rejects_lines_exceeding_limit(
        lines in (TEXT_MAX_LINES + 1)..=(TEXT_MAX_LINES * 2)
    ) {
        let text = vec!["x"; lines].join("\n");
        prop_assert!(matches!(
            validate_text(&text),
            Err(TextOverlayError::InvalidText(_))
        ));
    }

    /// 改行のみで構成された text も、上限内なら `Ok`。
    /// "\n" を n 個並べると行数 = n + 1 になるので、n は TEXT_MAX_LINES 未満に制限する。
    #[test]
    fn validate_text_accepts_only_newlines_within_limit(
        count in 0usize..TEXT_MAX_LINES
    ) {
        let text = "\n".repeat(count);
        prop_assert!(
            validate_text(&text).is_ok(),
            "count={count} の改行のみ text は Ok のはず"
        );
    }

    /// Unicode 制御文字 (U+0001..=U+001F、ただし U+000A 改行を除く) を含む短い text は Ok。
    /// raden の glyph フォールバックや silent skip 挙動を validate 層で弾かない仕様の確認。
    #[test]
    fn validate_text_accepts_control_chars(
        text in "[\\x01-\\x09\\x0B-\\x1F]{0,100}"
    ) {
        prop_assert!(
            validate_text(&text).is_ok(),
            "制御文字を含む短い text は validate を通る"
        );
    }

    /// フォント (PublicSans-Regular) に含まれない CJK 文字を含む text も validate は通る。
    /// 実際の描画では raden が glyph_id=0 で silent skip するが、validate 層はバイト数 / 行数のみ見る。
    #[test]
    fn validate_text_accepts_cjk_chars(text in "[\\u{3040}-\\u{309F}]{0,100}") {
        // ひらがな 1 文字は UTF-8 で 3 バイト、最大 300 バイトなので TEXT_MAX_BYTES (4096) 以下。
        prop_assert!(
            validate_text(&text).is_ok(),
            "CJK 文字を含む短い text は validate を通る"
        );
    }

    /// `fontSize` が `1..=canvas_height` の範囲内なら必ず `Ok`。
    /// 下限 1 と上限 canvas_height の境界も含む。
    #[test]
    fn validate_font_size_accepts_in_range(
        size in 1u32..=(TEST_CANVAS_HEIGHT as u32)
    ) {
        prop_assert!(
            validate_font_size(size, TEST_CANVAS_HEIGHT).is_ok(),
            "size={size} は範囲内のため Ok のはず"
        );
    }

    /// `fontSize` が canvas_height を超えると `InvalidFontSize`。
    #[test]
    fn validate_font_size_rejects_over_limit(
        size in ((TEST_CANVAS_HEIGHT as u32) + 1)..=u32::MAX
    ) {
        prop_assert!(matches!(
            validate_font_size(size, TEST_CANVAS_HEIGHT),
            Err(TextOverlayError::InvalidFontSize(_))
        ));
    }

    /// `TextOverlaySpec` が任意の i64 値 (= MIN〜MAX 全域) の x / y で構築でき、
    /// panic も overflow も起こさない。
    /// validate 関数を経由しない経路の確認 (issue で x/y は raden 側でクリップする仕様)。
    #[test]
    fn text_overlay_spec_constructs_for_any_i64_xy(x in any::<i64>(), y in any::<i64>()) {
        let spec = TextOverlaySpec {
            text: "x".to_owned(),
            x,
            y,
            font_size: 32,
            font_color_argb: 0xFFFFFFFF,
            font_name: "PublicSans-Regular.ttf".to_owned(),
            z: 0,
        };
        prop_assert_eq!(spec.x, x);
        prop_assert_eq!(spec.y, y);
    }
}

/// 空文字テキストは `Ok`。proptest を経由しない単体ケースとして固定検証する。
#[test]
fn validate_text_accepts_empty_string() {
    assert!(validate_text("").is_ok(), "空文字 text は Ok");
}

/// `fontSize = 0` は `InvalidFontSize` で拒否される。proptest で扱わない単一値の確認。
#[test]
fn validate_font_size_rejects_zero() {
    assert!(matches!(
        validate_font_size(0, TEST_CANVAS_HEIGHT),
        Err(TextOverlayError::InvalidFontSize(_))
    ));
}
