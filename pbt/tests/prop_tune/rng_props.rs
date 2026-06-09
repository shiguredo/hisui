use hisui::tune::rng;
use proptest::prelude::*;

// 乱数ヘルパーが「常に指定範囲内の値を返す」という安全性の不変条件を検証する。
// 一様性 (分布の偏り) までは検証しないが、境界の off-by-one や符号付き範囲の
// 取り違えはここで検出できる。

proptest! {
    // gen_range_i64(min, max) は常に [min, max] (両端含む) に収まる。
    // any::<i64>() を 2 つ生成して min/max に正規化することで、負値・min==max・
    // i64 全域 (min==i64::MIN, max==i64::MAX の count==0 分岐) も踏む。
    #[test]
    fn gen_range_i64_within_bounds(a in any::<i64>(), b in any::<i64>()) {
        let (min, max) = if a <= b { (a, b) } else { (b, a) };
        let v = rng::gen_range_i64(min, max).expect("乱数生成は成功する");
        prop_assert!(min <= v && v <= max, "範囲 [{min}, {max}] に対し {v} が範囲外");
    }

    // gen_unit_f64() は常に [0.0, 1.0) に収まる (1.0 を含まない)。
    // 入力は使わないが、proptest のケース数だけ繰り返し検証させる。
    #[test]
    fn gen_unit_f64_in_unit_interval(_seed in any::<u64>()) {
        let v = rng::gen_unit_f64().expect("乱数生成は成功する");
        prop_assert!((0.0..1.0).contains(&v), "{v} が [0.0, 1.0) の範囲外");
    }

    // gen_index(len) は常に [0, len) を返す。
    #[test]
    fn gen_index_below_len(len in 1usize..10_000) {
        let v = rng::gen_index(len).expect("乱数生成は成功する");
        prop_assert!(v < len, "len={len} に対し index={v} が範囲外");
    }
}
