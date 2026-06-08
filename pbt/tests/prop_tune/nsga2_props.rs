use hisui::tune::nsga2::{crowding_distance, dominates, non_dominated_sort};
use proptest::prelude::*;

// NSGA-II のコア純粋関数 (非劣ソート・混雑度距離) の不変条件を検証する。
//
// 同値・タイ・境界 (目的値が等しいケース) でドミナンス判定の `<` / `<=` 取り違えが
// 起きやすいため、これらを乱数生成された点集合で重点的に検証する。

// 2 目的の点集合を生成する戦略
//
// 値の範囲を狭めにとることで、同値・タイのケースが頻繁に現れるようにしている
// (ドミナンス判定の境界バグをあぶり出すため)。
fn points_strategy() -> impl Strategy<Value = Vec<[f64; 2]>> {
    let coord = -5.0f64..5.0f64;
    prop::collection::vec((coord.clone(), coord).prop_map(|(a, b)| [a, b]), 1..40)
}

proptest! {
    // rank 0 (パレートフロント) 上の任意の 2 点は互いに非劣である
    #[test]
    fn pareto_front_mutually_nondominated(points in points_strategy()) {
        let ranks = non_dominated_sort(&points);
        let front0: Vec<usize> = ranks
            .iter()
            .enumerate()
            .filter(|(_, r)| **r == 0)
            .map(|(i, _)| i)
            .collect();

        // フロント上の任意の 2 点 a, b について、どちらも他方を支配しない
        for &a in &front0 {
            for &b in &front0 {
                if a == b {
                    continue;
                }
                prop_assert!(!dominates(&points[a], &points[b]));
            }
        }
    }

    // rank が 1 以上の点は、必ず 1 つ下の rank の点に支配されている
    #[test]
    fn dominated_by_better_front(points in points_strategy()) {
        let ranks = non_dominated_sort(&points);
        for i in 0..points.len() {
            if ranks[i] == 0 {
                continue;
            }
            // rank[i] - 1 の点のうち少なくとも 1 つが i を支配する
            let dominated_by_prev_front = (0..points.len()).any(|j| {
                ranks[j] + 1 == ranks[i] && dominates(&points[j], &points[i])
            });
            prop_assert!(dominated_by_prev_front);
        }
    }

    // rank が小さい点は、より大きい rank の点に支配されない
    #[test]
    fn smaller_rank_not_dominated_by_larger(points in points_strategy()) {
        let ranks = non_dominated_sort(&points);
        for i in 0..points.len() {
            for j in 0..points.len() {
                if ranks[i] < ranks[j] {
                    prop_assert!(!dominates(&points[j], &points[i]));
                }
            }
        }
    }

    // 混雑度距離は入力と同じ長さを返し、端点は無限大・それ以外は非負の有限値になる
    #[test]
    fn crowding_distance_basic_properties(points in points_strategy()) {
        let dist = crowding_distance(&points);
        prop_assert_eq!(dist.len(), points.len());

        let n = points.len();
        if n >= 3 {
            // 各目的軸の端点が無限大になるため、無限大は最低 2 個存在する
            let inf_count = dist.iter().filter(|d| d.is_infinite()).count();
            prop_assert!(inf_count >= 2);
        }

        // 無限大でない距離は非負かつ有限
        for d in &dist {
            if !d.is_infinite() {
                prop_assert!(d.is_finite());
                prop_assert!(*d >= 0.0);
            }
        }
    }
}

// `dominates` の定義そのものの健全性 (反射律・非対称性) を確認する単体的なプロパティ
proptest! {
    // 同一点は自分自身を支配しない (狭義のパレート支配)
    #[test]
    fn point_does_not_dominate_itself(a in -5.0f64..5.0, b in -5.0f64..5.0) {
        let p = [a, b];
        prop_assert!(!dominates(&p, &p));
    }

    // a が b を支配するなら、b は a を支配しない (非対称性)
    #[test]
    fn domination_is_asymmetric(
        a0 in -5.0f64..5.0, a1 in -5.0f64..5.0,
        b0 in -5.0f64..5.0, b1 in -5.0f64..5.0,
    ) {
        let a = [a0, a1];
        let b = [b0, b1];
        if dominates(&a, &b) {
            prop_assert!(!dominates(&b, &a));
        }
    }
}
