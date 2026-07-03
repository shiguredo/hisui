# [BUG] pbt/tests/prop_h264_sps.rs の ok_path 系 4 テストが bit reader exhausted で確定失敗する

- Priority: High
- Created: 2026-07-03
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/fix-h264-sps-pbt-bit-reader-exhausted
- Polished: {YYYY-MM-DD}

## 目的

`pbt/tests/prop_h264_sps.rs` の `ok_path::` 系 5 テストのうち 4 テストが、`parse_sps` の bit reader 枯渇で確定的に失敗する。原因を特定して修正する。

## 優先度根拠

High。ローカル環境で確定的に再現し、develop 単体で失敗する。CI で seed 差により偶然通っているだけの可能性が高く、seed が回ったタイミングで突如 CI が赤くなり得る。h264 SPS パーサは inbound endpoint / mp4 reader / video decoder 経路の骨格でもあるため、パーサ側の実バグならば録画データの読み込みにも影響する。

## 現状

### 症状

- **失敗テスト (4 件)**:
  - `ok_path::prop_h264_sample_entry_round_trips_profile_level_constraint`
  - `ok_path::prop_h264_sample_entry_reflects_high_profile_fields`
  - `ok_path::prop_h264_sample_entry_preserves_sps_pps_lists`
  - `ok_path::prop_h264_sample_entry_visual_matches_frame_size`
- **pass するテスト**: 同じ `ok_path::` 内の `reflects_cropping_in_visual_and_frame_size`、`err_path::` 系 7 件すべて
- **panic 場所**: `pbt/tests/prop_h264_sps.rs:237` (proptest! マクロ)
- **内部エラー**: `Ok 経路 SPS が parse_sps で Err になった: bit reader: exhausted before requested read (at src/video/bit_reader.rs:48)`

### 再現手順

```
git checkout develop
SDKROOT=$(xcrun --sdk macosx --show-sdk-path) cargo test -p pbt --test prop_h264_sps
```

3 回連続で同じ 4 テストが失敗することを確認済み (feature ブランチ / develop 単体の両方)。

### Minimal failing input

proptest の shrink で得られた最小失敗ケース (`prop_h264_sample_entry_visual_matches_frame_size`):

```
SpsBuildParams {
    profile_idc: 66,
    constraint_set_flags: 0,
    level_idc: 0,
    chroma_format_idc: 0,
    bit_depth_luma_minus8: 0,
    bit_depth_chroma_minus8: 0,
    raw_width: 32768,
    raw_height: 53536,
    frame_mbs_only_flag: true,
    seq_scaling_matrix_present_flag: false,
    pic_order_cnt_type: 0,
    log2_max_pic_order_cnt_lsb_minus4: 3,
    num_ref_frames_in_pic_order_cnt_cycle: 0,
    frame_cropping: None,
}
```

### 想定される原因

- **(1) `ok_sps_strategy()` の SPS 生成側**: 生成する SPS bitstream が想定より短い / ビット揃えが不正確で、parse_sps が読み切れずに枯渇する
- **(2) `build_sps_for_pbt` のシリアライズ不完全**: 特定パラメタ組み合わせで trailing bits が省略される
- **(3) `parse_sps` の実装側**: 本来 Ok で通るべきパラメタを reject している (spec 解釈の誤り)

いずれも `pbt/tests/prop_h264_sps.rs` 側または `src/video/h264` 系の parse_sps 実装のいずれかにバグがある。

## 設計方針

1. まず minimal failing input で `build_sps_for_pbt` → シリアライズされたバイト列を dump して、SPS の意図した bit 位置構造と実バイト長を突き合わせる
2. `parse_sps` の bit_reader 消費経過を追い、どの field で exhausted になっているかを特定する
3. 特定した箇所が Strategy 側の生成漏れなら Strategy を修正、`parse_sps` 側のバグなら parser を修正する

## 完了条件

- `cargo test -p pbt --test prop_h264_sps` が develop で 3 回連続 pass する
- `cargo test --workspace --features candle` が全 target で pass する

## 解決方法

上記設計方針に従って原因を特定し、`pbt/tests/prop_h264_sps.rs` の Strategy か `src/video/h264` 系の SPS parser 実装を修正する。回帰防止のため、修正で fix される元 minimal input を再現テストとして残す。

## 参考

- panic 起源: `src/video/bit_reader.rs:48` (`read_bit` で `byte_pos >= data.len()` 時のエラー)
- 直近の develop マージ: PR #302 `feature/refactor-mp4-reader-async-video-decoder` (h264 関連ロジックに間接影響の可能性、要確認)
