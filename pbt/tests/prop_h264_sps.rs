//! `h264_sample_entry_from_sps_pps_lists` 経路に対する構造化 PBT
//!
//! Ok / Err 経路の責務分担は各 `mod` のヘッダコメントを参照。
//! クラッシュフリー検証は `fuzz/fuzz_targets/fuzz_h264_sample_entry.rs` が担う。

use hisui::video::h264::{
    H264_HIGH_PROFILES, SpsBuildParams, build_sps_for_pbt, h264_sample_entry_from_sps_pps_lists,
};
use proptest::prelude::*;
use shiguredo_mp4::boxes::SampleEntry;

/// 最小 PPS NAL (固定、本体 `src/video/h264.rs::tests::PPS_NAL` と同一バイト列)。
///
/// `h264_sample_entry_from_sps_pps_lists` は `pps_list[i]` 先頭バイトの `& 0x1F == 8` のみ
/// 検査するため、本 PBT の解像度 / avcC 検証には PPS payload の中身は影響しない。
const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2];

// ----------------------------------------------------------------
// Strategy ヘルパー
// ----------------------------------------------------------------

/// 仕様準拠プロファイル群 (`{66, 77, 88}` ∪ `H264_HIGH_PROFILES`) から profile_idc を選ぶ Strategy
fn supported_profile_idc() -> impl Strategy<Value = u8> {
    prop_oneof![
        Just(66u8),
        Just(77u8),
        Just(88u8),
        prop::sample::select(&H264_HIGH_PROFILES[..]),
    ]
}

/// マクロブロック境界の raw 幅 (16 倍数、u16::MAX 内)
fn raw_width_strategy() -> impl Strategy<Value = u32> {
    (1u32..=4095).prop_map(|n| n * 16)
}

/// マクロブロック境界の raw 高 (16 倍数)。
///
/// `SpsBuildParams.raw_height` は SPS の `pic_height_in_map_units_minus1` から計算される field 単位の
/// 値で、`parse_sps::read_dimensions_with_cropping` 内で `raw_height * (2 - frame_mbs_only_flag)` が
/// frame 単位高さとして u16 上限検査される。interlaced (`frame_mbs_only_flag=false`) では 2 倍されるため、
/// frame_mbs_only_flag に応じて raw_height の上限を変える (戻り値は `BoxedStrategy<u32>` で揃える)。
fn raw_height_strategy(frame_mbs_only_flag: bool) -> BoxedStrategy<u32> {
    if frame_mbs_only_flag {
        (1u32..=4095).prop_map(|n| n * 16).boxed()
    } else {
        // interlaced のとき frame 単位高さは raw_height * 2 になるため、上限を半分にする。
        (1u32..=2047).prop_map(|n| n * 16).boxed()
    }
}

/// High 系プロファイル固有フィールド群 (Strategy 値)
#[derive(Debug, Clone, Copy, Default)]
struct HighProfileFields {
    chroma_format_idc: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
}

/// 仕様 Ok 値域内の High 系プロファイル固有フィールドを生成する
fn high_profile_fields_strategy() -> impl Strategy<Value = HighProfileFields> {
    (0u8..=3, 0u8..=6, 0u8..=6).prop_map(
        |(chroma_format_idc, bit_depth_luma_minus8, bit_depth_chroma_minus8)| HighProfileFields {
            chroma_format_idc,
            bit_depth_luma_minus8,
            bit_depth_chroma_minus8,
        },
    )
}

