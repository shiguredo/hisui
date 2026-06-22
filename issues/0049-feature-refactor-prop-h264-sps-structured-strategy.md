# prop_h264_sps の PBT を構造化 Strategy に置き換えてクラッシュフリー専用テストを fuzz/ に移管する

- Priority: Low
- Created: 2026-06-19
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-prop-h264-sps-structured-strategy
- Polished: 2026-06-22

## 目的

`pbt/tests/prop_h264_sps.rs` は次の 2 つの問題を抱えている。

1. **本番経路から外れた PBT ターゲット**: issue 0043 (closed) で `h264_sample_entry_from_annexb` は `h264_sample_entry_from_sps_pps_lists → parse_sps` 経由に変更され、本番経路から `extract_dimensions_from_sps` の呼び出しは消えた。`src/video/h264.rs::extract_dimensions_from_sps` の docstring も「本関数は本番経路からは呼ばれず、`pbt/tests/prop_h264_sps.rs` のクラッシュフリー PBT 専用の公開 API として残している」と明記。PBT は依然としてこの PBT 専用ラッパー経由なため、本番経路から離れた位置で検証している。
2. **クラッシュフリー専用テスト規約違反**: 現状の PBT は「任意入力でパニックや無限ループを起こさず Ok か Err を返すこと」のみを検証し、戻り値を `let _ = ...` で捨てている。これは `shiguredo-rust` 規約「PBT に『任意入力でパニックしないことだけを検証するテスト』を書かないこと（fuzzing の役割）」に反する。

本 issue では PBT を構造化 Strategy で SPS バイト列を生成し、本番入口 `h264_sample_entry_from_sps_pps_lists` の戻り値 (`SampleEntry::Avc1(Avc1Box { avcc_box, visual, .. })` と `VideoFrameSize`) を **外形 API として観測** する形で仕様準拠の不変条件 (round-trip 性 / High プロファイル判定の整合 / cropping 反映 等) を検証する。クラッシュフリー検証は cargo-fuzz の fuzz ターゲットに移管し、本番入口 `h264_sample_entry_from_sps_pps_lists` を直接 fuzz する。

PBT 構造化と fuzz 移管を **同一 issue 内で同時に実施** する根拠: PBT を構造化するだけでは現状 `let _ = extract_dimensions_from_sps(&sps);` (1024 cases) で担保していたクラッシュフリー検証が消失する。fuzz 移管を別 issue に分離すると本 issue 完了時点でクラッシュフリー検証が穴になり、`Don't live with broken windows` (CLAUDE.md) に反する。よって fuzz 一式 (新規 crate / fuzz target / シード) も本 issue に含める。

本 issue 完了後は `extract_dimensions_from_sps` の呼び出し元がゼロになるため、同関数の削除も本 issue の完了条件に含める (詳細は `### §1.4 extract_dimensions_from_sps の削除` 参照)。

## 優先度根拠

Low。テスト戦略の規約準拠が主目的で、バグや機能面の問題は発生していない。

- 現状の PBT もクラッシュフリー性質自体は担保しているため、本番品質に影響しない。
- 構造化 Strategy 化で proptest の探索空間は狭まる (現状の任意バイト列 0..=4096 → 構造化生成された数十バイト SPS) が、クラッシュフリー保証は fuzz/ 側で本番入口関数を直接探索する形で補強するため、トータルのテスト網羅性は維持または向上する想定。
- issue 0043 で本番経路が変わった以上、PBT のターゲット選択を見直す機会として扱う。
- `Don't live with broken windows` (CLAUDE.md): `extract_dimensions_from_sps` の呼び出し元がゼロになる中間状態を残さず、本 issue 完了と同時に削除する。

## 現状

行番号は実装着手時に grep で再特定する。以下の grep キーが使える:

```
rg -n 'const H264_HIGH_PROFILES' src/video/h264.rs                # §1.2 で pub 化対象
rg -n 'pub fn extract_dimensions_from_sps' src/video/h264.rs      # §1.4 削除対象
rg -n 'fn parse_sps' src/video/h264.rs                            # private fn のまま維持
rg -n 'struct SpsBuilder' src/video/h264.rs                       # #[cfg(test)] 内 private のまま維持
rg -n 'struct SpsBitWriter' src/video/h264.rs                     # 同上
rg -n 'pub fn h264_sample_entry_from_sps_pps_lists' src/video/h264.rs
rg -nE '^\s*fn extract_dimensions_' src/video/h264.rs             # 既存テスト 13 件 (11 件は置き換え、2 件 #4 #12 は削除)
rg -nE 'pub\(crate\) const SPS_320X240|pub\(crate\) const SPS_1920X1080' src/video/h264.rs
```

### `pbt/tests/prop_h264_sps.rs`

全 30 行。1024 cases で `extract_dimensions_from_sps` に「先頭 1 バイトを `0x67` (SPS NAL ヘッダ) に固定 + 残り任意バイト列 (0..=4096 バイト)」を投入し、戻り値を `let _ = ...` で捨ててクラッシュフリーのみ検証する。

### `src/video/h264.rs` 関連シンボルの可視性

`#[cfg(test)] pub(crate) mod tests` 配下:

- `struct SpsBuilder`: 可視性修飾子なしの **private struct**。`with_*` setter / `raw` / `build` も全 private。
- `struct SpsBitWriter`: 同じく全 private。
- `const SPS_320X240` / `SPS_1920X1080` 等: `pub(crate)` (同一クレート内テストモジュール間共有のため)。
- `const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2]`: private const (`h264_sample_entry_from_sps_pps_lists_*` テスト群で利用)。

モジュール外:

- `const H264_HIGH_PROFILES: [u8; 13]`: **private const**。コメントで ITU-T H.264 (2017/06) 仕様 7.3.2.1.1 由来と明記。
- `pub fn extract_dimensions_from_sps(sps: &[u8]) -> crate::Result<(usize, usize)>`: 既存 pub。`parse_sps` の薄いラッパー (`parse_sps(sps).map(|p| (p.width as usize, p.height as usize))`)。
- `pub fn h264_sample_entry_from_sps_pps_lists(sps_list: Vec<Vec<u8>>, pps_list: Vec<Vec<u8>>) -> crate::Result<(SampleEntry, VideoFrameSize)>`: 本番入口、既存 pub。引数 Vec は move 取得。
- `fn parse_sps(sps: &[u8]) -> crate::Result<SpsParams>`: **private fn**。
- `struct SpsParams` / `struct HighProfileSpsParams`: **private struct**。

`pbt/` は別クレートで、`pbt/Cargo.toml` で `[dev-dependencies] hisui = { path = ".." }`。Rust の可視性ルール上、別クレートからは:

- 本体クレート側の `#[cfg(test)]` モジュールは **本体クレートのテストビルド以外では存在しない** ため `pub` を付けても見えない (`cfg(test)` は依存クレートに伝播しない)。
- `pub(crate)` で公開されたシンボルも別クレートからは見えない。

したがって `SpsBuilder` / `SpsBitWriter` / `H264_HIGH_PROFILES` / `parse_sps` / `SpsParams` のいずれも、別クレート `pbt/` から直接参照する手段は現状ない。

### `shiguredo_mp4` の Uint 型

`Avc1Box.avcc_box` (shiguredo_mp4 2026.3.0) のフィールド型:

- `chroma_format: Option<Uint<u8, 2>>` (2 bit、`0..=3` の値専用)
- `bit_depth_luma_minus8: Option<Uint<u8, 3>>` (3 bit、`0..=7` の値専用)
- `bit_depth_chroma_minus8: Option<Uint<u8, 3>>` (同上)

`Uint::new(v)` は値域外を渡すとデバッグビルドで panic する。PBT の round-trip 比較で `Uint::new` を呼ぶ場合は値域内のみで使う (`parse_sps` の値域 Err 化で Err 経路では `params.high_profile_params` が `None` のため、Err 経路 PBT で `Uint::new` を呼ぶ経路は parse_sps 側に存在しない)。

### Rust toolchain

`rust-toolchain.toml` で `channel = "stable"` 固定。`cargo-fuzz` のデフォルト動作 (AddressSanitizer 有効) は nightly 必須のため、本 issue では nightly を前提とする。`--sanitizer none` 等の stable 経路は本 issue のスコープ外。

### `fuzz/` ディレクトリ

存在しない。`cargo fuzz init` で新規初期化する。

### workspace 構成

