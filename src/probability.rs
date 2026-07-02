//! 0 以上 1 以下の確率値を表す共通型。

/// `[0.0, 1.0]` の範囲内であることが型で保証された確率値。
///
/// - コンストラクタ `new` は範囲外 (負値、1.0 超、NaN、Inf) のとき `None` を返す
/// - `get` で内部の `f32` を取り出せる (比較演算や外部 API に渡すときに使う)
/// - VAD の閾値・発話確率、YOLO の confidence、CLIP の類似度など、確率的な指標を型で表す
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Probability(f32);

impl Probability {
    /// `[0.0, 1.0]` の範囲内なら `Some` を返す。NaN・Inf・範囲外はすべて `None`。
    ///
    /// NaN は `>= 0.0` と `<= 1.0` の両方が false になるため、自然に弾かれる。
    pub const fn new(v: f32) -> Option<Self> {
        if v >= 0.0 && v <= 1.0 {
            Some(Self(v))
        } else {
            None
        }
    }

    /// 内部の `f32` を取り出す。
    pub const fn get(self) -> f32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 有効範囲内の代表値でコンストラクタが Some を返し、get で同じ値が取り出せる。
    #[test]
    fn new_accepts_values_in_range() {
        for &v in &[0.0, 0.25, 0.5, 0.999, 1.0] {
            let p = Probability::new(v).expect("有効範囲内は Some のはず");
            assert_eq!(p.get(), v, "get は new に渡した値と一致するはず");
        }
    }

    /// 範囲外・NaN・Inf はすべて None。
    #[test]
    fn new_rejects_out_of_range_values() {
        assert_eq!(Probability::new(-0.0001), None);
        assert_eq!(Probability::new(1.0001), None);
        assert_eq!(Probability::new(f32::NAN), None);
        assert_eq!(Probability::new(f32::INFINITY), None);
        assert_eq!(Probability::new(f32::NEG_INFINITY), None);
    }

    /// PartialOrd は内部 f32 の比較と一致する。
    #[test]
    fn partial_ord_matches_inner_f32() {
        let low = Probability::new(0.3).expect("有効");
        let mid = Probability::new(0.5).expect("有効");
        let high = Probability::new(0.7).expect("有効");
        assert!(low < mid);
        assert!(mid < high);
        assert!(low < high);
    }
}