/// Ok 経路用の `SpsBuildParams` Strategy
///
/// profile_idc が High 系のときだけ High 系固有フィールドを生成する (`prop_flat_map` で分岐)。
/// Baseline / Main / Extended では `HighProfileFields::default()` を使うが、`build_sps_for_pbt`
/// は非 High 系では SPS バイト列に書き込まないため値の中身は影響しない。
/// cropping は生成しない (cropping 反映の検証は別 Strategy で扱う)。
fn ok_sps_strategy() -> impl Strategy<Value = SpsBuildParams> {
    (
        supported_profile_idc(),
        any::<u8>(),
        any::<u8>(),
        raw_width_strategy(),
        any::<bool>(),
        prop::sample::select(vec![0u32, 1, 2]),
    )
        .prop_flat_map(
            |(
                profile_idc,
                constraint_set_flags,
                level_idc,
                raw_width,
                frame_mbs_only_flag,
                pic_order_cnt_type,
            )| {
                // raw_height は frame_mbs_only_flag に応じて u16::MAX 内で生成する。
                let height_strategy = raw_height_strategy(frame_mbs_only_flag);
                let high_strategy = if H264_HIGH_PROFILES.contains(&profile_idc) {
                    high_profile_fields_strategy().boxed()
                } else {
                    Just(HighProfileFields::default()).boxed()
                };
                (height_strategy, high_strategy).prop_map(move |(raw_height, high)| {
                    SpsBuildParams {
                        profile_idc,
                        constraint_set_flags,
                        level_idc,
                        chroma_format_idc: high.chroma_format_idc,
                        bit_depth_luma_minus8: high.bit_depth_luma_minus8,
                        bit_depth_chroma_minus8: high.bit_depth_chroma_minus8,
                        raw_width,
                        raw_height,
                        frame_mbs_only_flag,
                        seq_scaling_matrix_present_flag: false,
                        pic_order_cnt_type,
                        frame_cropping: None,
                    }
                })
            },
        )
}

/// cropping 反映検証用の Ok 経路 Strategy
///
/// raw_width / raw_height を小さめに固定し、crop_offsets で実用範囲のクロップを掛ける。
/// アンダーフローや u16::MAX 超過に至らないよう値域を抑える。
fn ok_sps_with_cropping_strategy() -> impl Strategy<Value = SpsBuildParams> {
    (
        supported_profile_idc(),
        prop::sample::select(vec![320u32, 640, 1280, 1920]),
        prop::sample::select(vec![240u32, 480, 720, 1088]),
        0u32..=3,
        0u32..=3,
        0u32..=3,
        0u32..=3,
    )
        .prop_flat_map(|(profile_idc, raw_width, raw_height, l, r, t, b)| {
            let high_strategy = if H264_HIGH_PROFILES.contains(&profile_idc) {
                high_profile_fields_strategy().boxed()
            } else {
                Just(HighProfileFields::default()).boxed()
            };
            high_strategy.prop_map(move |high| SpsBuildParams {
                profile_idc,
                constraint_set_flags: 0,
                level_idc: 31,
                chroma_format_idc: high.chroma_format_idc,
                bit_depth_luma_minus8: high.bit_depth_luma_minus8,
                bit_depth_chroma_minus8: high.bit_depth_chroma_minus8,
                raw_width,
                raw_height,
                frame_mbs_only_flag: true,
                seq_scaling_matrix_present_flag: false,
                pic_order_cnt_type: 2,
                frame_cropping: Some((l, r, t, b)),
            })
        })
}

/// `SpsBuildParams` から chroma_array_type を算出する (仕様 7.4.2.1.1、separate_colour_plane_flag=0 前提)。
fn high_profile_chroma_array_type(params: &SpsBuildParams) -> u32 {
    if H264_HIGH_PROFILES.contains(&params.profile_idc) {
        u32::from(params.chroma_format_idc)
    } else {
        1
    }
}

/// cropping 適用後 width の期待値 (`parse_sps::read_dimensions_with_cropping` と同等ロジック)
fn expected_cropped_width(params: &SpsBuildParams) -> u32 {
    let chroma_array_type = high_profile_chroma_array_type(params);
    let crop_unit_x = match chroma_array_type {
        0 | 3 => 1,
        1 | 2 => 2,
        _ => unreachable!("Strategy で 0..=3 のみ生成"),
    };
    let Some((l, r, _, _)) = params.frame_cropping else {
        return params.raw_width;
    };
    params.raw_width - (l + r) * crop_unit_x
}

/// cropping 適用後 height の期待値
///
/// CropUnitY = chroma_array_type=0 で `frame_mbs_factor`、=1 で `2 * frame_mbs_factor`、
/// =2 で `frame_mbs_factor` (SubHeightC=1)、=3 で `frame_mbs_factor` (SubHeightC=1)。
/// frame_mbs_factor = 2 - frame_mbs_only_flag (仕様 6.2 / 7.4.2.1.1)。
fn expected_cropped_height(params: &SpsBuildParams) -> u32 {
    let chroma_array_type = high_profile_chroma_array_type(params);
    let frame_mbs_factor = if params.frame_mbs_only_flag { 1 } else { 2 };
    let crop_unit_y = match chroma_array_type {
        0 => frame_mbs_factor,
        1 => 2 * frame_mbs_factor,
        2 | 3 => frame_mbs_factor,
        _ => unreachable!("Strategy で 0..=3 のみ生成"),
    };
    let raw_height = params.raw_height * frame_mbs_factor;
    let Some((_, _, t, b)) = params.frame_cropping else {
        return raw_height;
    };
    raw_height - (t + b) * crop_unit_y
}

