# parse_sps の pic_order_cnt_type 仕様外値 (0/1/2 以外) を Err 化して仕様準拠の堅牢性を補強する

- Priority: Low
- Created: 2026-06-18
- Completed: 2026-06-22
- Model: Opus 4.7
- Branch: feature/refactor-reject-invalid-pic-order-cnt-type
- Polished: 2026-06-22

## 目的

issue 0037（closed）で導入された `src/video/h264.rs` の `skip_pic_order_cnt_type_extras` 関数は、`pic_order_cnt_type` の値が 0 / 1 / 2 のいずれかであることを ITU-T H.264 仕様 7.4.2.1.1 で要求されているにもかかわらず、それ以外の値を黙って通している（`_ => {}` で何もせず先に進む）。本 issue では仕様外値を `Err` として早期に弾くことで、不正 / 破損 SPS に対する内部堅牢性を補強する。

`skip_pic_order_cnt_type_extras` は `parse_sps` → 本番経路 `h264_sample_entry_from_sps_pps_lists` から呼ばれているため、本 issue の Err 化は本番経路の堅牢性補強となる。

本 issue は `## develop` 内中間状態の堅牢性補強で利用者挙動に影響しないため CHANGES.md は更新しない（closed 0030 / 0031 / 0032 / 0033 / 0034 / 0037 / 0043 と同方針で、`## CHANGES.md` 節自体を立てない）。

## 優先度根拠

Low。

- 仕様準拠の publisher（libx264 / ハードウェアエンコーダ等）では `pic_order_cnt_type` が 0 / 1 / 2 以外の値を出すことは無いため、実運用での発生はほぼ無い。
- 仕様外値が来た場合の現挙動は「後続フィールドの読み出しを続行 → 誤った解像度を返すか、別の Err で弾かれる」で、最悪のシナリオでも誤解像度の MP4 sample_entry が生成される程度。クラッシュや無限ループにはならない（`pbt/tests/prop_h264_sps.rs` のクラッシュフリー PBT で確認済み）。
- 仕様外値は publisher のバグ・伝送破損・中間プロキシでのビット位置ズレ起因で発生し得る。後者は `skip_pic_order_cnt_type_extras` より前の Exp-Golomb 読み出しが 1 ビットでもずれると以降の ue(v) が壊れて巨大値や仕様外値を返すケースで、Err 化はこの「位置ズレ後の暴走を早期検出する」役割も持つ。
- 既存の `parse_sps` 経路は `read_high_profile_sps_fields` 内で `chroma_format_idc > 3` / `bit_depth_luma_minus8 > 6` / `bit_depth_chroma_minus8 > 6` の仕様値域検査を導入済みで、`skip_pic_order_cnt_type_extras` 内でも `num_ref_frames_in_pic_order_cnt_cycle > 255` の仕様上限検査を導入済み。本 issue は「`parse_sps` 経路が読む仕様値域・enum 型値はすべて読み出し直後に検査する」一律ポリシーを `pic_order_cnt_type` にも適用する補強。

## 現状

行番号は実装着手時に `skip_pic_order_cnt_type_extras` の関数名で grep して再特定する。本文では関数名で参照する。

### 該当箇所

`src/video/h264.rs` の `skip_pic_order_cnt_type_extras` 関数:

```rust
fn skip_pic_order_cnt_type_extras(reader: &mut H264BitReader<'_>) -> crate::Result<()> {
    let pic_order_cnt_type = reader.read_ue()?;
    match pic_order_cnt_type {
        0 => {
            reader.skip_ue()?; // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            reader.skip_u(1)?; // delta_pic_order_always_zero_flag
            reader.skip_se()?; // offset_for_non_ref_pic
            reader.skip_se()?; // offset_for_top_to_bottom_field
            let num_ref_frames_in_pic_order_cnt_cycle = reader.read_ue()?;
            // 仕様 7.4.2.1.1 で 0..=255 の範囲。それを超える値は仕様外で、巨大値での無駄な se(v) ループを防ぐ。
            if num_ref_frames_in_pic_order_cnt_cycle > 255 {
                return Err(crate::Error::new(format!(
                    "invalid H.264 SPS: num_ref_frames_in_pic_order_cnt_cycle exceeds 255 ({num_ref_frames_in_pic_order_cnt_cycle})"
                )));
            }
            for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
                reader.skip_se()?; // offset_for_ref_frame[i]
            }
        }
        // pic_order_cnt_type == 2 のときは追加読み出しなし
        _ => {}
    }
    Ok(())
}
```

`_ => {}` で 3 以上の値を素通ししている。

### 仕様

ITU-T H.264 仕様 7.4.2.1.1 で `pic_order_cnt_type` は 0 / 1 / 2 のいずれかと規定。

