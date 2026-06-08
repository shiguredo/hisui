// NSGA-II 用の乱数ヘルパー
//
// 再現性 (シード指定) は今回スコープ外のため、暗号ライブラリ aws-lc-rs の乱数を直接使う
// (src/srt/inbound_endpoint.rs の pseudo_random_u32 と同じ流儀)。
// シード可能な PRNG が必要になった時点で改めて検討する。

/// 一様乱数として `u64` を 1 つ生成する
fn next_u64() -> crate::Result<u64> {
    let mut bytes = [0u8; 8];
    aws_lc_rs::rand::fill(&mut bytes)
        .map_err(|_| crate::Error::new("failed to generate random bytes with aws-lc-rs"))?;
    Ok(u64::from_le_bytes(bytes))
}

/// `[0.0, 1.0)` の一様乱数を生成する
pub fn gen_unit_f64() -> crate::Result<f64> {
    // 53 bit の仮数部に収まるように上位 53 bit を使って [0, 1) に正規化する
    let v = next_u64()? >> 11;
    Ok(v as f64 / (1u64 << 53) as f64)
}

/// `[min, max]` (両端含む) の一様乱数整数を生成する
///
/// `min` が負の閉区間 (例: `min = -1`) も扱える。
/// 素朴な剰余によるモジュロバイアスを避けるため rejection sampling を用いる。
pub fn gen_range_i64(min: i64, max: i64) -> crate::Result<i64> {
    debug_assert!(min <= max, "min must be less than or equal to max");
    if min == max {
        return Ok(min);
    }

    // 区間幅 (両端含む) を u64 で求める。i64 の差分はオーバーフローしうるので
    // wrapping 差分を取ってから +1 する (max > min が保証されているので 0 にはならない)。
    let span = max.wrapping_sub(min) as u64;
    let count = span.wrapping_add(1);

    if count == 0 {
        // span == u64::MAX のケース (i64 全域)。剰余なしでそのまま使える。
        return Ok(min.wrapping_add(next_u64()? as i64));
    }

    // [0, count) を一様に得るために、count で割り切れる最大の閾値を超えた値を棄却する
    let zone = u64::MAX - (u64::MAX % count);
    loop {
        let v = next_u64()?;
        if v < zone {
            let offset = v % count;
            return Ok(min.wrapping_add(offset as i64));
        }
    }
}

/// `[0, len)` の一様乱数インデックスを生成する
pub fn gen_index(len: usize) -> crate::Result<usize> {
    debug_assert!(len > 0, "len must be greater than zero");
    Ok(gen_range_i64(0, len as i64 - 1)? as usize)
}

/// 確率 `prob` (0.0〜1.0) で `true` を返す
pub fn gen_bool(prob: f64) -> crate::Result<bool> {
    Ok(gen_unit_f64()? < prob)
}