// ----------------------------------------------------------------
// Ok 経路: 構造化 Strategy で生成した SPS が parse_sps を Ok で通過し、
// avcc_box / visual / VideoFrameSize に round-trip / 整合が確認できること
// ----------------------------------------------------------------

mod ok_path {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            .. ProptestConfig::default()
        })]

        /// avc_profile_indication / profile_compatibility / avc_level_indication が
        /// SPS の入力値とそのまま一致すること (round-trip)
        #[test]
        fn prop_h264_sample_entry_round_trips_profile_level_constraint(
            params in ok_sps_strategy(),
        ) {
            let sps = build_sps_for_pbt(params);
            let (entry, _frame_size) = h264_sample_entry_from_sps_pps_lists(
                vec![sps.clone()],
                vec![PPS_NAL.to_vec()],
            )
            .map_err(|e| TestCaseError::fail(format!(
                "Ok 経路 SPS が parse_sps で Err になった: {e:?}"
            )))?;
            let SampleEntry::Avc1(avc1) = entry else {
                prop_assert!(false, "Avc1 SampleEntry を期待したが他の variant が返った");
                unreachable!()
            };
            prop_assert_eq!(
                avc1.avcc_box.avc_profile_indication,
                params.profile_idc,
                "avc_profile_indication が SPS の profile_idc と一致しない"
            );
            prop_assert_eq!(
                avc1.avcc_box.profile_compatibility,
                params.constraint_set_flags,
                "profile_compatibility が SPS の constraint_set_flags と一致しない"
            );
            prop_assert_eq!(
                avc1.avcc_box.avc_level_indication,
                params.level_idc,
                "avc_level_indication が SPS の level_idc と一致しない"
            );
        }

        /// High 系プロファイル時のみ avcc_box.chroma_format / bit_depth_* が Some になり、
        /// 非 High 系では None になること (構築不変条件の外形検証 + round-trip)
        #[test]
        fn prop_h264_sample_entry_reflects_high_profile_fields(
            params in ok_sps_strategy(),
        ) {
            let sps = build_sps_for_pbt(params);
            let (entry, _frame_size) = h264_sample_entry_from_sps_pps_lists(
                vec![sps.clone()],
                vec![PPS_NAL.to_vec()],
            )
            .map_err(|e| TestCaseError::fail(format!(
                "Ok 経路 SPS が parse_sps で Err になった: {e:?}"
            )))?;
            let SampleEntry::Avc1(avc1) = entry else {
                prop_assert!(false, "Avc1 SampleEntry を期待したが他の variant が返った");
                unreachable!()
            };
            let is_high = H264_HIGH_PROFILES.contains(&params.profile_idc);
            prop_assert_eq!(
                avc1.avcc_box.chroma_format.is_some(),
                is_high,
                "chroma_format.is_some() が H264_HIGH_PROFILES 判定と一致しない"
            );
            prop_assert_eq!(
                avc1.avcc_box.bit_depth_luma_minus8.is_some(),
                is_high,
                "bit_depth_luma_minus8.is_some() が H264_HIGH_PROFILES 判定と一致しない"
            );
            prop_assert_eq!(
                avc1.avcc_box.bit_depth_chroma_minus8.is_some(),
                is_high,
                "bit_depth_chroma_minus8.is_some() が H264_HIGH_PROFILES 判定と一致しない"
            );
            if is_high {
                prop_assert_eq!(
                    avc1.avcc_box.chroma_format.expect("High 系で Some").get(),
                    params.chroma_format_idc,
                    "High 系の chroma_format が SPS の chroma_format_idc と一致しない"
                );
                prop_assert_eq!(
                    avc1.avcc_box
                        .bit_depth_luma_minus8
                        .expect("High 系で Some")
                        .get(),
                    params.bit_depth_luma_minus8,
                    "High 系の bit_depth_luma_minus8 が SPS と一致しない"
                );
                prop_assert_eq!(
                    avc1.avcc_box
                        .bit_depth_chroma_minus8
                        .expect("High 系で Some")
                        .get(),
                    params.bit_depth_chroma_minus8,
                    "High 系の bit_depth_chroma_minus8 が SPS と一致しない"
                );
            }
        }

        /// avcc_box.sps_list / pps_list が呼び出し時に渡したバイト列をそのまま保持すること
        #[test]
        fn prop_h264_sample_entry_preserves_sps_pps_lists(
            params in ok_sps_strategy(),
        ) {
            let sps = build_sps_for_pbt(params);
            let pps = PPS_NAL.to_vec();
            let (entry, _frame_size) = h264_sample_entry_from_sps_pps_lists(
                vec![sps.clone()],
                vec![pps.clone()],
            )
            .map_err(|e| TestCaseError::fail(format!(
                "Ok 経路 SPS が parse_sps で Err になった: {e:?}"
            )))?;
            let SampleEntry::Avc1(avc1) = entry else {
                prop_assert!(false, "Avc1 SampleEntry を期待したが他の variant が返った");
                unreachable!()
            };
            prop_assert_eq!(avc1.avcc_box.sps_list.len(), 1, "sps_list の件数が 1 でない");
            prop_assert_eq!(
                format!("{:02x?}", &avc1.avcc_box.sps_list[0]),
                format!("{:02x?}", &sps),
                "avcc_box.sps_list[0] が入力 SPS と一致しない"
            );
            prop_assert_eq!(avc1.avcc_box.pps_list.len(), 1, "pps_list の件数が 1 でない");
            prop_assert_eq!(
                format!("{:02x?}", &avc1.avcc_box.pps_list[0]),
                format!("{:02x?}", &pps),
                "avcc_box.pps_list[0] が入力 PPS と一致しない"
            );
        }

        /// `Avc1Box.visual.width / .height` と戻り値タプルの `VideoFrameSize` が型を揃えた比較で一致すること
        #[test]
        fn prop_h264_sample_entry_visual_matches_frame_size(
            params in ok_sps_strategy(),
        ) {
            let sps = build_sps_for_pbt(params);
            let (entry, frame_size) = h264_sample_entry_from_sps_pps_lists(
                vec![sps.clone()],
                vec![PPS_NAL.to_vec()],
            )
            .map_err(|e| TestCaseError::fail(format!(
                "Ok 経路 SPS が parse_sps で Err になった: {e:?}"
            )))?;
            let SampleEntry::Avc1(avc1) = entry else {
                prop_assert!(false, "Avc1 SampleEntry を期待したが他の variant が返った");
                unreachable!()
            };
            prop_assert_eq!(
                avc1.visual.width as usize,
                frame_size.width,
                "visual.width と VideoFrameSize.width が一致しない"
            );
            prop_assert_eq!(
                avc1.visual.height as usize,
                frame_size.height,
                "visual.height と VideoFrameSize.height が一致しない"
            );
        }

        /// cropping 適用後の解像度が parse_sps のロジックと一致すること
        #[test]
        fn prop_h264_sample_entry_reflects_cropping_in_visual_and_frame_size(
            params in ok_sps_with_cropping_strategy(),
        ) {
            let sps = build_sps_for_pbt(params);
            let (entry, frame_size) = h264_sample_entry_from_sps_pps_lists(
                vec![sps.clone()],
                vec![PPS_NAL.to_vec()],
            )
            .map_err(|e| TestCaseError::fail(format!(
                "Ok 経路 (cropping あり) SPS が parse_sps で Err になった: {e:?}"
            )))?;
            let SampleEntry::Avc1(avc1) = entry else {
                prop_assert!(false, "Avc1 SampleEntry を期待したが他の variant が返った");
                unreachable!()
            };
            let expected_w = expected_cropped_width(&params);
            let expected_h = expected_cropped_height(&params);
            prop_assert_eq!(
                frame_size.width as u32,
                expected_w,
                "frame_size.width が cropping 期待値と一致しない"
            );
            prop_assert_eq!(
                frame_size.height as u32,
                expected_h,
                "frame_size.height が cropping 期待値と一致しない"
            );
            prop_assert_eq!(
                avc1.visual.width as u32,
                expected_w,
                "visual.width が cropping 期待値と一致しない"
            );
            prop_assert_eq!(
                avc1.visual.height as u32,
                expected_h,
                "visual.height が cropping 期待値と一致しない"
            );
        }
    }
}