ルート `Cargo.toml` の `[workspace] members = [..., "pbt"]`。`fuzz/` を members に追加すると `cargo build --workspace` で `libfuzzer-sys` (nightly 依存) が巻き込まれて本体ビルドが壊れる懸念があるため、`fuzz/Cargo.toml` に空 `[workspace]` 節を置いて独立 workspace 化する。`cargo fuzz init` のバージョンによっては生成物に空 `[workspace]` 節が含まれない場合があるため、生成直後に有無を確認し無ければ追記する。

## 設計方針

### §1 PBT の構造化 Strategy 化

`pbt/tests/prop_h264_sps.rs` を構造化 Strategy で SPS バイト列を生成し、本番入口 `h264_sample_entry_from_sps_pps_lists` の戻り値を外形 API として観測して仕様準拠の不変条件を検証する形に書き換える。

#### §1.1 PBT 用 SPS ビルダーヘルパーを `src/video/h264.rs` に新設 (案 D-(b) 採用)

**確定方針**: `SpsBuilder` を本体に pub 化する案ではなく、**PBT 専用の pub fn ヘルパー** `pub fn build_sps_for_pbt(...)` を `src/video/h264.rs` のモジュール外 (非 `#[cfg(test)]` 領域) に新設し、その内部に SPS ビット組み立てロジックを cfg なし領域で **独立実装する** (案 D-(b) を採用)。`SpsBuilder` / `SpsBitWriter` は `#[cfg(test)] mod tests` 配下の private 構造体として現状維持する。これにより本体に出る API サーフェスを `build_sps_for_pbt` + `SpsBuildParams` のみに最小化する。

##### 採用案の比較

| 案 | 内容 | 不採用理由 / 採用理由 |
| --- | --- | --- |
| A | `SpsBuilder` を本体コードに昇格 + pub 化、`mod tests` の `#[cfg(test)]` 削除 | 不採用: テスト用 struct + 全 `with_*` setter + `SpsBitWriter` を本番バイナリに常時含めることになり、API サーフェスが過大 |
| B | feature gate `#[cfg(any(test, feature = "pbt-helpers"))]` で `SpsBuilder` / `SpsBitWriter` を公開 | 不採用: `pbt/Cargo.toml` で feature 指定が必要になり、本体クレートの公開 feature が増える副作用 |
| C | pbt 側に `SpsBuilder` / `SpsBitWriter` 一式を複製 | 不採用: 数百行の重複コードと二重メンテのリスク |
| D-(a) | `SpsBuilder` を `pub` 化しつつ cfg gate を `#[cfg(any(test, feature = "pbt-helpers"))]` に緩めて `build_sps_for_pbt` から呼ぶ | 不採用: 実質的に案 A + 案 B の合成で副作用が重なる |
| D-(b) | `build_sps_for_pbt` 用に SPS ビット組み立ての最小ロジックを cfg なし領域に独立実装する (本案) | 採用: 本体に出る API サーフェスを `build_sps_for_pbt` + `SpsBuildParams` のみに最小化できる。実装重複は `SpsBuilder::build` 本体 (約 80 行) + `SpsBitWriter` 相当のビット書き込みヘルパー (約 60 行) を合わせて **約 140 行**。`SpsBuilder` (cfg test 内) は既存 test 用 fluent API を引き続き提供し、`build_sps_for_pbt` (cfg なし) は単発関数として並走する |

##### 二重メンテ検出機構

案 D-(b) の二重メンテ問題 (`SpsBuilder::build` 改修時に `build_sps_for_pbt` 側にも同じ変更を入れる必要) は、`src/video/h264.rs::tests` 内に **乖離検出単体テスト** `sps_builder_and_build_sps_for_pbt_emit_byte_compatible_sps_*` を Baseline / Main / High10 の 3 経路ぶん追加して防ぐ。Baseline 1 ケースだけだと `is_high` 分岐 (`SpsBuilder::build` line 1128 付近) が `false` 評価され High 系プロファイル経路の乖離を検出できないため、High 経路も含めて担保する:

```rust
#[test]
fn sps_builder_and_build_sps_for_pbt_emit_byte_compatible_sps_baseline() {
    // Baseline (profile_idc=66) 320x240、crop なし、デフォルト。is_high=false 経路を担保。
    let from_builder = SpsBuilder::raw(320, 240).build();
    let from_pbt = build_sps_for_pbt(SpsBuildParams {
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
        pic_order_cnt_type: 2,
        frame_cropping: None,
    });
    assert_eq!(from_builder, from_pbt, "Baseline: SpsBuilder と build_sps_for_pbt のバイト列が乖離");
}

#[test]
fn sps_builder_and_build_sps_for_pbt_emit_byte_compatible_sps_main() {
    // Main (profile_idc=77) 1920x1088、デフォルト。is_high=false の別 profile_idc 経路。
    let from_builder = SpsBuilder::raw(1920, 1088).with_profile_idc(77).build();
    let from_pbt = build_sps_for_pbt(SpsBuildParams {
        profile_idc: 77,
        constraint_set_flags: 0,
        level_idc: 31,
        chroma_format_idc: 1,
        bit_depth_luma_minus8: 0,
        bit_depth_chroma_minus8: 0,
        raw_width: 1920,
        raw_height: 1088,
        frame_mbs_only_flag: true,
        seq_scaling_matrix_present_flag: false,
        pic_order_cnt_type: 2,
        frame_cropping: None,
    });
    assert_eq!(from_builder, from_pbt, "Main: SpsBuilder と build_sps_for_pbt のバイト列が乖離");
}

#[test]
fn sps_builder_and_build_sps_for_pbt_emit_byte_compatible_sps_high10() {
    // High10 (profile_idc=110) + bit_depth_luma_minus8=2 + bit_depth_chroma_minus8=2。
    // is_high=true 経路 + chroma_format_idc / bit_depth_* の SPS 書き込みを担保。
    let from_builder = SpsBuilder::raw(1920, 1088)
        .with_profile_idc(110)
        .with_bit_depth_luma_minus8(2)
        .with_bit_depth_chroma_minus8(2)
        .build();
    let from_pbt = build_sps_for_pbt(SpsBuildParams {
        profile_idc: 110,
        constraint_set_flags: 0,
        level_idc: 31,
        chroma_format_idc: 1,
        bit_depth_luma_minus8: 2,
        bit_depth_chroma_minus8: 2,
        raw_width: 1920,
        raw_height: 1088,
        frame_mbs_only_flag: true,
        seq_scaling_matrix_present_flag: false,
        pic_order_cnt_type: 2,
        frame_cropping: None,
    });
    assert_eq!(from_builder, from_pbt, "High10: SpsBuilder と build_sps_for_pbt のバイト列が乖離");
}
```

##### 公開シグネチャ

```rust
/// PBT (`pbt/tests/prop_h264_sps.rs`) から構造化 Strategy で SPS バイト列を生成するためのヘルパー。
///
/// 本関数はテスト戦略 (PBT) からのみ使うことを想定し、本番経路からは呼ばない。
/// 引数の値域は ITU-T H.264 仕様 7.3.2.1.1 / 7.4.2.1.1 に対応する。
/// `raw_width` / `raw_height` は 16 の倍数 (マクロブロック境界) であること。
/// Strategy 側で `(raw_width % 16 == 0) && (raw_height % 16 == 0)` を保証して渡す。
pub fn build_sps_for_pbt(params: SpsBuildParams) -> Vec<u8>;

/// PBT Strategy が SPS 生成のパラメータを束ねる pub 構造体。
///
/// 本構造体はテスト戦略 (PBT) からのみ使うことを想定し、本番経路からは利用しない。
#[derive(Debug, Clone, Copy)]
pub struct SpsBuildParams {
    pub profile_idc: u8,
    pub constraint_set_flags: u8,        // u8 全体、reserved_zero_2bits 含む (parse_sps は reserved 検査なし)
    pub level_idc: u8,
    pub chroma_format_idc: u8,           // 0..=7 (Ok 経路は 0..=3、Err 経路で 4..=7 も生成)
    pub bit_depth_luma_minus8: u8,       // 0..=7 (Ok 経路は 0..=6、Err 経路で 7 も生成)
    pub bit_depth_chroma_minus8: u8,     // 0..=7 (Ok 経路は 0..=6、Err 経路で 7 も生成)
    pub raw_width: u32,
    pub raw_height: u32,
    pub frame_mbs_only_flag: bool,
    pub seq_scaling_matrix_present_flag: bool,
    pub pic_order_cnt_type: u32,         // 0..=u32::MAX (Ok 経路は 0..=2、Err 経路で 3 以上の discrete 値)
    pub frame_cropping: Option<(u32, u32, u32, u32)>,
}
```