### 既存テスト

`src/video/h264.rs` の `#[cfg(test)] mod tests` に `SpsBuilder` がある。

- `pic_order_cnt_type` のデフォルト値は 2。フィールド型は `pic_order_cnt_type: u32`。
- `with_pic_order_cnt_type_1()`（引数なしのトグル）が `pic_order_cnt_type = 1` に切り替える形で残っている。SPS の単一値フィールド setter のうち `with_pic_order_cnt_type_1` だけが「特定値固定トグル」で、他の単一値フィールド（`with_profile_idc(profile_idc: u32)` / `with_constraint_set_flags(flags: u32)` / `with_chroma_format_idc(value: u32)` / `with_bit_depth_luma_minus8(value: u32)` / `with_bit_depth_chroma_minus8(value: u32)` / `with_pic_width_in_mbs_minus1(value: u32)`）は `u32` 引数版に統一されている。引数名は setter ごとに `value` か field 名そのものを使い分けている。
- `with_high_profile_and_scaling_matrix()`（引数なしの経路トグル）/ `with_interlaced(raw_height_field: u32)`（経路指定 + パラメータ）/ `with_cropping(left, right, top, bottom: u32)`（複合引数）は単一値 setter ではなく、本 issue の対象外。
- 本 issue 着手時点で既存の値域外 Err テストは `parse_sps_rejects_unsupported_profile_idc` / `parse_sps_rejects_chroma_format_idc_out_of_range` / `parse_sps_rejects_bit_depth_luma_minus8_out_of_range` / `parse_sps_rejects_bit_depth_chroma_minus8_out_of_range` の 4 件で、すべて `parse_sps(&sps)` を直接呼んで `assert!(result.is_err(), "...: {result:?}")` 形式でアサートする。Err メッセージ文言の `format!("{err:?}").contains(...)` 検証は行わない。
- 別系統の `extract_dimensions_rejects_*` 系（`extract_dimensions_rejects_crop_underflow` / `extract_dimensions_rejects_zero_dimensions_after_cropping` / `extract_dimensions_rejects_width_exceeding_u16_max` の 3 件）は `extract_dimensions_from_sps` 経由のテストで、本 issue では参照しない。
- 既存 Err メッセージは `"invalid H.264 SPS: <field> out of spec range (0..=N): {value}"` 形式で統一されている。

## 設計方針

### Err 化

`skip_pic_order_cnt_type_extras` 内で、`reader.read_ue()` 直後に `pic_order_cnt_type > 2` を弾く `if` 検査を追加する（既存 `read_high_profile_sps_fields` 内の `chroma_format_idc > 3` 検査と同じ「読み出し直後に値域検査して早期 return」スタイル）。

```rust
fn skip_pic_order_cnt_type_extras(reader: &mut H264BitReader<'_>) -> crate::Result<()> {
    let pic_order_cnt_type = reader.read_ue()?;
    if pic_order_cnt_type > 2 {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: pic_order_cnt_type out of spec range (0..=2): {pic_order_cnt_type}"
        )));
    }
    match pic_order_cnt_type {
        0 => { /* § 該当箇所のコードブロックの 0 アームをそのまま維持 */ }
        1 => { /* § 該当箇所のコードブロックの 1 アームをそのまま維持 */ }
        // pic_order_cnt_type == 2 のときは追加読み出しなし
        _ => {}
    }
    Ok(())
}
```

ポイント:

- 値検査位置は `match` の前に置く（既存 `chroma_format_idc > 3` / `bit_depth_*_minus8 > 6` と同じスタイル）。`match` の `_` アームに `Err` を埋め込む案は採らない（既存パターンと不揃いになる）。
- Err メッセージ表現は既存 `parse_sps` 経路の値域 Err 群と同じ `"<field> out of spec range (0..=N): {value}"` 形式で統一する。
- `match` 末尾の `_ => {}` は事前検査により `pic_order_cnt_type == 2` のときのみ通る。コメント「pic_order_cnt_type == 2 のときは追加読み出しなし」はそのまま残す。

### 公開 API への影響

本 issue の変更は `skip_pic_order_cnt_type_extras` の内部実装のみで、`parse_sps` / `extract_dimensions_from_sps` / `h264_sample_entry_from_sps_pps_lists` の公開 API シグネチャは変更しない。仕様準拠 publisher が生成する SPS は `pic_order_cnt_type ∈ {0, 1, 2}` のため、外部から観測可能な挙動変化は仕様外 SPS 入力時の Err 化のみ。

### テスト

`SpsBuilder` を以下の通り改修する:

- `with_pic_order_cnt_type(value: u32)` を新設し、既存 `with_pic_order_cnt_type_1()` を削除する。実装は `self.pic_order_cnt_type = value;` のみ（`SpsBuilder.pic_order_cnt_type` フィールドが既に `u32` のため `as u8` のような丸めは不要）。
- 引数型 `u32` の動機は次の 3 点: (a) `SpsBuilder.pic_order_cnt_type` 内部フィールド型 `u32` と直接対応する、(b) `SpsBuilder::build()` 内の `w.write_ue(self.pic_order_cnt_type)` の引数型 `u32` と整合する、(c) 仕様外値 (3 以上) のテストを許容する。
- 既存テスト `extract_dimensions_handles_pic_order_cnt_type_1` は **テスト関数名 / アサート対象 (`extract_dimensions_from_sps`) / テスト本体スタイル (`expect("SPS パース成功")` で width / height をアサート) すべて維持** し、`.with_pic_order_cnt_type_1()` 呼び出しのみを `.with_pic_order_cnt_type(1)` に書き換える（テスト名の `_1` は `pic_order_cnt_type == 1` の経路を意味するため、setter リネーム後もそのまま意味が通る）。

`SpsBuilder::build()` 側の変更は不要。`pic_order_cnt_type` 値そのものは `w.write_ue(self.pic_order_cnt_type)` で無条件出力するため、`pic_order_cnt_type = 3` でもバイト列は出力できる。追加バイトは `match self.pic_order_cnt_type { 0 => ..., 1 => ..., _ => {} }` の `_` アームで書かないが、reader 側も読み出し直後に Err を返すため整合する。

新規テスト `parse_sps_rejects_pic_order_cnt_type_out_of_range` を以下の方針で追加する:

- 配置: `src/video/h264.rs` の `#[cfg(test)] mod tests` 内、`parse_sps の単体テスト群` コメントブロック内の末尾（既存 `parse_sps_rejects_bit_depth_chroma_minus8_out_of_range` の直後、`h264_sample_entry_from_sps_pps_lists の単体テスト群` コメントブロックの直前）に追加する。
- アサート対象: `parse_sps(&sps)` を直接呼んで `result.is_err()` を検証する（既存 `parse_sps_rejects_*` 4 件と同じスタイル）。
- 入力: `SpsBuilder::raw(1920, 1088).with_pic_order_cnt_type(3).build()`。`SpsBuilder::raw` のデフォルト `profile_idc = 66` (Baseline) のため `parse_sps` 内の High 系プロファイル追加フィールド読み出し (`read_high_profile_sps_fields`) を経由せず、`skip_pic_order_cnt_type_extras` に直行して本 issue で追加する `> 2` 検査で Err を返す。
- Err メッセージ文言の内容検証 (`format!("{err:?}").contains(...)`) は行わず `is_err()` のみとする（既存 `parse_sps_rejects_*` 4 件と粒度を揃える）。

テスト本体は既存 `parse_sps_rejects_*` 4 件と同じ形式で書く:

```rust
#[test]
fn parse_sps_rejects_pic_order_cnt_type_out_of_range() {
    // pic_order_cnt_type=3 (仕様 7.4.2.1.1 の {0,1,2} 値域外) は Err
    let sps = SpsBuilder::raw(1920, 1088)
        .with_pic_order_cnt_type(3)
        .build();
    let result = parse_sps(&sps);
    assert!(
        result.is_err(),
        "pic_order_cnt_type=3 は仕様値域外で Err: {result:?}"
    );
}
```

### PBT との関係

`pbt/tests/prop_h264_sps.rs` の既存 PBT は `let _ = extract_dimensions_from_sps(&sps);` で戻り値を捨ててクラッシュフリーのみを検証しているため、本 issue による Err 化は PBT の合否に影響しない。構造化 Strategy で `pic_order_cnt_type` 仕様外値の Err 経路を不変条件として検証する作業は issue 0049（PBT 構造化）に委ね、本 issue では `pbt/` を触らない。

## 完了条件

- `skip_pic_order_cnt_type_extras` が `pic_order_cnt_type > 2` のとき Err を返す（Err メッセージ文言は設計方針 → Err 化のサンプルに従う）。
- `SpsBuilder::with_pic_order_cnt_type(value: u32)` が追加され、既存 `with_pic_order_cnt_type_1()` が削除されている。
- 既存テスト `extract_dimensions_handles_pic_order_cnt_type_1` の `.with_pic_order_cnt_type_1()` 呼び出しのみが `.with_pic_order_cnt_type(1)` に書き換わっている（関数名 / アサート対象 / 本体スタイルは設計方針に従い維持）。
- `SpsBuilder::build()` の `match self.pic_order_cnt_type` ロジックは変更されていない。
- 新規テスト `parse_sps_rejects_pic_order_cnt_type_out_of_range` が `parse_sps の単体テスト群` コメントブロック内末尾に追加され、`SpsBuilder::raw(1920, 1088).with_pic_order_cnt_type(3).build()` のバイト列で `parse_sps` を呼んだとき `is_err()` であることをアサートする。
- 既存テスト（`cargo test`）が全て pass する。
- `cargo clippy --all-targets -- --deny warnings` / `cargo fmt --all -- --check` がパスする。