// ----------------------------------------------------------------
// Err 経路: 仕様外入力を Strategy でスイープし、parse_sps の境界判定が崩れていないことを確認する。
// 各テストは検証対象の Err 条件のみを Strategy 変動させ、他フィールドは Baseline 系の Ok 固定とする。
// メッセージ文字列は検証しない (`is_err()` のみ)。代表値での Err 検証は単体テスト側が維持する。
// ----------------------------------------------------------------

mod err_path {
    use super::*;

    /// 仕様準拠和集合 `{66, 77, 88} ∪ H264_HIGH_PROFILES` 外の profile_idc を生成する Strategy
    fn unsupported_profile_idc_strategy() -> impl Strategy<Value = u8> {
        any::<u8>().prop_filter("supported プロファイル群外を選ぶ", |p| {
            !H264_HIGH_PROFILES.contains(p) && !matches!(*p, 66 | 77 | 88)
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            .. ProptestConfig::default()
        })]

        /// profile_idc が仕様準拠和集合外なら Err
        #[test]
        fn prop_h264_sample_entry_rejects_unsupported_profile_idc(
            profile_idc in unsupported_profile_idc_strategy(),
        ) {
            let params = SpsBuildParams {
                profile_idc,
                constraint_set_flags: 0,
                level_idc: 31,
                chroma_format_idc: 1,
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8: 0,
                raw_width: 320,
                raw_height: 240,
                frame_mbs_only_flag: true,
                seq_scaling_matrix_present_flag: false,
                pic_order_cnt_type: 2,
                frame_cropping: None,
            };
            let sps = build_sps_for_pbt(params);
            let result = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()]);
            prop_assert!(
                result.is_err(),
                "profile_idc={profile_idc} は仕様外プロファイルなので Err になるはず"
            );
        }

        /// High 系プロファイル + chroma_format_idc を 0..=7 でスイープし、境界 (≤3: Ok / ≥4: Err) を検証
        #[test]
        fn prop_h264_sample_entry_chroma_format_idc_boundary(
            high_profile_idc in prop::sample::select(&H264_HIGH_PROFILES[..]),
            chroma_format_idc in 0u8..=7,
        ) {
            let params = SpsBuildParams {
                profile_idc: high_profile_idc,
                constraint_set_flags: 0,
                level_idc: 31,
                chroma_format_idc,
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8: 0,
                raw_width: 320,
                raw_height: 240,
                frame_mbs_only_flag: true,
                seq_scaling_matrix_present_flag: false,
                pic_order_cnt_type: 2,
                frame_cropping: None,
            };
            let sps = build_sps_for_pbt(params);
            let result = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()]);
            if chroma_format_idc <= 3 {
                prop_assert!(
                    result.is_ok(),
                    "chroma_format_idc={chroma_format_idc} (≤3) は Ok のはず: {result:?}"
                );
            } else {
                prop_assert!(
                    result.is_err(),
                    "chroma_format_idc={chroma_format_idc} (≥4) は Err のはず"
                );
            }
        }

        /// High 系 + bit_depth_luma_minus8 を 0..=7 でスイープし、境界 (≤6: Ok / =7: Err) を検証
        #[test]
        fn prop_h264_sample_entry_bit_depth_luma_minus8_boundary(
            high_profile_idc in prop::sample::select(&H264_HIGH_PROFILES[..]),
            bit_depth_luma_minus8 in 0u8..=7,
        ) {
            let params = SpsBuildParams {
                profile_idc: high_profile_idc,
                constraint_set_flags: 0,
                level_idc: 31,
                chroma_format_idc: 1,
                bit_depth_luma_minus8,
                bit_depth_chroma_minus8: 0,
                raw_width: 320,
                raw_height: 240,
                frame_mbs_only_flag: true,
                seq_scaling_matrix_present_flag: false,
                pic_order_cnt_type: 2,
                frame_cropping: None,
            };
            let sps = build_sps_for_pbt(params);
            let result = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()]);
            if bit_depth_luma_minus8 <= 6 {
                prop_assert!(
                    result.is_ok(),
                    "bit_depth_luma_minus8={bit_depth_luma_minus8} (≤6) は Ok のはず: {result:?}"
                );
            } else {
                prop_assert!(
                    result.is_err(),
                    "bit_depth_luma_minus8={bit_depth_luma_minus8} (=7) は Err のはず"
                );
            }
        }

        /// High 系 + bit_depth_chroma_minus8 を 0..=7 でスイープし、境界 (≤6: Ok / =7: Err) を検証
        #[test]
        fn prop_h264_sample_entry_bit_depth_chroma_minus8_boundary(
            high_profile_idc in prop::sample::select(&H264_HIGH_PROFILES[..]),
            bit_depth_chroma_minus8 in 0u8..=7,
        ) {
            let params = SpsBuildParams {
                profile_idc: high_profile_idc,
                constraint_set_flags: 0,
                level_idc: 31,
                chroma_format_idc: 1,
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8,
                raw_width: 320,
                raw_height: 240,
                frame_mbs_only_flag: true,
                seq_scaling_matrix_present_flag: false,
                pic_order_cnt_type: 2,
                frame_cropping: None,
            };
            let sps = build_sps_for_pbt(params);
            let result = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()]);
            if bit_depth_chroma_minus8 <= 6 {
                prop_assert!(
                    result.is_ok(),
                    "bit_depth_chroma_minus8={bit_depth_chroma_minus8} (≤6) は Ok のはず: {result:?}"
                );
            } else {
                prop_assert!(
                    result.is_err(),
                    "bit_depth_chroma_minus8={bit_depth_chroma_minus8} (=7) は Err のはず"
                );
            }
        }

        /// pic_order_cnt_type の境界 + 巨大値で Err 値域を検証する
        ///
        /// `value=u32::MAX` のとき `PbtSpsBitWriter::write_ue` 内の `value.checked_add(1).expect(...)` で
        /// panic するため、本 PBT では実用範囲 (上限 100_000) のスイープで境界 (≤2: Ok / ≥3: Err) を検証する。
        #[test]
        fn prop_h264_sample_entry_pic_order_cnt_type_boundary(
            pic_order_cnt_type in prop::sample::select(vec![0u32, 1, 2, 3, 4, 100, 1000, 100_000]),
        ) {
            let params = SpsBuildParams {
                profile_idc: 66,
                constraint_set_flags: 0,
                level_idc: 31,
                chroma_format_idc: 1,
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8: 0,
                raw_width: 320,
                raw_height: 240,
                frame_mbs_only_flag: true,
                seq_scaling_matrix_present_flag: false,
                pic_order_cnt_type,
                frame_cropping: None,
            };
            let sps = build_sps_for_pbt(params);
            let result = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()]);
            if pic_order_cnt_type <= 2 {
                prop_assert!(
                    result.is_ok(),
                    "pic_order_cnt_type={pic_order_cnt_type} (≤2) は Ok のはず: {result:?}"
                );
            } else {
                prop_assert!(
                    result.is_err(),
                    "pic_order_cnt_type={pic_order_cnt_type} (≥3) は Err のはず"
                );
            }
        }

        /// pic_width_in_mbs_minus1 ≥ 4095 で raw_width ≥ u16::MAX + 1 となり Err
        #[test]
        fn prop_h264_sample_entry_width_exceeding_u16_max_boundary(
            mb_count in 4090u32..=4100,
        ) {
            let raw_width = mb_count * 16; // 65440..=65600 (u16::MAX 周辺)
            let params = SpsBuildParams {
                profile_idc: 66,
                constraint_set_flags: 0,
                level_idc: 31,
                chroma_format_idc: 1,
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8: 0,
                raw_width,
                raw_height: 16,
                frame_mbs_only_flag: true,
                seq_scaling_matrix_present_flag: false,
                pic_order_cnt_type: 2,
                frame_cropping: None,
            };
            let sps = build_sps_for_pbt(params);
            let result = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()]);
            if raw_width <= u16::MAX as u32 {
                prop_assert!(
                    result.is_ok(),
                    "raw_width={raw_width} (≤u16::MAX) は Ok のはず: {result:?}"
                );
            } else {
                prop_assert!(
                    result.is_err(),
                    "raw_width={raw_width} (>u16::MAX) は Err のはず"
                );
            }
        }
    }
}