値渡し (`params: SpsBuildParams`) を採るのは、`Copy` 型なので借用との実コスト差がなく、呼び出し側の `&` 付け忘れミスを回避するため。Strategy 関数内で同 params を後段 assertion で参照したい場合は `let params = ...; let sps = build_sps_for_pbt(params); /* params もスコープに残る (Copy) */` と書ける。

`SpsBuildParams` のフィールド型は `u8` を基本とし、`raw_width` / `raw_height` / `pic_order_cnt_type` のみ `u32` (Exp-Golomb で `u32` 単位の書き込みになるため整合させる)。`chroma_format_idc` / `bit_depth_*_minus8` は値域 0..=7 で十分なため `u8`。これにより `Uint::new(params.chroma_format_idc)` のような assertion で `as u8` キャスト不要になる。

##### High 系プロファイル固有フィールドの SPS バイト列反映

`SpsBuildParams.chroma_format_idc` / `bit_depth_*_minus8` / `seq_scaling_matrix_present_flag` は **`profile_idc` が High 系のときのみ SPS バイト列に書き込まれる** (ITU-T H.264 仕様 7.3.2.1.1 の `if (profile_idc == 100 | 110 | ...)` ブロック)。Baseline / Main / Extended の場合は無視される。Strategy 側で:

- Ok 経路用 Strategy: `profile_idc` が High 系のときだけ `chroma_format_idc ∈ 0..=3` 等を非デフォルト値にする (`prop_flat_map` で分岐)。
- Err 経路用 Strategy: 検証対象 Err 条件と独立に他フィールドを固定する。

詳細は §1.3.2 の擬似コード参照。

#### §1.2 H264_HIGH_PROFILES を pub 化

`pbt/` 側の Strategy で「`profile_idc` を Baseline/Main/Extended `{66, 77, 88}` ∪ `H264_HIGH_PROFILES` から `prop::sample::select` で選ぶ」「`H264_HIGH_PROFILES.contains(&profile_idc)` の不変条件を assert する」ために `H264_HIGH_PROFILES` を `pub const` に昇格する。本配列は ITU-T H.264 仕様 7.3.2.1.1 由来で外部から参照されても意味が変わらない値。

pub 化に伴い、既存の `//` 行コメントを `///` docstring に書き換える (本体のコメント慣習に合わせて半角括弧で統一):

```rust
/// High 系プロファイル群 (ITU-T H.264 (2017/06) 仕様 7.3.2.1.1 の `if (profile_idc == ...)` 条件節)。
///
/// 該当プロファイルでは SPS に chroma_format_idc 以下の追加フィールド群が含まれる。
/// 仕様改訂で要素が増減する可能性があるため、将来の仕様アップデート時に同期する。
pub const H264_HIGH_PROFILES: [u8; 13] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];
```

`H264_HIGH_PROFILES.contains(&profile_idc)` は 13 要素線形探索だが、PBT cases 数 (Ok 経路 256 / Err 経路 128) では実害なし。本体 `parse_sps` 内でも同じ線形探索を使っているため、最適化 (`HashSet` 化等) は不要。

#### §1.3 PBT 検証経路は外形 API (案 C 採用)

`SpsParams` / `HighProfileSpsParams` / `parse_sps` の pub 化は採用しない (issue 0043 の設計判断「`h264_sample_entry_from_sps_pps_lists` の戻り値や引数には露出しないため pub にしない」を踏襲)。代わりに PBT は **`h264_sample_entry_from_sps_pps_lists(vec![sps], vec![pps])` の戻り値タプル `(SampleEntry::Avc1(Avc1Box { avcc_box, visual, .. }), VideoFrameSize)` を観測** する。

検証する不変条件 (Ok 経路):

| 不変条件 | 検証手段 / 注記 |
| --- | --- |
| `avcc_box.avc_profile_indication == params.profile_idc` (round-trip) | Strategy 入力と戻り値の `Avc1Box.avcc_box.avc_profile_indication` を比較 |
| `avcc_box.profile_compatibility == params.constraint_set_flags` (round-trip) | `constraint_set_flags` は u8 全体 (上位 6 bit = constraint_set0..5_flag、下位 2 bit = reserved_zero_2bits)。`parse_sps` は reserved 検査をしないため Strategy で `0u8..=255` の任意値を投入してそのまま round-trip する |
| `avcc_box.avc_level_indication == params.level_idc` (round-trip) | 同上 (`avcc_box.avc_level_indication`) |
| `avcc_box.chroma_format.is_some() == H264_HIGH_PROFILES.contains(&params.profile_idc)` | `parse_sps` 内構築不変条件の外形検証 |
| High 系プロファイル時 `avcc_box.chroma_format == Some(Uint::new(params.chroma_format_idc))` (round-trip) | Ok 経路では `params.chroma_format_idc ∈ 0..=3` のみ Strategy が生成するため `Uint<u8, 2>` の panic 回避 |
| High 系プロファイル時 `avcc_box.bit_depth_luma_minus8 == Some(Uint::new(params.bit_depth_luma_minus8))` (round-trip) | Ok 経路では値域 0..=6 のみ Strategy が生成 (`Uint<u8, 3>` 自体は 0..=7 で panic-free だが、parse_sps が 7 以上で Err を返すため Ok 経路では 7 が来ない) |
| High 系プロファイル時 `avcc_box.bit_depth_chroma_minus8 == Some(Uint::new(params.bit_depth_chroma_minus8))` (round-trip) | 同上 |
| `avcc_box.sps_list.len() == 1 && avcc_box.sps_list[0] == sps_input` | PBT 側で `sps_input.clone()` を関数に渡し、戻り値の `avcc_box.sps_list[0]` と比較する。件数と内容を分離して assert (失敗時メッセージ縮減のため hex フォーマット `format!("{:02x?}", ...)` を活用) |
| `avcc_box.pps_list.len() == 1 && avcc_box.pps_list[0] == PPS_NAL.to_vec()` | 同上 (`PPS_NAL.to_vec()`) |
| `visual.width as usize == frame_size.width && visual.height as usize == frame_size.height` | 戻り値タプル 2 要素間の整合 (型を揃えて比較。`u16 → usize` は無条件 lossless) |
| `frame_size.width == expected_cropped_width(&params) && frame_size.height == expected_cropped_height(&params)` (cropping 反映) | Strategy 入力から計算した期待値と戻り値の `VideoFrameSize` 比較 (`expected_cropped_*` ヘルパーは §1.3.2 に擬似実装) |

`visual.width > 0 && visual.height > 0` は `parse_sps` で常時保証 (実コード `width == 0 || height == 0` を Err 化) されているため、Ok 経路 PBT で重複検証しない (自明な恒真)。

##### 入力 Vec の move 注意

`h264_sample_entry_from_sps_pps_lists(sps_list: Vec<Vec<u8>>, pps_list: Vec<Vec<u8>>)` は引数 Vec を move 取得し `AvccBox.sps_list` / `pps_list` にそのまま渡す。PBT 側で `sps_input` / `pps_input` を assert で参照するためには **clone してから関数に渡す** 必要がある:

```rust
let sps = build_sps_for_pbt(params);
let pps = PPS_NAL.to_vec();
let (entry, frame_size) = h264_sample_entry_from_sps_pps_lists(vec![sps.clone()], vec![pps.clone()])?;
let SampleEntry::Avc1(avc1) = entry else { /* prop_assert! で失敗 */ };
prop_assert_eq!(avc1.avcc_box.sps_list.len(), 1);
prop_assert_eq!(format!("{:02x?}", &avc1.avcc_box.sps_list[0]), format!("{:02x?}", &sps));
```

検証する不変条件 (Err 経路、§1.3.1 で重複排除指針を遵守):

各 Err 経路は **検証対象 Err 条件のみを Strategy 変動させ、他フィールドは Baseline 系の Ok 固定** とすることで、複数 Err 条件が重なって意図しない経路で Err になることを防ぐ。

