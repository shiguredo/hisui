# prop_h264_sps の PBT を構造化 Strategy に置き換えてクラッシュフリー専用テストを fuzz/ に移管する

- Priority: Low
- Created: 2026-06-19
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-prop-h264-sps-structured-strategy
- Polished: {YYYY-MM-DD}

## 目的

`pbt/tests/prop_h264_sps.rs` は次の 2 つの問題を抱えている。

1. **本番経路から外れた PBT ターゲット**: issue 0043 (closed) で `h264_sample_entry_from_annexb` は `h264_sample_entry_from_sps_pps_lists → parse_sps` 経由に変更され、本番経路から `extract_dimensions_from_sps` の呼び出しは消えた。PBT は依然として `extract_dimensions_from_sps` (PBT 専用 pub API として残置されている薄いラッパー) を呼んでおり、論理的には本番経路から外れたターゲットになっている。
2. **クラッシュフリー専用テスト規約違反**: 現状の PBT は「任意入力でパニックや無限ループを起こさず Ok か Err を返すこと」のみを検証し、戻り値を `let _ = ...` で捨てている。これは `shiguredo-rust` 規約「PBT に『任意入力でパニックしないことだけを検証するテスト』を書かないこと（fuzzing の役割）」に反する。

本 issue では PBT を構造化 Strategy (`SpsBuilder` 風) で SPS を生成し、`parse_sps` の出力に対する仕様準拠の不変条件を検証するプロパティに置き換え、クラッシュフリー検証は cargo-fuzz のターゲットに移管する。

## 優先度根拠

Low。テスト戦略の規約準拠が主目的で、バグや機能面の問題は発生していない。

- 現状の PBT もクラッシュフリー性質自体は担保しているため、本番品質に影響しない。
- ただし issue 0043 で本番経路が変わった以上、PBT のターゲット選択を見直す機会として扱う。

## 現状

### `pbt/tests/prop_h264_sps.rs`

```rust
use hisui::video::h264::extract_dimensions_from_sps;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, .. ProptestConfig::default() })]

    #[test]
    fn extract_dimensions_from_sps_does_not_panic(payload in prop::collection::vec(any::<u8>(), 0..=4096)) {
        // 先頭バイトは SPS NAL ヘッダ固定
        let mut sps = Vec::with_capacity(payload.len() + 1);
        sps.push(0x67);
        sps.extend_from_slice(&payload);
        // パース結果は問わない (Ok でも Err でもよい)。重要なのは panic / 無限ループしないこと。
        let _ = extract_dimensions_from_sps(&sps);
    }
}
```

- 任意バイト列 (0..=4096 バイト) の先頭に NAL ヘッダ 0x67 を付けて投入。
- 結果を捨ててクラッシュフリーのみ検証 (proptest 1024 cases)。

### 本番呼び出し側

issue 0043 (closed) 後の状況を実装着手時に再確認する。`extract_dimensions_from_sps` は PBT 専用 pub API として残置されている想定 (本 issue 着手時点で本番呼び出しがゼロであることを grep で再確認)。

### fuzz/ ディレクトリ

現状リポジトリに `fuzz/` ディレクトリは存在しない。`cargo-fuzz` の初期化 (`cargo fuzz init`) と fuzz ターゲット用の `fuzz/Cargo.toml` 追加が必要。

## 設計方針

### §1 PBT の構造化 Strategy 化

`pbt/tests/prop_h264_sps.rs` を構造化 Strategy で SPS を生成し、`parse_sps` 出力に対する不変条件を検証する形に書き換える。

