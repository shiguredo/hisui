//! 0 以上 1 以下の確率値と、対数領域の確率値を表す共通型。

/// `[0.0, 1.0]` の範囲内であることが型で保証された確率値。
///
/// - コンストラクタ `new` は範囲外 (負値、1.0 超、NaN、Inf) のとき `None` を返す
/// - `get` で内部の `f64` を取り出せる (比較演算や外部 API に渡すときに使う)
/// - VAD の閾値・発話確率、YOLO の confidence、CLIP の類似度など、確率的な指標を型で表す
/// - 内部型を f64 にしてある理由は、ML 推論エンジンが accumulator に f64 を採ることが多く、
///   境界で narrowing しないほうが素直に扱えるため。呼び出し側で f32 が要れば `get() as f32` する
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probability(f64);

impl Probability {
    /// `[0.0, 1.0]` の範囲内なら `Some` を返す。NaN・Inf・範囲外はすべて `None`。
    ///
    /// NaN は `>= 0.0` と `<= 1.0` の両方が false になるため、自然に弾かれる。
    pub const fn new(v: f64) -> Option<Self> {
        if v >= 0.0 && v <= 1.0 {
            Some(Self(v))
        } else {
            None
        }
    }

    /// 内部の `f64` を取り出す。
    pub const fn get(self) -> f64 {
        self.0
    }
}

// `[0.0, 1.0]` の有限値のみを許容するため、Probability には全順序が定義できる。
// f64 は NaN のため `Eq` / `Ord` を実装しないが、Probability は NaN を排除しているので impl できる。
// `PartialOrd` は `Ord::cmp` に委譲して両者の整合性 (clippy::derive_ord_xor_partial_ord) を保つ。
impl Eq for Probability {}

impl Ord for Probability {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Probability は有限値のみを許容するため `f64::partial_cmp` は必ず `Some` を返す。
        self.0
            .partial_cmp(&other.0)
            .expect("Probability は有限値のみを許容するので全順序")
    }
}

impl PartialOrd for Probability {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// 対数領域の確率値。`(-∞, 0]` の範囲内であることが型で保証される。
///
/// - `log(p)` where `p ∈ (0, 1]` の値。0 は `log(1)` に、`-∞` は `log(0)` に相当する
/// - コンストラクタ `new` は正の値や NaN のとき `None` を返す。`-∞` は許容する
///   (softmax や積の対数で数値的アンダーフローが起きうるため)
/// - `get` で内部の `f64` を取り出せる (比較演算や外部 API に渡すときに使う)
/// - Whisper の `avg_logprob` 等、対数領域で確率を扱うときに使う
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogProbability(f64);

impl LogProbability {
    /// `(-∞, 0]` の範囲内なら `Some` を返す。NaN と正の値は `None`。`-∞` は許容する。
    ///
    /// NaN は `<= 0.0` が false になるため、自然に弾かれる。
    pub const fn new(v: f64) -> Option<Self> {
        if v <= 0.0 { Some(Self(v)) } else { None }
    }

    /// 内部の `f64` を取り出す。
    pub const fn get(self) -> f64 {
        self.0
    }
}

// `(-∞, 0]` (NaN 排除、`-∞` は含む) に絞っているため、LogProbability には全順序が定義できる。
// `-∞` は f64 の `partial_cmp` で自身とも他の有限値とも比較可能。
impl Eq for LogProbability {}

impl Ord for LogProbability {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // NaN を排除しているため `f64::partial_cmp` は必ず `Some` を返す (`-∞` も含む)。
        self.0
            .partial_cmp(&other.0)
            .expect("LogProbability は NaN を排除しているので全順序")
    }
}

impl PartialOrd for LogProbability {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
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
        assert_eq!(Probability::new(f64::NAN), None);
        assert_eq!(Probability::new(f64::INFINITY), None);
        assert_eq!(Probability::new(f64::NEG_INFINITY), None);
    }

    /// PartialOrd は内部 f64 の比較と一致する。
    #[test]
    fn partial_ord_matches_inner_f64() {
        let low = Probability::new(0.3).expect("有効");
        let mid = Probability::new(0.5).expect("有効");
        let high = Probability::new(0.7).expect("有効");
        assert!(low < mid);
        assert!(mid < high);
        assert!(low < high);
    }

    /// Ord::cmp は Less / Equal / Greater を PartialOrd と一貫して返す (全順序)。
    #[test]
    fn ord_returns_expected_ordering() {
        let low = Probability::new(0.3).expect("有効");
        let mid = Probability::new(0.5).expect("有効");
        let high = Probability::new(0.7).expect("有効");
        assert_eq!(low.cmp(&mid), std::cmp::Ordering::Less);
        assert_eq!(mid.cmp(&mid), std::cmp::Ordering::Equal);
        assert_eq!(high.cmp(&mid), std::cmp::Ordering::Greater);
    }

    /// Ord を実装したので配列 (や Vec) が sort できる (実用例の代表)。
    #[test]
    fn probability_slice_can_be_sorted() {
        let mut probs = [
            Probability::new(0.8).expect("有効"),
            Probability::new(0.3).expect("有効"),
            Probability::new(0.5).expect("有効"),
        ];
        probs.sort();
        assert_eq!(probs[0].get(), 0.3);
        assert_eq!(probs[1].get(), 0.5);
        assert_eq!(probs[2].get(), 0.8);
    }

    /// LogProbability: 有効範囲内の代表値 (負値、0、-∞) で Some を返し、get で同じ値が取り出せる。
    #[test]
    fn log_probability_new_accepts_values_in_range() {
        for &v in &[f64::NEG_INFINITY, -100.0, -1.0, -0.001, 0.0] {
            let p = LogProbability::new(v).expect("有効範囲内は Some のはず");
            assert_eq!(p.get(), v, "get は new に渡した値と一致するはず");
        }
    }

    /// LogProbability: 正の値・NaN・+∞ はすべて None。
    #[test]
    fn log_probability_new_rejects_out_of_range_values() {
        assert_eq!(LogProbability::new(0.0001), None);
        assert_eq!(LogProbability::new(1.0), None);
        assert_eq!(LogProbability::new(f64::NAN), None);
        assert_eq!(LogProbability::new(f64::INFINITY), None);
    }

    /// LogProbability の PartialOrd は内部 f64 の比較と一致する (小さいほど確率が低い)。
    #[test]
    fn log_probability_partial_ord_matches_inner_f64() {
        let low = LogProbability::new(-3.0).expect("有効");
        let mid = LogProbability::new(-1.0).expect("有効");
        let high = LogProbability::new(-0.1).expect("有効");
        assert!(low < mid);
        assert!(mid < high);
        assert!(low < high);
    }

    /// LogProbability: `-∞` は任意の有限値より小さく、`-∞` 自身との比較は Equal を返す (全順序の境界)。
    #[test]
    fn log_probability_neg_infinity_orders_below_finite_values() {
        let neg_inf = LogProbability::new(f64::NEG_INFINITY).expect("有効");
        let finite = LogProbability::new(-10.0).expect("有効");
        assert!(neg_inf < finite);
        assert_eq!(neg_inf.cmp(&neg_inf), std::cmp::Ordering::Equal);
    }
}