| 不変条件 | Strategy 構成 |
| --- | --- |
| `profile_idc` が `{66, 77, 88} ∪ H264_HIGH_PROFILES` 以外で Err | `profile_idc ∈ 0u8..=255` のうち上記和集合外を `prop::sample::select` で選ぶ。他フィールドは固定 (Baseline 系の Ok 値) |
| High 系プロファイル + `chroma_format_idc ∈ 4..=7` で Err | `profile_idc` は `H264_HIGH_PROFILES` から固定選択 (例: 100)、`chroma_format_idc ∈ 0u8..=7` をスイープし境界 (≤3: Ok / ≥4: Err) が崩れていないこと |
| High 系プロファイル + `bit_depth_luma_minus8 = 7` で Err | 同様に `bit_depth_luma_minus8 ∈ 0u8..=7` をスイープし境界 (≤6: Ok / =7: Err、ITU-T H.264 仕様 7.4.2.1.1 の値域は 0..=6) |
| High 系プロファイル + `bit_depth_chroma_minus8 = 7` で Err | 同上 |
| `pic_order_cnt_type ≥ 3` で Err (ITU-T H.264 仕様 7.4.2.1.1 の値域は 0..=2) | `pic_order_cnt_type ∈ prop::sample::select(vec![0u32, 1, 2, 3, 4, 100, 1000, u32::MAX / 2, u32::MAX])` で境界 + 巨大値 + `u32::MAX` を含めて Err スイープ (parse_sps 内の `read_ue` 経路 overflow も間接的に検証) |
| `pic_width_in_mbs_minus1 ≥ 4095` で `u16::MAX` 超え Err | `raw_width` を `4090*16..=4100*16` の境界スイープで Err 境界検証 |
| cropping アンダーフロー (`crop_left + crop_right` 等が raw を超える) で Err | Strategy で raw_width = 16 程度 (極小) + crop_offsets 広範囲生成し境界検出 |

PBT の Err 経路は **メッセージ文字列までは検証せず `is_err()` のみ確認** する。代表値検証 (メッセージ含む) は §1.3.1 の責務分担で既存単体テストに残す。

##### §1.3.1 既存単体テストとの責務分担

`src/video/h264.rs::tests` には既存単体テスト `parse_sps_rejects_unsupported_profile_idc` / `parse_sps_rejects_chroma_format_idc_out_of_range` / `parse_sps_rejects_bit_depth_luma_minus8_out_of_range` / `parse_sps_rejects_bit_depth_chroma_minus8_out_of_range` / `parse_sps_rejects_pic_order_cnt_type_out_of_range` 等が「特定の値 (例: `chroma_format_idc=4`) で Err」を代表値で検証している。これらは維持する。

PBT の Err 経路は **代表値検証ではなく値域全体のスイープ** とすることで責務を分離する。具体的には:

- 既存単体テスト: 代表値 1 点 (`chroma_format_idc=4`) で Err、エラーメッセージ文字列も検証。
- PBT: `chroma_format_idc ∈ prop::sample::select(0..=7)` の範囲で「≤3 で Ok、≥4 で Err」の境界位置自体が崩れていないことをスイープで保証。エラーメッセージ文字列は検証しない (`is_err()` のみ)。Strategy 化の付加価値は組合せ空間 (`profile_idc × chroma_format_idc × bit_depth × pic_order_cnt_type × frame_mbs_only_flag × frame_cropping_flag × crop_offsets`) の確率的探索。

これにより `shiguredo-rust` 規約「PBT でカバーできるものを単体テストで書かないこと」と「pbt 以下に unittest を書かないこと」の両方に整合する。

##### §1.3.2 PBT 関数命名と Strategy 構成方針

既存 `pbt/tests/prop_sample_entry.rs` と同じ命名規則 `prop_<観測対象>_<期待挙動>` を採用する。PBT 関数名は **`prop_h264_sample_entry_*`** 接頭辞で `src/video/h264.rs::tests` 内既存単体テスト `parse_sps_*` と接頭辞レベルで明確に区別する (PBT は `h264_sample_entry_from_sps_pps_lists` の戻り値経由、単体テストは `parse_sps` 直接呼びという検証対象差を関数名から読み取れる)。例:

- `prop_h264_sample_entry_round_trips_profile_indication`
- `prop_h264_sample_entry_reflects_high_profile_chroma_format`
- `prop_h264_sample_entry_reflects_cropping_in_visual_and_frame_size`
- `prop_h264_sample_entry_rejects_unsupported_profile_idc`
- `prop_h264_sample_entry_rejects_chroma_format_idc_above_three`

Strategy 構成の擬似コード:

```rust
fn supported_profile_idc() -> impl Strategy<Value = u8> {
    prop_oneof![
        Just(66u8), Just(77u8), Just(88u8),
        prop::sample::select(&hisui::video::h264::H264_HIGH_PROFILES[..]),
    ]
}

fn raw_width_strategy() -> impl Strategy<Value = u32> {
    // 16 倍数を保証 (SpsBuildParams の事前条件)。u16::MAX 内に収める範囲。
    (1u32..=4095).prop_map(|n| n * 16)
}

fn raw_height_strategy() -> impl Strategy<Value = u32> {
    (1u32..=2160).prop_map(|n| n * 16)
}

#[derive(Debug, Clone, Copy, Default)]
struct HighProfileFields {
    chroma_format_idc: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
}

fn high_profile_fields_strategy() -> impl Strategy<Value = HighProfileFields> {
    (0u8..=3, 0u8..=6, 0u8..=6).prop_map(|(c, l, c2)| HighProfileFields {
        chroma_format_idc: c, // 0=monochrome / 1=4:2:0 / 2=4:2:2 / 3=4:4:4 (仕様 7.4.2.1.1 全 Ok 値域)
        bit_depth_luma_minus8: l,
        bit_depth_chroma_minus8: c2,
    })
}

fn ok_sps_strategy() -> impl Strategy<Value = SpsBuildParams> {
    supported_profile_idc().prop_flat_map(|profile_idc| {
        let high_fields = if hisui::video::h264::H264_HIGH_PROFILES.contains(&profile_idc) {
            high_profile_fields_strategy().boxed()
        } else {
            // 非 High 系では SPS バイト列に書き込まれないため HighProfileFields::default() (全 0) で OK。
            // build_sps_for_pbt 側で profile_idc に応じた書き込み分岐を行う。
            Just(HighProfileFields::default()).boxed()
        };
        // raw_width / raw_height / pic_order_cnt_type / crop_offsets と組み合わせて
        // SpsBuildParams にマップする。Ok 経路は pic_order_cnt_type ∈ 0..=2、cropping は None または非過大値。
        ...
    })
}

// High 系プロファイル時の chroma_array_type を SpsBuildParams から算出する。
// 仕様 7.4.2.1.1: chroma_array_type = if separate_colour_plane_flag == 1 { 0 } else { chroma_format_idc }
// SpsBuildParams は separate_colour_plane_flag をパラメータ化しておらず、build_sps_for_pbt は
// chroma_format_idc=3 のとき separate_colour_plane_flag=0 を書き込む前提のため
// chroma_array_type == chroma_format_idc とみなせる。
// Baseline / Main / Extended (非 High 系) では parse_sps が chroma_format_idc 自体を SPS から
// 読み出さず chroma_array_type=1 (4:2:0 デフォルト、parse_sps line 263-269 参照) を使うため 1 を返す。
fn high_profile_chroma_array_type(params: &SpsBuildParams) -> u32 {
    if hisui::video::h264::H264_HIGH_PROFILES.contains(&params.profile_idc) {
        u32::from(params.chroma_format_idc)
    } else {
        1
    }
}

// cropping 適用後 width の期待値 (parse_sps の read_dimensions_with_cropping と同等ロジック)。
fn expected_cropped_width(params: &SpsBuildParams) -> u32 {
    let chroma_array_type = high_profile_chroma_array_type(params);
    let crop_unit_x = match chroma_array_type {
        0 | 3 => 1,
        1 | 2 => 2,
        _ => unreachable!("Strategy で 0..=3 のみ生成"),
    };
    let Some((l, r, _, _)) = params.frame_cropping else { return params.raw_width; };
    params.raw_width - (l + r) * crop_unit_x
}

// cropping 適用後 height の期待値。
// CropUnitY = (chroma_array_type=0 なら 2 - frame_mbs_only_flag、それ以外なら SubHeightC * (2 - frame_mbs_only_flag))。
// SubHeightC は chroma_array_type=1 で 2、=2 で 1、=3 で 1 (仕様 6.2 / 7.4.2.1.1)。
fn expected_cropped_height(params: &SpsBuildParams) -> u32 {
    let chroma_array_type = high_profile_chroma_array_type(params);
    let frame_mbs_factor = if params.frame_mbs_only_flag { 1 } else { 2 };
    let crop_unit_y = match chroma_array_type {
        0 => frame_mbs_factor,
        1 => 2 * frame_mbs_factor,
        2 | 3 => frame_mbs_factor,
        _ => unreachable!("Strategy で 0..=3 のみ生成"),
    };
    let Some((_, _, t, b)) = params.frame_cropping else { return params.raw_height; };
    params.raw_height - (t + b) * crop_unit_y
}
```

