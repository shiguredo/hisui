//! hex 色文字列を扱う共通 Color 型。

/// hex 文字列由来の色値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// `Color::from_hex` / `Color::from_hex_rgb` のパース失敗種別。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorParseError {
    /// `#` プレフィックス不在 (空文字もここに分類する)。
    MissingHashPrefix,
    /// `#` を除いた後の文字数が 6 / 8 以外。 引数値は `#` を除いた後の長さ。
    InvalidLength(usize),
    /// hex 以外の文字が含まれる。
    InvalidHex,
}

impl Color {
    /// `#RRGGBB` (a=0xFF として扱う) と `#RRGGBBAA` の両方を受け付ける。
    /// 空文字は `MissingHashPrefix` で拒否する。
    /// 大文字・小文字いずれの hex も受理する。
    pub fn from_hex(s: &str) -> Result<Self, ColorParseError> {
        let stripped = s
            .strip_prefix('#')
            .ok_or(ColorParseError::MissingHashPrefix)?;
        if stripped.len() != 6 && stripped.len() != 8 {
            return Err(ColorParseError::InvalidLength(stripped.len()));
        }
        if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ColorParseError::InvalidHex);
        }
        let r = u8::from_str_radix(&stripped[0..2], 16).expect(
            "unreachable: length and hex digits validated above; implementation bug if reached",
        );
        let g = u8::from_str_radix(&stripped[2..4], 16).expect(
            "unreachable: length and hex digits validated above; implementation bug if reached",
        );
        let b = u8::from_str_radix(&stripped[4..6], 16).expect(
            "unreachable: length and hex digits validated above; implementation bug if reached",
        );
        let a = if stripped.len() == 8 {
            u8::from_str_radix(&stripped[6..8], 16).expect(
                "unreachable: length and hex digits validated above; implementation bug if reached",
            )
        } else {
            0xFF
        };
        Ok(Self { r, g, b, a })
    }

    /// `#RRGGBB` のみ受け付ける。 6 桁以外は `InvalidLength(<#を除いた長さ>)` で拒否する
    /// (8 桁を渡しても `InvalidLength(8)` で拒否される)。
    /// 空文字 / `#` 不在は `MissingHashPrefix` で拒否する (`from_hex` と同じ挙動)。
    /// 成功時は `a = 0xFF` を埋める。
    pub fn from_hex_rgb(s: &str) -> Result<Self, ColorParseError> {
        // 8 桁を弾く受理範囲チェックだけ先に行い、 残りの hex 検証と r/g/b 抽出は
        // 6 桁分岐の挙動が `from_hex` と同一なのでそのまま委譲する (`a = 0xFF` が埋まる)。
        let stripped = s
            .strip_prefix('#')
            .ok_or(ColorParseError::MissingHashPrefix)?;
        if stripped.len() != 6 {
            return Err(ColorParseError::InvalidLength(stripped.len()));
        }
        Self::from_hex(s)
    }

    /// ARGB u32 (`0xAARRGGBB` レイアウト) から `Color` を構築する。 失敗しない変換。
    pub const fn from_argb_u32(argb: u32) -> Self {
        Self {
            a: ((argb >> 24) & 0xFF) as u8,
            r: ((argb >> 16) & 0xFF) as u8,
            g: ((argb >> 8) & 0xFF) as u8,
            b: (argb & 0xFF) as u8,
        }
    }

    /// 常に 8 桁 `#RRGGBBAA` を出力する (alpha=0xFF でも `FF` を付与、 hex は大文字)。
    pub fn to_hex_string(&self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
    }

    /// ARGB u32 (`((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)`) を返す。
    pub const fn to_argb_u32(&self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// `(r, g, b)` 順のタプル。 alpha は捨てる。
    pub const fn rgb_tuple(&self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 大文字・小文字混在の hex 文字列も受理することを明示確認する。
    /// (PBT 側では `[0-9A-Fa-f]` strategy で広く検証するが、 ここでは挙動を
    /// ドキュメント代わりに明示する。)
    #[test]
    fn from_hex_accepts_lowercase() {
        let c = Color::from_hex("#abCD12").expect("大文字小文字混在は受理されるはず");
        assert_eq!(
            c,
            Color {
                r: 0xab,
                g: 0xcd,
                b: 0x12,
                a: 0xFF,
            },
            "大文字小文字混在でも値はそのまま復元される"
        );
    }

    /// 空文字は `MissingHashPrefix` として拒否する。
    #[test]
    fn from_hex_rejects_empty() {
        assert_eq!(
            Color::from_hex(""),
            Err(ColorParseError::MissingHashPrefix),
            "空文字は MissingHashPrefix で拒否されるはず"
        );
    }

    /// hex 以外の文字が含まれる場合は `InvalidHex` を返す。
    #[test]
    fn from_hex_rejects_non_hex_chars() {
        assert_eq!(
            Color::from_hex("#GGGGGG"),
            Err(ColorParseError::InvalidHex),
            "hex 以外の文字は InvalidHex で拒否されるはず"
        );
    }

    /// 6 / 8 桁以外は `InvalidLength(<#を除いた長さ>)` で拒否する。
    /// `InvalidLength` の引数値仕様 (`#` を除いた後の長さ) を境界値で確認する。
    #[test]
    fn from_hex_rejects_wrong_length() {
        assert_eq!(
            Color::from_hex("#FFF"),
            Err(ColorParseError::InvalidLength(3)),
            "3 桁は InvalidLength(3) で拒否されるはず"
        );
        assert_eq!(
            Color::from_hex("#FFFFFFFFF"),
            Err(ColorParseError::InvalidLength(9)),
            "9 桁は InvalidLength(9) で拒否されるはず"
        );
    }

    /// 代表色について `to_hex_string` が `a=0xFF` でも 8 桁を返し、
    /// hex は大文字であることを確認する。
    #[test]
    fn to_hex_string_always_8_digits_uppercase() {
        let cases = [
            (
                Color {
                    r: 0xFF,
                    g: 0x00,
                    b: 0x00,
                    a: 0xFF,
                },
                "#FF0000FF",
            ),
            (
                Color {
                    r: 0x00,
                    g: 0xFF,
                    b: 0x00,
                    a: 0xFF,
                },
                "#00FF00FF",
            ),
            (
                Color {
                    r: 0x00,
                    g: 0x00,
                    b: 0xFF,
                    a: 0xFF,
                },
                "#0000FFFF",
            ),
            (
                Color {
                    r: 0x00,
                    g: 0x00,
                    b: 0x00,
                    a: 0xFF,
                },
                "#000000FF",
            ),
            (
                Color {
                    r: 0xFF,
                    g: 0xFF,
                    b: 0xFF,
                    a: 0xFF,
                },
                "#FFFFFFFF",
            ),
        ];
        for (color, expected) in cases {
            assert_eq!(
                color.to_hex_string(),
                expected,
                "{expected:?} は 8 桁大文字で出力されるはず"
            );
        }
    }
}
