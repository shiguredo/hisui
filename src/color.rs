//! hex 色文字列を扱う共通 Color 型。

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
    pub fn from_hex(s: &str) -> Option<Self> {
        let stripped = s.strip_prefix('#')?;
        if stripped.len() != 6 && stripped.len() != 8 {
            return None;
        }
        if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
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
        Some(Self { r, g, b, a })
    }

    /// `#RRGGBB` のみ受け付ける。 6 桁以外 / 空文字 / `#` 不在 / hex 以外の文字を
    /// 含む場合はすべて `None` を返す。 成功時は `a = 0xFF` を埋める。
    pub fn from_hex_rgb(s: &str) -> Option<Self> {
        // 6 桁分岐の挙動は `from_hex` と同一なので、 8 桁を弾くチェックだけ先に行って委譲する。
        let stripped = s.strip_prefix('#')?;
        if stripped.len() != 6 {
            return None;
        }
        Self::from_hex(s)
    }

    /// ARGB u32 (`0xAARRGGBB` レイアウト) から `Color` を構築する。 失敗しない変換。
    pub fn from_argb_u32(argb: u32) -> Self {
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
    pub fn to_argb_u32(&self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// `(r, g, b)` 順のタプル。 alpha は捨てる。
    pub fn to_rgb(&self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// hex 以外の文字が含まれる場合は `None` を返す。
    #[test]
    fn from_hex_rejects_non_hex_chars() {
        assert_eq!(
            Color::from_hex("#GGGGGG"),
            None,
            "hex 以外の文字を含む入力は None を返すはず"
        );
    }

    /// 6 / 8 桁以外は `None` を返す (境界値: 3 桁と 9 桁)。
    #[test]
    fn from_hex_rejects_wrong_length() {
        assert_eq!(Color::from_hex("#FFF"), None, "3 桁入力は None を返すはず");
        assert_eq!(
            Color::from_hex("#FFFFFFFFF"),
            None,
            "9 桁入力は None を返すはず"
        );
    }
}