`SpsBuildParams` 自体は本体側で `#[derive(Debug, Clone, Copy)]` する。proptest の Strategy 値として渡すため `Send + Sync` も満たす (Copy 型のため自動)。

`HighProfileFields` には `#[derive(Default)]` を付け、Baseline / Main / Extended 経路では `Default::default()` (全 0) を使う。`build_sps_for_pbt` は `profile_idc` が `H264_HIGH_PROFILES.contains` を満たさないとき chroma_format_idc 以下を SPS バイト列に書き込まないため、デフォルト値の内容は影響しない。

`prop::sample::select(&H264_HIGH_PROFILES[..])` でスライス参照を渡すことで Vec の都度アロケートを避ける (proptest 1.x の `prop::sample::select` は `Slice<T>` を受ける形にも対応)。

`proptest_config` の `cases` 数は以下に確定:

| 関数群 | cases | 根拠 |
| --- | --- | --- |
| Ok 経路 (round-trip 系、5 関数想定) | 256 | 構造化生成で値域空間が狭まるため proptest デフォルト相当で十分 |
| Err 経路 (境界スイープ系、5-6 関数想定) | 128 | 値域内の境界 1 点だけ踏めば良いため Ok 経路の半分 |
| 全体合算目安 | ≤ 2048 cases | 現状 1024 cases × 1 関数の 2 倍以内に収める |

##### §1.3.3 PPS の扱い

PBT は `h264_sample_entry_from_sps_pps_lists` を呼ぶため PPS バイト列が必要。Strategy 化のスコープを SPS に絞り、PPS は最小 PPS NAL `[0x68, 0xce, 0x06, 0xe2]` (`src/video/h264.rs::tests` 内に `const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2];` として既存) を **PBT モジュール内に同名 const として保持** する固定値を使う。`pbt/tests/prop_h264_sps.rs` 先頭で `const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2];` と本体 const と同一名で定義する (本体 const の pub 化は不要、コピー関係であることが名前で明示される)。

PPS バイト列の意味: NAL ヘッダ `0x68` (forbidden_zero_bit=0, nal_ref_idc=3, nal_unit_type=8 = PPS) + payload `0xce 0x06 0xe2` (pic_parameter_set_id=0 / seq_parameter_set_id=0 / entropy_coding_mode_flag=0 等の最小 PPS RBSP)。`h264_sample_entry_from_sps_pps_lists` は `pps_list[i]` の先頭バイト `& 0x1F == 8` のみ検査するため、PPS payload の内容は本 PBT の解像度・avcC 検証には影響しない。Strategy で PPS バリエーションを生成する余地は本 issue のスコープ外。

##### §1.3.4 ファイル分割

PBT ファイル長が増えるなら `pbt/tests/prop_h264_sps.rs` 内で `mod ok_path { ... } mod err_path { ... }` のように in-file 分割する。各 mod 内で独立した `proptest!` ブロックを置き、Ok 経路 = 256 cases / Err 経路 = 128 cases の `proptest_config` を別々に指定する (§1.3.2 の表に整合)。ディレクトリ化 (`pbt/tests/prop_h264_sps/main.rs`) は採用しない (規模が見合わないため)。

##### §1.3.5 入力契約

- `h264_sample_entry_from_sps_pps_lists` の `sps_list[0]` は **NAL ヘッダ 1 バイト (`0x67`) 含む raw NAL バイト列**、`pps_list[0]` は **NAL ヘッダ 1 バイト (`0x68`) 含む raw NAL バイト列**。start code は含まない。引数 Vec は move 取得 (前述「入力 Vec の move 注意」参照)。
- `build_sps_for_pbt` は NAL ヘッダ込みのバイト列を返す。
- emulation prevention byte (`0x00 0x00 0x03`) を Strategy で意図的に挿入する経路は本 issue のスコープ外。fuzz/ 側のシード `seed_1920x1080.bin` (実機 `SPS_1920X1080` を含み、emulation prevention byte を 2 箇所含む) で `rbsp_from_sps_nalu` の縮約経路を初期コーパスからカバーする。

#### §1.4 extract_dimensions_from_sps の削除と既存テストの置き換え

本 issue 完了後、`extract_dimensions_from_sps` の呼び出し元は以下のとおり消える:

- 旧 PBT (`pbt/tests/prop_h264_sps.rs`): 構造化 Strategy 化で `h264_sample_entry_from_sps_pps_lists` 直接呼びに移行 → 消える。
- `src/video/h264.rs::tests` 内の既存単体テスト **13 件** (実コード grep `rg -nE '^\s*fn extract_dimensions_' src/video/h264.rs` で確認)。**11 件は `parse_sps` 直接呼びに置き換え、2 件 (テスト #4 / #12) は削除** する:

  | # | 既存関数名 | 改名後関数名 / 処理 |
  | --- | --- | --- |
  | 1 | `extract_dimensions_from_baseline_no_crop_320x240` | `parse_sps_from_baseline_no_crop_320x240` (置き換え) |
  | 2 | `extract_dimensions_from_baseline_with_crop_1920x1080` | `parse_sps_from_baseline_with_crop_1920x1080` (置き換え) |
  | 3 | `extract_dimensions_fails_on_truncated_sps` | `parse_sps_fails_on_truncated_sps` (置き換え) |
  | 4 | `extract_dimensions_from_sps_rejects_non_sps_nal` | **削除 + 新規追加で代替**。元意図 (実コードコメント「pub 関数として誤呼出時に release ビルドでも検出できることの回帰防止」) は `extract_dimensions_from_sps` 削除と同時に失われる。`parse_sps` は private fn のため型システム上外部誤呼出が不可能で、テストの存在意義が消える。NAL タイプ検査の検証は既存 `rbsp_from_sps_nalu_*` テスト群 (`rbsp_from_sps_nalu_removes_emulation_prevention_bytes` / `rbsp_from_sps_nalu_rejects_empty_input` の 2 件のみ、grep `rg -n 'fn rbsp_from_sps_nalu_' src/video/h264.rs` で確認済) には「非 SPS NAL → Err」検証が **欠落している** ため、本 issue で **`rbsp_from_sps_nalu_rejects_non_sps_nal` を新規追加** し、PPS NAL (`[0x68, 0xce, 0x06, 0xe2]` 等) を渡したとき `rbsp_from_sps_nalu` が Err を返すことを `rbsp_from_sps_nalu` 直接呼びで検証する |
  | 5 | `extract_dimensions_handles_high_profile_with_scaling_matrix` | `parse_sps_handles_high_profile_with_scaling_matrix` (置き換え) |
  | 6 | `extract_dimensions_handles_pic_order_cnt_type_1` | `parse_sps_handles_pic_order_cnt_type_1` (置き換え) |
  | 7 | `extract_dimensions_handles_pic_order_cnt_type_0` | `parse_sps_handles_pic_order_cnt_type_0` (置き換え) |
  | 8 | `extract_dimensions_handles_interlaced_frame_mbs_only_flag_zero` | `parse_sps_handles_interlaced_frame_mbs_only_flag_zero` (置き換え) |
  | 9 | `extract_dimensions_handles_frame_cropping_to_1080` | `parse_sps_handles_frame_cropping_to_1080` (置き換え) |
  | 10 | `extract_dimensions_rejects_crop_underflow` | `parse_sps_rejects_crop_underflow` (置き換え) |
  | 11 | `extract_dimensions_rejects_zero_dimensions_after_cropping` | `parse_sps_rejects_zero_dimensions_after_cropping` (置き換え) |
  | 12 | `extract_dimensions_does_not_panic_on_huge_pic_width` | **削除** |
  | 13 | `extract_dimensions_rejects_width_exceeding_u16_max` | `parse_sps_rejects_width_exceeding_u16_max` (置き換え) |

  置き換えは `let (width, height) = extract_dimensions_from_sps(&sps).expect(...)` → `let params = parse_sps(&sps).expect(...)` + `(params.width as usize, params.height as usize)` の形 (private fn の同一クレート内呼び出しは可)。`is_err()` 系も `parse_sps(&sps).is_err()` で同等。

  **テスト #12 (`extract_dimensions_does_not_panic_on_huge_pic_width`) は削除する** 理由: 本テストは `pic_width_in_mbs_minus1 = u32::MAX / 2` を渡し「panic しないこと」のみ assert する設計で、コメントにも「32 bit 環境ではオーバーフローで Err、64 bit 環境では巨大値で Ok になることがあるため、ここではパニックしないこと (Ok か Err のどちらかが返ること) だけ確認する」と明記されている。これは `shiguredo-rust` 規約「PBT (fuzzing) に任せるべき性質を単体テストで書かないこと」に反する形を残しており、本 issue で同時に PBT/fuzz でクラッシュフリーを担保する立て付けなので、本テスト固有の役割は完全に消える。`is_ok()` か `is_err()` のどちらかを単独 assert すると 32bit/64bit でクロスプラットフォーム CI が割れるため書き換えは不可。クラッシュフリー保証は §2 fuzz target で巨大値入力も含めて担保される。
- fuzz target: §2 のとおり `h264_sample_entry_from_sps_pps_lists` を呼ぶため `extract_dimensions_from_sps` には依存しない。
- `src/srt/inbound_endpoint.rs:1347` 付近: テスト関数 `srt_h264_sample_entry_and_size_reflect_sps_dimensions` の docstring コメント `// extract_dimensions_from_sps の出力が IDR ごとに正しく sample_entry と VideoFrame.size に流れることの回帰防止。` を以下に書き換える:

  ```
  // h264_sample_entry_from_sps_pps_lists 経由で SPS_INITIAL / SPS_UPDATED から解釈された解像度が
  // IDR ごとに sample_entry / VideoFrame.size に流れることの回帰防止。
  ```

`Don't live with broken windows` の観点で、呼び出し元ゼロの pub 関数を残さないため、本 issue で `pub fn extract_dimensions_from_sps` を削除する。

### §2 クラッシュフリー検証を cargo-fuzz に移管

`fuzz/fuzz_targets/h264_sample_entry_from_sps_pps_lists.rs` を新設し、現状の PBT が担保しているクラッシュフリー性質を本番入口関数の直接 fuzz に移す。

#### §2.1 fuzz target の対象関数と入力分割

本番入口 `h264_sample_entry_from_sps_pps_lists` を直接呼ぶ。`extract_dimensions_from_sps` (§1.4 で削除) や `parse_sps` (内部 fn のまま) は呼ばない。これにより「本番経路から外れた fuzz ターゲット」問題 (本 issue の主目的 1.) を fuzz 側でも回避する。

入力設計は **PPS を固定して SPS 側に fuzz 入力を集中** させる最小設計を採用する (PBT の §1.3.3 と同じ方針で、PPS バリエーションは本 issue のスコープ外)。

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

// 最小 PPS NAL (固定、本体 src/video/h264.rs::tests::PPS_NAL と同一バイト列)。
// h264_sample_entry_from_sps_pps_lists は pps_list[i] 先頭バイトの `& 0x1F == 8` のみ検査するため、
// PPS payload の内容はクラッシュフリー検証に影響しない。
const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2];