- `src/video/h264.rs::tests::SpsBuilder` は `pub(crate)` 化されているが、PBT ファイルは別クレート (`pbt/`) からの参照のため `pub` (またはテスト用エクスポート) が必要。`SpsBuilder` を `pub` 化するか、PBT 用の生成ヘルパー関数を `src/video/h264.rs` に切り出す。
- Strategy: `profile_idc` / `chroma_format_idc` / `bit_depth_*_minus8` / `raw_width` / `raw_height` / `pic_order_cnt_type` 等を proptest の `prop::sample` / `prop::range` で構造化生成し、`SpsBuilder` で組み立てる。
- 検証する不変条件 (Ok 経路):
  - `params.width <= u16::MAX` かつ `params.width > 0`
  - `params.height <= u16::MAX` かつ `params.height > 0`
  - `params.profile_idc` が `{66, 77, 88} ∪ H264_HIGH_PROFILES` のいずれかに含まれる
  - `params.high_profile_params.is_some()` ↔ `H264_HIGH_PROFILES.contains(profile_idc)`
  - High 系プロファイル時に `chroma_format_idc <= 3` / `bit_depth_*_minus8 <= 6`
- 検証する不変条件 (Err 経路): 値域外入力を Strategy で生成し `parse_sps` が Err を返すこと (例: `chroma_format_idc=4` で必ず Err)。

### §2 クラッシュフリー検証を cargo-fuzz に移管

`fuzz/fuzz_targets/h264_parse_sps.rs` を新設し、現状の PBT が担保しているクラッシュフリー性質をここに移す。

- `cargo-fuzz` 初期化と `fuzz/Cargo.toml` 整備。
- `fuzz_target!(|data: &[u8]| { let _ = hisui::video::h264::extract_dimensions_from_sps(data); });` 程度のシンプルな fuzz ターゲット。
- CI への組み込みは本 issue のスコープ外 (CI から fuzz を回す体制が現状無いため別途検討)。

### §3 `h264_sample_entry_from_sps_pps_lists` 経路の PBT 追加

issue 0043 で新設した `h264_sample_entry_from_sps_pps_lists` は外部入力経路 (SRT inbound / RTSP) から呼ばれる。同関数に対する構造化 PBT (SPS / PPS リストを生成して `AvccBox` のマッピングを検証) を追加するかは、§1 完了後に判断する。本 issue のスコープには含めず、必要なら別 issue。

### スコープ外

- 既存単体テスト (`src/video/h264.rs::tests` の `parse_sps_*` テスト群) はそのまま維持する。境界値の意図的なテストとして PBT で代替できないケースをカバーしているため。
- `extract_dimensions_from_sps` を pub のまま残すか削除するかは本 issue のスコープ外 (issue 0043 では PBT 専用 API として残置する判断)。本 issue 完了後に PBT が `parse_sps` を直接呼ぶ形になれば、`extract_dimensions_from_sps` の削除可否を別 issue で再検討する。

## 完了条件

- `pbt/tests/prop_h264_sps.rs` の PBT が構造化 Strategy で SPS を生成し、`parse_sps` 出力に対する不変条件を検証する形に書き換わっている。
- 検証する不変条件 (Ok 経路 / Err 経路) が複数ケース実装されている。
- `fuzz/` ディレクトリと `fuzz/Cargo.toml`、`fuzz/fuzz_targets/h264_parse_sps.rs` が新規追加されている。
- `cargo fuzz run h264_parse_sps -- -max_total_time=10` 等で短時間動作することを確認する (CI 組み込みはスコープ外)。
- `cargo test -p pbt` で新 PBT が pass する。
- 既存テストへの影響がない (`cargo test` で全 pass)。

### CHANGES.md

記載しない (テスト戦略のリファクタで利用者挙動変化なし)。

## 関連

- issue 0043 (closed): `h264_sample_entry_from_annexb` を SPS / PPS リスト受け取り版にリファクタ。本 issue の前提となる本番経路変更。
- issue 0044 (open): `extract_dimensions_from_sps` の `pic_order_cnt_type` 仕様外値 Err 化。本 issue で構造化 Strategy を導入する際、0044 の Err 経路もカバーする。
- 将来別 issue: `h264_sample_entry_from_sps_pps_lists` 経路の構造化 PBT 追加。

## 解決方法

実装着手後にここに記述する。