## 解決方法

ブランチ `feature/refactor-reject-invalid-pic-order-cnt-type` で実装した。

### `skip_pic_order_cnt_type_extras` への値域検査追加

- `src/video/h264.rs::skip_pic_order_cnt_type_extras` 内で `reader.read_ue()` 直後に `if pic_order_cnt_type > 2 { return Err(...) }` を挿入。既存 `read_high_profile_sps_fields` 内の `chroma_format_idc > 3` / `bit_depth_*_minus8 > 6` 検査と同じ「読み出し直後に値域検査して早期 return」スタイルに統一。
- Err メッセージは `"invalid H.264 SPS: pic_order_cnt_type out of spec range (0..=2): {pic_order_cnt_type}"` で、既存 `parse_sps` 経路の値域 Err 群 (`<field> out of spec range (0..=N): {value}`) と完全整合。
- `match` 末尾の `_ => {}` は事前検査により `pic_order_cnt_type == 2` のときのみ通る形になり、既存コメント「pic_order_cnt_type == 2 のときは追加読み出しなし」はそのまま意味が保たれる。

### `SpsBuilder` の setter 一般化

- `SpsBuilder::with_pic_order_cnt_type_1()`（引数なしトグル）を削除し、`with_pic_order_cnt_type(value: u32)` を新設。既存の `with_chroma_format_idc(value: u32)` / `with_profile_idc(profile_idc: u32)` / `with_bit_depth_luma_minus8(value: u32)` 等の `u32` 引数 setter 群と命名規則・引数型を揃えた。
- 既存テスト `extract_dimensions_handles_pic_order_cnt_type_1` の `.with_pic_order_cnt_type_1()` 呼び出しを `.with_pic_order_cnt_type(1)` に追従。関数名・アサート対象 (`extract_dimensions_from_sps`)・本体スタイルは維持。
- `SpsBuilder::build()` の `match self.pic_order_cnt_type` ロジックは無変更（`pic_order_cnt_type = 3` でも `w.write_ue(self.pic_order_cnt_type)` で値そのものは出力され、追加バイトは `_` アームで書かない。reader 側も読み出し直後に Err を返すため整合）。

### テスト追加

- `parse_sps_rejects_pic_order_cnt_type_out_of_range` を `parse_sps の単体テスト群` コメントブロック内末尾、既存 `parse_sps_rejects_bit_depth_chroma_minus8_out_of_range` の直後に追加。`SpsBuilder::raw(1920, 1088).with_pic_order_cnt_type(3).build()` で `parse_sps` を呼んで `result.is_err()` のみアサート（既存 `parse_sps_rejects_*` 4 件と同じ粒度）。
- `extract_dimensions_handles_pic_order_cnt_type_0` を `extract_dimensions_handles_pic_order_cnt_type_1` の直後に追加。`pic_order_cnt_type = 0` 経路（`log2_max_pic_order_cnt_lsb_minus4` 読み飛ばし）の正常通過と width / height 値を確認する（`/review-diff-code` の境界値補強指摘への対応）。

### レビュー指摘の反映

`/review-diff-code` の指摘を反映した。

- テストコメントから「本 issue で追加した」表記を含む経路追跡コメント 2 行を削除。`shiguredo-issues` 規約「ソースコード本体への issue 参照禁止」と既存 `parse_sps_rejects_*` 群のコメント粒度に揃えた。
- 境界値補強として `pic_order_cnt_type = 0` 経路の正常系単体テストを追加。

### CHANGES.md

記載なし。`## develop` 内中間状態の堅牢性補強で、仕様準拠の publisher (libx264 / ハードウェアエンコーダ等) が生成する SPS では発生しない挙動変化のため、利用者観測挙動に影響しない。

## 関連

- issue 0037（closed: `feature-add-h264-sps-dimensions-parser`）。
- issue 0043（closed: `feature-refactor-h264-sample-entry-from-sps-pps-lists`）。`SpsBuilder` を `with_<field>(value: u32)` の一般 setter スタイルに統一し、`parse_sps` 直接呼び出しの Err テスト群を確立した closed issue。本 issue が踏襲する命名規則・テストスタイルの原典。
- issue 0049（open: `feature-refactor-prop-h264-sps-structured-strategy`）。0049 の構造化 Strategy 化で `SpsBuilder::with_pic_order_cnt_type(value: u32)` を再利用し、本 issue の Err 経路を PBT でカバーする想定。