fuzz_target!(|data: &[u8]| {
    // 空入力 (data[0] への代入が panic する) のみ早期 return。
    // 1..=4 バイト入力でも parse_sps は Err を返して終了するため、
    // H264BitReader::read_u の buffer exhausted Err 経路の coverage 確保のため fuzz 対象に含める。
    if data.is_empty() {
        return;
    }

    // SPS 先頭バイトを 0x67 (forbidden_zero_bit=0, nal_ref_idc=3, nal_unit_type=7) に強制差し替え。
    // これで rbsp_from_sps_nalu の NAL タイプ検査 (`& 0x1F != H264_NALU_TYPE_SPS` → Err) を通過させ、
    // parse_sps 本体のビット読み出しパスを fuzz 対象にする。
    let mut sps = data.to_vec();
    sps[0] = 0x67;

    let _ = hisui::video::h264::h264_sample_entry_from_sps_pps_lists(
        vec![sps],
        vec![PPS_NAL.to_vec()],
    );
});
```

##### Arbitrary 採用検討経緯

`libfuzzer-sys` の `arbitrary-derive` feature 経由で `(Vec<u8>, Vec<u8>)` を直接 fuzz target 引数にする方式は、SPS / PPS の分離が明確になる利点があるが、本 issue では以下の理由で採用しない:

- 本 issue では PPS バリエーションを扱わない (§1.3.3 / §2.1 上記方針)。`(Vec<u8>, Vec<u8>)` 引数の PPS 側は実質固定で使わないため、`Arbitrary` 派生の利点が出ない。
- `libfuzzer-sys = { version = "0.4", features = ["arbitrary-derive"] }` の追加依存が発生する。最小依存 (CLAUDE.md「依存は最小限にすること」) を優先する。
- 上記の手書き設計で SPS 側に fuzz 入力を集中させれば、深いコードパス (`parse_sps` 本体の Exp-Golomb 経路 / `read_high_profile_sps_fields` / `skip_pic_order_cnt_type_extras` / `read_dimensions_with_cropping` / `rbsp_from_sps_nalu`) に到達できる。

#### §2.2 cargo-fuzz の toolchain 要件

`rust-toolchain.toml` (`channel = "stable"`) は本体ビルドに必要なため変更しない。fuzz 実行は nightly 必須なので:

- `fuzz/rust-toolchain.toml` に `channel = "nightly"` を別途配置する (cargo-fuzz の公式パターン)。`fuzz/` ディレクトリ内で rustup が自動で nightly に切り替わる (`cd fuzz && rustc --version` で `nightly` 表記が出ることを確認できる)。
- 実装者は事前に `rustup toolchain install nightly` を実行する。
- 実行コマンドは `cd fuzz && cargo fuzz run h264_sample_entry_from_sps_pps_lists -- -max_total_time=10` または ルート から `cargo +nightly fuzz run h264_sample_entry_from_sps_pps_lists -- -max_total_time=10`。

CI 組み込みは本 issue のスコープ外 (CI から fuzz を回す体制が現状無いため別途検討)。

#### §2.3 workspace 構成 と fuzz/Cargo.toml

`cargo fuzz init` の生成物 (`fuzz/Cargo.toml`) には空 `[workspace]` 宣言が含まれているバージョン (cargo-fuzz 0.12.x 以降) と含まれていないバージョンがあるため、生成直後に `fuzz/Cargo.toml` 先頭の `[workspace]` 空節の **有無を確認** し、無ければ追記する。これにより:

- ルートの `cargo build --workspace` / `cargo test --workspace` に `libfuzzer-sys` (nightly 必須) が混入しない。
- `cargo fuzz run` (fuzz/ 配下で実行) は独立 workspace として正常動作する。

ルート `Cargo.toml` の `[workspace] members` には `fuzz` を追加しない (members 経由で本体 workspace に巻き込まれることを避ける)。

`fuzz/Cargo.toml` の `[dependencies]` で `hisui = { path = ".." }` と直接 path 参照する (workspace 切り離しのため `workspace = true` 経由は使えない)。`libfuzzer-sys` のバージョンは `cargo fuzz init` のデフォルト出力 (執筆時点 `0.4`) を採用し、特定バージョン pin はしない。`fuzz/` クレートは workspace から切り離されているため `Cargo.lock` も独立。

#### §2.4 シードコーパス・辞書

- 初期コーパス: `fuzz/seeds/h264_sample_entry_from_sps_pps_lists/` ディレクトリにシードバイナリを配置する (corpus 自体は `.gitignore` 対象、シードは別ディレクトリで管理。§2.5 参照)。シードは:
  - `seed_320x240.bin`: 既存実機 SPS `SPS_320X240` (24 バイト) のバイト列。`parse_sps` の Baseline + 320x240 (crop なし) Ok 経路の coverage を提供。
  - `seed_1920x1080.bin`: 既存実機 SPS `SPS_1920X1080` (26 バイト) のバイト列。`parse_sps` の Baseline + 1920x1080 + crop_bottom Ok 経路 + emulation prevention byte 縮約経路 (`rbsp_from_sps_nalu`) の coverage を提供 (`SPS_1920X1080` は emulation prevention byte を 2 箇所含むことが既存テスト `rbsp_from_sps_nalu_removes_emulation_prevention_bytes` で確認済)。
- シード生成手段: `SPS_320X240` / `SPS_1920X1080` は `pub(crate) const` で `src/video/h264.rs::tests` 内 (cfg test 配下) のため、`fuzz/` クレートから直接 use できない。手順は以下のとおり (PR レビューで検証可能な形で実施):
  - `src/video/h264.rs::tests::SPS_320X240` / `SPS_1920X1080` のバイト列リテラルを目視で読み取り、`printf '%b' '\x67\x42\xc0\x0d...' > fuzz/seeds/h264_sample_entry_from_sps_pps_lists/seed_320x240.bin` で書き出して git にコミット。
  - 検証手段: PR で `xxd fuzz/seeds/h264_sample_entry_from_sps_pps_lists/seed_320x240.bin` の出力が `src/video/h264.rs::tests::SPS_320X240` のバイト列リテラルと一致することを diff として PR 説明欄に添付する。`seed_1920x1080.bin` も同様。
- 初回 fuzz 実行手順: `fuzz/` 配下に簡易シェルスクリプト `fuzz/run.sh` を追加し、シードの corpus へのコピー + fuzz run を 1 コマンドで実行できるようにする:

  ```sh
  #!/bin/sh
  # Usage: fuzz/run.sh <target-name> [-- <libfuzzer args>]
  set -eu
  target="$1"
  shift
  mkdir -p "corpus/$target"
  cp -n "seeds/$target/"* "corpus/$target/" 2>/dev/null || true
  cargo fuzz run "$target" "$@"
  ```

  実行例: `cd fuzz && ./run.sh h264_sample_entry_from_sps_pps_lists -- -max_total_time=10`。
- 辞書ファイル: 本 issue のスコープ外 (libFuzzer 辞書は emulation prevention byte パターン等が候補だが、シードコーパスで十分な探索開始点が確保できるため省略)。
- `-max_len`: libFuzzer デフォルト 4096 (cargo-fuzz 経由) のままとし `-max_len=` 明示指定はしない。現状 PBT の `0..=4096` バイトと一致する。

#### §2.5 `.gitignore` / `fuzz/` 配下管理

`fuzz/.gitignore` で `corpus/` / `artifacts/` / `coverage/` を全体 ignore し、シードは別ディレクトリ `fuzz/seeds/<target>/` でバージョン管理する (cargo-fuzz コミュニティで推奨される運用パターン)。これにより libFuzzer が `corpus/<target>/` に SHA-1 ハッシュ名で書き込む自動生成入力ファイルとシードが衝突しない。

`fuzz/.gitignore` 内容:

```
target/
corpus/
artifacts/
coverage/
```

`coverage/` は本 issue で `cargo fuzz coverage` を使わないが、将来実行された場合の untracked ファイル流入を予防的に ignore する。

#### §2.6 fuzz target 命名規則

本リポジトリには既存 fuzz target がない (本 issue で初導入)。本 issue では fuzz target 名を **対象本番関数名と同一** に揃える方針を採り、`h264_sample_entry_from_sps_pps_lists` とする。

短縮案 (`h264_sps_pps` 等) を採らない理由: 本 issue で初例を確立する以上、明示的な対応関係 (fuzz target ⇔ 本番関数) を優先する。target 名の長さは `cargo fuzz run` コマンドが長くなる程度の問題で、shell 補完および §2.4 の `fuzz/run.sh` で十分カバーできる。将来 fuzz target が増えて命名規則の再評価が必要になったら別途検討する (本 issue 完了で「対象本番関数名と同一」が事実上の前例として残る)。

`shiguredo-rust` スキル側の fuzz 命名規約追記は本 issue 完了後に手動で行う (Hisui 本 issue から `shiguredo/llm-feedback` のスキル更新を指示するのは越権)。

### スコープ外

- 既存単体テスト (`src/video/h264.rs::tests` の `parse_sps_*` テスト群、`h264_sample_entry_from_sps_pps_lists_*` テスト群) はそのまま維持する。境界値の代表値テストは PBT で代替せず単体テストで残す。
- `H264_HIGH_PROFILES` 以外の `parse_sps` 内部実装の pub 化はしない (`parse_sps` / `SpsParams` / `HighProfileSpsParams` は非 pub を維持)。
- emulation prevention byte の意図的挿入を Strategy で扱わない (fuzz 側でシード `seed_1920x1080.bin` 経由でカバー)。
- fuzz 辞書 / `-max_len` 詳細チューニング / `cargo fuzz coverage` / CI 組み込みは本 issue 外。
- `pbt/Cargo.toml` の `proptest` バージョン更新は本 issue 外 (既存 `proptest = "1.11"` をそのまま使う)。
- `pbt/tests/prop_h264_sps.rs` のファイル名は `shiguredo-rust` 規約「PBT のファイル名は `pbt/tests/prop_<module>.rs` とし `src/<module>.rs` に対応させること」と厳密一致しない (`src/video/h264.rs` 対応のため `pbt/tests/prop_video/h264.rs` のディレクトリモジュール対応が規約準拠) が、本 issue では現状ファイル名を維持する。ファイル名リネームは別 issue で扱う。
- 将来別 issue で `h264_sample_entry_from_sps_pps_lists` 経路の **より高粒度な構造化 PBT** (複数 SPS / 複数 PPS / NAL タイプ越境 等) を追加する余地はあるが、本 issue の Ok / Err 経路カバレッジで主要不変条件は担保される想定。

### モック/スタブ規約との整合

`build_sps_for_pbt` で生成する SPS バイト列は ITU-T H.264 仕様 7.3.2.1.1 / 7.4.2.1.1 に基づく「実 SPS の仕様準拠合成」であり、CLAUDE.md / `shiguredo-rust` の「モックやスタブを使わないこと」のモック (実装の振る舞いを偽装する代用品) には該当しない (closed 0037 で同様の判断を確立済)。Strategy で生成される値域外入力 (例: `chroma_format_idc=4`) も「仕様外バイト列を本番入口に投入する Err 経路テスト」であり、H.264 仕様の値域定義そのものに基づくため同様にスタブではない。

### ソースコードへの issue 番号・自リファレンス禁止

PBT 関数名・assertion メッセージ・テストコメント・docstring・fuzz target ファイル等に issue 番号 (`0043` / `0044` / `0049` 等) や issue への言及を一切持ち込まない (`shiguredo-issues` の「issue 番号・issue への言及をソースコードに持ち込まないこと」)。「§N の不変条件」のような issue 自リファレンスも避け、仕様参照のみで記述する (例: 「ITU-T H.264 仕様 7.4.2.1.1 の値域外」)。本 issue 本文内での節参照 (例: 「§1.3.1 の責務分担」) は issue 内部参照のため許容するが、ソースコードに移植する際に削除する。

### テストログ日本語化規約

PBT 内の `prop_assert!` / `prop_assert_eq!` の失敗メッセージは AGENTS.md「テストのログメッセージは全て日本語にすること」に従い日本語で書く。例:

```rust
prop_assert_eq!(
    avcc_box.avc_profile_indication,
    params.profile_idc,
    "avc_profile_indication が SPS の profile_idc と一致しない: avcC={}, SPS={}",
    avcc_box.avc_profile_indication,
    params.profile_idc
);
```

proptest フレームワーク自身の出力 (`proptest: assertion failed`, `cc <hex>`, `shrunk to`) は英語固定で、`AGENTS.md` 規約と整合しない部分が残るが、フレームワーク内部メッセージは規約対象外として扱う。本 issue では `prop_assert!` / `prop_assert_eq!` のユーザー記述メッセージ部分のみ日本語化する。

## 完了条件

### 機能・コード変更

- `src/video/h264.rs` に PBT 用 pub fn `build_sps_for_pbt(SpsBuildParams) -> Vec<u8>` および pub struct `SpsBuildParams` (`#[derive(Debug, Clone, Copy)]`) が追加されている。実装は §1.1 案 D-(b) のとおり cfg なし領域に SPS ビット組み立てロジックを独立実装する。
- `src/video/h264.rs::tests` 内に乖離検出単体テスト 3 件 (`sps_builder_and_build_sps_for_pbt_emit_byte_compatible_sps_baseline` / `_main` / `_high10`) が追加され、`SpsBuilder` と `build_sps_for_pbt` が Baseline / Main / High10 の各経路で同等パラメータから同一バイト列を生成することを assert する (High 経路の乖離も検出するため複数経路を網羅)。
- `src/video/h264.rs::H264_HIGH_PROFILES` が `pub const` に昇格し、`///` docstring 形式で ITU-T H.264 (2017/06) 仕様 7.3.2.1.1 由来であることが明記されている (本体コメント慣習に合わせて半角括弧で統一)。
- `pub fn extract_dimensions_from_sps` が削除されている。
- `src/video/h264.rs::tests` 内の既存 `extract_dimensions_*` 系単体テスト **13 件** のうち、**11 件が §1.4 改名後関数名対応表のとおり `parse_sps` 直接呼びに置き換わり、引き続き pass する**。テスト #4 (`extract_dimensions_from_sps_rejects_non_sps_nal`) と #12 (`extract_dimensions_does_not_panic_on_huge_pic_width`) は **削除されている**。テスト #4 が担っていた「非 SPS NAL → Err」検証の代替として、`rbsp_from_sps_nalu_rejects_non_sps_nal` を新規追加 (PPS NAL バイト列を `rbsp_from_sps_nalu` に直接渡して Err 返却を assert) し、`rbsp_from_sps_nalu` 直接呼びで NAL タイプ検査の回帰防止を担保している。
- `src/srt/inbound_endpoint.rs:1347` 付近の `srt_h264_sample_entry_and_size_reflect_sps_dimensions` テスト関数の docstring コメントが §1.4 のとおり書き換わっている (関数名 `h264_sample_entry_from_sps_pps_lists` 経由を明示)。

