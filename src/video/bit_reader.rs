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
