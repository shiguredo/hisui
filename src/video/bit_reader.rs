//! 汎用ビットリーダー
//!
//! H.264 / H.265 の SPS パーサで共有するビット単位読み出しユーティリティ。
//! バイト列を 1 ビット単位で読み出し、ITU-T 仕様の Exp-Golomb 復号 (ue(v) / se(v)) と
//! 固定長ビットフィールド (u(n)) の読み出しを提供する。

/// バイト列を 1 ビット単位で読み出すリーダー
///
/// 全 read メソッド（`read_u` / `read_ue` / `read_se`）はバッファ末尾を超える読み出しで Err を返す。
/// パニックや無限ループは起こらないため、proptest のクラッシュフリー保証はこの構造で担保される。
pub struct BitReader<'a> {
    data: &'a [u8],
    // バイト単位の現在位置
    byte_pos: usize,
    // 現バイト内のビット位置（0 = MSB, 7 = LSB）
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// n ビット符号なし整数を読み出す（仕様の u(n) に相当）
    ///
    /// n は最大 32 まで対応する。バッファ末尾を超える場合は Err を返す。
    pub fn read_u(&mut self, n: usize) -> crate::Result<u32> {
        if n > 32 {
            return Err(crate::Error::new(format!(
                "bit reader: read_u with n > 32 (n={n})"
            )));
        }
        let mut value: u32 = 0;
        for _ in 0..n {
            value = (value << 1) | self.read_bit()? as u32;
        }
        Ok(value)
    }

    /// 1 ビットを読み出す（内部ヘルパー）
    fn read_bit(&mut self) -> crate::Result<u8> {
        if self.byte_pos >= self.data.len() {
            return Err(crate::Error::new(
                "bit reader: exhausted before requested read",
            ));
        }
        let bit = (self.data[self.byte_pos] >> (7 - self.bit_pos)) & 1;
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
        Ok(bit)
    }

    /// 符号なし Exp-Golomb 復号（仕様 9.1 の ue(v) に相当）
    ///
    /// 連続する 0 ビットの数を leading_zeros として数え、続く 1 ビットを読み、
    /// その後 leading_zeros 個のビットを読んで `(1 << leading_zeros) - 1 + bits` を返す。
    pub fn read_ue(&mut self) -> crate::Result<u32> {
        let mut leading_zeros: u32 = 0;
        loop {
            let bit = self.read_bit()?;
            if bit == 1 {
                break;
            }
            leading_zeros += 1;
            // 仕様 9.1 上 codeNum は最大 2^32 - 2 まで表現可能だが、`1u32 << 32` がシフト範囲外で
            // panic / 未定義動作になるため 31 で制限する。Hisui で扱う SPS フィールド値はすべて 31 bit 以下。
            if leading_zeros > 31 {
                return Err(crate::Error::new(
                    "bit reader: ue(v) leading_zeros exceeds 31 (overflow)",
                ));
            }
        }
        if leading_zeros == 0 {
            return Ok(0);
        }
        let suffix = self.read_u(leading_zeros as usize)?;
        // (1 << leading_zeros) - 1 + suffix が u32 にちょうど収まる範囲
        // leading_zeros == 31 のとき、(1 << 31) - 1 + suffix は最大 (2^31 - 1) + (2^31 - 1) = 2^32 - 2
        let prefix = (1u32 << leading_zeros).wrapping_sub(1);
        prefix
            .checked_add(suffix)
            .ok_or_else(|| crate::Error::new("bit reader: ue(v) value overflow on combine"))
    }

    /// 符号付き Exp-Golomb 復号（仕様 9.1.1 の se(v) に相当）
    ///
    /// 内部で ue(v) を読み、code_num が偶数なら -code_num / 2、奇数なら (code_num + 1) / 2 を返す。
    pub fn read_se(&mut self) -> crate::Result<i32> {
        let code_num = self.read_ue()?;
        if code_num % 2 == 1 {
            let value = code_num.div_ceil(2);
            i32::try_from(value)
                .map_err(|_| crate::Error::new("bit reader: se(v) positive value overflow"))
        } else {
            let value = code_num / 2;
            let negated = i64::from(value)
                .checked_neg()
                .ok_or_else(|| crate::Error::new("bit reader: se(v) negation overflow"))?;
            i32::try_from(negated)
                .map_err(|_| crate::Error::new("bit reader: se(v) negative value overflow"))
        }
    }

    /// n ビット符号なし整数を読み飛ばす（戻り値を捨てる `read_u` のラッパー）
    pub fn skip_u(&mut self, n: usize) -> crate::Result<()> {
        self.read_u(n).map(|_| ())
    }

    /// ue(v) を読み飛ばす（戻り値を捨てる `read_ue` のラッパー）
    pub fn skip_ue(&mut self) -> crate::Result<()> {
        self.read_ue().map(|_| ())
    }

    /// se(v) を読み飛ばす（戻り値を捨てる `read_se` のラッパー）
    pub fn skip_se(&mut self) -> crate::Result<()> {
        self.read_se().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_ue_decodes_specification_examples() {
        // ITU-T H.264 仕様 9.1 表 9-1 の代表的な値を網羅的に検証する
        // codeNum = 0 → "1"
        // codeNum = 1 → "010"
        // codeNum = 2 → "011"
        // codeNum = 3 → "00100"
        // codeNum = 4 → "00101"
        // codeNum = 5 → "00110"
        // codeNum = 6 → "00111"
        let data = [
            // "1 010 011 00100 00101 00110 00111" を 8 bit 単位に詰める
            // MSB から bit 0..27 を順に並べると次の 4 バイトになる:
            //   1010 0110 = 0xa6
            //   0100 0010 = 0x42
            //   1001 1000 = 0x98
            //   1110 0000 = 0xe0（最後の 3 bit は "111"、残り 5 bit は 0 padding）
            0xa6, 0x42, 0x98, 0xe0,
        ];
        let mut reader = BitReader::new(&data);
        let expected = [0u32, 1, 2, 3, 4, 5, 6];
        for &want in &expected {
            let got = reader.read_ue().expect("ue(v) 読み出し成功");
            assert_eq!(got, want, "ue(v) のデコード結果が期待値と一致すること");
        }
    }

    #[test]
    fn read_se_decodes_specification_examples() {
        // ITU-T H.264 仕様 9.1.1 表 9-3 の代表的な値:
        // ue codeNum 0 → se 0
        // ue codeNum 1 → se 1
        // ue codeNum 2 → se -1
        // ue codeNum 3 → se 2
        // ue codeNum 4 → se -2
        // ue を順に並べたバイト列を使う
        // ue: 1, 010, 011, 00100, 00101 = "1 010 011 00100 00101" を 8 bit 単位
        // 1 0 1 0 0 1 1 0 = 0xa6
        // 0 1 0 0 0 0 1 0 = 0x42
        // 1 ... 0 埋め → 0x80
        let data = [0xa6, 0x42, 0x80];
        let mut reader = BitReader::new(&data);
        let expected = [0i32, 1, -1, 2, -2];
        for &want in &expected {
            let got = reader.read_se().expect("se(v) 読み出し成功");
            assert_eq!(got, want, "se(v) のデコード結果が期待値と一致すること");
        }
    }

    #[test]
    fn read_u_fails_on_exhausted_buffer() {
        // バッファ末尾を超えた読み出しで Err を返すこと
        let data = [0xff];
        let mut reader = BitReader::new(&data);
        // 8 bit 読めるが、9 bit 目で Err
        assert!(reader.read_u(8).is_ok());
        assert!(
            reader.read_u(1).is_err(),
            "exhausted buffer で Err を返すはず"
        );
    }

    #[test]
    fn read_u_rejects_too_large_n() {
        // read_u(n) で n > 32 は Err を返すこと
        let data = [0xff; 8];
        let mut reader = BitReader::new(&data);
        assert!(reader.read_u(33).is_err(), "n > 32 は Err を返すはず");
    }

    #[test]
    fn read_ue_rejects_excessive_leading_zeros() {
        // 0 ビットを 32 個以上連続させると `1u32 << 32` がシフト範囲外になるため Err を返すこと
        // 32 個の連続 0 = 4 バイトすべて 0x00 にして、その後を埋める
        let data = [0x00, 0x00, 0x00, 0x00, 0xff, 0xff];
        let mut reader = BitReader::new(&data);
        let result = reader.read_ue();
        assert!(
            result.is_err(),
            "leading_zeros が 31 を超えると Err を返すはず: {result:?}"
        );
    }
}