### PBT (§1)

- `pbt/tests/prop_h264_sps.rs` の冒頭 docstring が「構造化 Strategy で生成した正当な SPS バイト列を `h264_sample_entry_from_sps_pps_lists` に投入し、戻り値の `Avc1Box` / `VideoFrameSize` の不変条件を検証する」旨に書き換わっている。
- `pbt/tests/prop_h264_sps.rs` 内に Strategy ヘルパー関数 `supported_profile_idc()` / `raw_width_strategy()` / `raw_height_strategy()` / `high_profile_fields_strategy()` / `ok_sps_strategy()` および Err 経路用 Strategy ヘルパー (例: `unsupported_profile_idc_strategy()`) が §1.3.2 擬似コードに沿って実装されている。これらを各 `proptest!` ブロック内に inline 展開せず、再利用可能な関数として抽出することで Ok 経路 PBT 群と Err 経路 PBT 群の重複を削減する。
- `pbt/tests/prop_h264_sps.rs` 内に PPS バイト列 const `const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2];` が定義されている (本体 const と同一名・同一バイト列)。
- Ok 経路 PBT 関数 (`prop_h264_sample_entry_round_trips_profile_indication` / `prop_h264_sample_entry_reflects_high_profile_chroma_format` / `prop_h264_sample_entry_reflects_cropping_in_visual_and_frame_size` 等) が §1.3 Ok 経路不変条件表をカバーしている。各関数は内部で `hisui::video::h264::build_sps_for_pbt(params)` を呼んで SPS バイト列を生成し、その結果を `h264_sample_entry_from_sps_pps_lists(vec![sps.clone()], vec![PPS_NAL.to_vec()])` に投入する。
- Err 経路 PBT 関数 (`prop_h264_sample_entry_rejects_unsupported_profile_idc` 等) が §1.3 Err 経路不変条件表をカバーし、§1.3.1 の責務分担方針 (値域全体スイープによる境界検証、メッセージ文字列は検証しない) に従っている。
- `proptest_config` の `cases` 数が §1.3.2 の表 (Ok 経路 256 / Err 経路 128) に従い設定されている。
- PBT 関数名は `prop_h264_sample_entry_*` 接頭辞で `parse_sps_*` (既存単体テスト) と区別されている。
- `prop_assert!` / `prop_assert_eq!` の失敗メッセージは日本語で記述されている。
- PBT 内・assertion メッセージ・コメントに issue 番号 / issue への言及 / 「§N」自リファレンスがない (仕様番号での参照に統一)。
- ファイル長が増えた場合は `mod ok_path` / `mod err_path` で in-file 分割している (§1.3.4)。

### fuzz (§2)

- `fuzz/Cargo.toml` が新規追加され、`cargo fuzz init` 直後に `[workspace]` 空節の有無を確認し無ければ追記している。`[dependencies]` で `hisui = { path = ".." }` と path 参照している。
- `fuzz/rust-toolchain.toml` に `channel = "nightly"` が指定されている。
- `fuzz/fuzz_targets/h264_sample_entry_from_sps_pps_lists.rs` が新規追加され、§2.1 の入力設計 (SPS 先頭バイト 0x67 強制 + PPS 固定 `PPS_NAL`) で本番入口を呼ぶ実装になっている (空入力のみ早期 return)。
- `fuzz/seeds/h264_sample_entry_from_sps_pps_lists/seed_320x240.bin` / `seed_1920x1080.bin` が配置され git 管理されている。PR 説明欄に `xxd <seed-file>` の出力と `src/video/h264.rs::tests::SPS_320X240` / `SPS_1920X1080` のバイト列リテラルとの diff (= 差分なし) が添付されている。
- `fuzz/.gitignore` で `target/` / `corpus/` / `artifacts/` / `coverage/` が ignore されている。
- `fuzz/run.sh` が新規追加され、`./fuzz/run.sh <target-name>` でシード copy + fuzz run が 1 コマンドで動く。実行ビット (`chmod +x`) も立っている。
- ルート `Cargo.toml` の `[workspace] members` に `fuzz/` を追加していない (本体ビルドへの混入回避)。
- `cargo build --workspace` が `fuzz/` を巻き込まずに pass することを確認している。
- `cd fuzz && ./run.sh h264_sample_entry_from_sps_pps_lists -- -max_total_time=10` を実行し、クラッシュなく終了することを確認している (nightly 環境のローカル確認)。

### CI / 品質チェック

- 以下が pass する:
  - `cargo test --workspace` (本体クレート + pbt クレート両方の全テスト)
  - `cargo test -p pbt` (PBT 単独確認)
  - `cargo clippy --all-targets --workspace -- --deny warnings` (fuzz/ は別 workspace で除外される)
  - `cargo fmt --all -- --check`
- fuzz/ クレートに対しては PR 提出前に `cd fuzz && cargo clippy --all-targets -- --deny warnings` を 1 回確認する (CI 必須ではない。`fuzz/rust-toolchain.toml` で nightly が効くため `+nightly` 指定は不要)。

### CHANGES.md

記載しない。

- 利用者挙動への影響なし (本 issue の変更は `pbt/` / `fuzz/` 配下と `src/video/h264.rs` の API 整理のみで、公開機能・出力 MP4・ログには影響しない)。
- `H264_HIGH_PROFILES` の pub 化と `build_sps_for_pbt` / `SpsBuildParams` の新規 pub 化はテスト戦略上の API 追加で、利用者向け機能ではない (`build_sps_for_pbt` の docstring で PBT 専用 API である旨を明示することで誤用を抑制)。Hisui を library として使う外部ユーザーの semver 上は pub fn / pub const 追加で `[ADD]` 相当だが、PBT 専用 API のため CHANGES.md 非記載。
- `extract_dimensions_from_sps` の削除は利用者から見て呼び出し元ゼロの pub 関数の整理であり、Hisui の公開機能には影響しない。
- closed 0037 / 0043 / 0044 の「`## develop` 内中間状態の修正は CHANGES.md に書かない」方針と整合。

## 関連

- issue 0037 (closed: `feature-add-h264-sps-dimensions-parser`): `extract_dimensions_from_sps` および付随する Exp-Golomb パーサを新規導入。
- issue 0043 (closed: `feature-refactor-h264-sample-entry-from-sps-pps-lists`): `h264_sample_entry_from_annexb` を `h264_sample_entry_from_sps_pps_lists → parse_sps` 経由にリファクタ。`extract_dimensions_from_sps` を「PBT 専用 pub API として残置」とした closed 判断と、`SpsParams` / `HighProfileSpsParams` を非 pub のまま保持した closed 判断を本 issue が踏襲・補完する。本 issue は 0043 の「## 残懸念」リストから派生したテスト戦略整理を扱う。
- issue 0044 (closed: `feature-refactor-reject-invalid-pic-order-cnt-type`): `parse_sps` の `pic_order_cnt_type` 仕様外値 (0/1/2 以外) を Err 化。0044 で導入された `SpsBuilder::with_pic_order_cnt_type(value: u32)` を本 issue の `SpsBuildParams` Strategy から参照する形は採らない (案 D-(b) で `build_sps_for_pbt` は SpsBuilder を呼ばないため)。Err 経路 (`pic_order_cnt_type ∈ 3..`) は単体テストで維持し、本 issue の PBT は境界全体スイープで補強する役割分担とする。
- issue 0048 (open: `feature-refactor-h265-sample-entry-from-vps-sps-pps-lists`): H.265 経路の sample_entry リファクタ。本 issue の構造化 Strategy 方針は H.265 SPS パーサ (0048 で新規実装) の PBT 構造化にも適用できるため、将来 `pbt/tests/prop_h265.rs` 等を追加する際の参考にする。
- issue 0050 (open: `feature-refactor-rtmp-avc-sequence-header-from-sps-pps-lists`): RTMP 経路の H.264 sample_entry 統合。本 issue の Ok 経路 PBT は `h264_sample_entry_from_sps_pps_lists` 経路を直接検証するため、0050 経由の RTMP 入力経路も同 PBT で間接的にカバーされる。

## 解決方法

実装着手後にここに記述する。
