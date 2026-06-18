# extract_dimensions_from_sps の pic_order_cnt_type 仕様外値 (0/1/2 以外) を Err 化して仕様準拠の堅牢性を補強する

- Priority: Low
- Created: 2026-06-18
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-reject-invalid-pic-order-cnt-type
- Polished: {YYYY-MM-DD}

## 目的

issue 0037（closed）で導入された `src/video/h264.rs` の `skip_pic_order_cnt_type_extras` 関数は、`pic_order_cnt_type` の値が 0 / 1 / 2 のいずれかであることを ITU-T H.264 仕様 7.4.2.1.1 で要求されているにもかかわらず、それ以外の値を黙って通している（`_ => {}` で何もせず先に進む）。本 issue では仕様外値を `Err` として早期に弾くことで、不正 / 破損 SPS に対する内部堅牢性を補強する。

## 優先度根拠

Low。

- 仕様準拠の publisher（libx264 / ハードウェアエンコーダ等）では `pic_order_cnt_type` が 0 / 1 / 2 以外の値を出すことは無いため、実運用での発生はほぼ無い。
- 仕様外値が来た場合の現挙動は「後続フィールドの読み出しを続行 → 誤った解像度を返すか、別の Err で弾かれる」で、最悪のシナリオでも誤解像度の MP4 sample_entry が生成される程度。クラッシュや無限ループにはならない（PBT で確認済み）。
- issue 0037 で `num_ref_frames_in_pic_order_cnt_cycle > 255` の仕様上限検査は追加済み。本 issue は同じ整合性で `pic_order_cnt_type` の値自体も検査する補強。

## 現状

行番号は HEAD（develop = 7134dab9）時点。実装着手時は grep で再特定する。

### 該当箇所

`src/video/h264.rs` の `skip_pic_order_cnt_type_extras` 関数（行番号は HEAD 時点で 250 付近）:

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

ITU-T H.264 仕様 7.4.2.1.1 で `pic_order_cnt_type` は 0 / 1 / 2 のいずれかと規定。それ以外の値はエンコーダのバグや伝送破損で発生する可能性のみ。

### 既存テスト

`src/video/h264.rs` の `#[cfg(test)] mod tests` に `SpsBuilder` がある（issue 0037 で導入）。`pic_order_cnt_type` をデフォルト 2 で設定し、`with_pic_order_cnt_type_1()` で 1 に切り替えるメソッドが既存。本 issue ではこれを拡張して任意の値を設定できるようにするか、`with_invalid_pic_order_cnt_type(value)` のようなテスト専用メソッドを追加する。

## 設計方針

### Err 化

`skip_pic_order_cnt_type_extras` の `_ => {}` を `_ => Err(...)` に変える:

```rust
_ => {
    return Err(crate::Error::new(format!(
        "invalid H.264 SPS: pic_order_cnt_type out of range (0..=2): {pic_order_cnt_type}"
    )));
}
```

### テスト

`SpsBuilder` に `with_pic_order_cnt_type(value: u32)` を追加（任意の値を設定可能にする）するか、テスト関数内で SPS バイト列を直接構築する。`pic_order_cnt_type = 3` の SPS で `extract_dimensions_from_sps` が Err を返すことをアサートする単体テストを 1 件追加。

## 完了条件

- `skip_pic_order_cnt_type_extras` が `pic_order_cnt_type` が 3 以上のとき `invalid H.264 SPS: pic_order_cnt_type out of range (0..=2): ...` 相当の Err を返す。
- `pic_order_cnt_type = 3` を含む SPS で `extract_dimensions_from_sps` が Err を返すことを確認する単体テストが追加されている。
- 既存テスト（特に `pic_order_cnt_type = 0 / 1 / 2` の経路を踏むテスト）が全て pass する。
- `cargo test` / `cargo clippy --all-targets -- --deny warnings` / `cargo fmt --all -- --check` がパスする。

### CHANGES.md

本 issue では記載しない（内部堅牢性向上で外部から観測可能な挙動変化は仕様外入力時のみ。仕様準拠の publisher では発生しない）。issue 0037 の方針と同様。

## 解決方法

実装着手後にここに記述する。

## 関連

- issue 0037（closed: `feature-add-h264-sps-dimensions-parser`。本 issue 対象の `skip_pic_order_cnt_type_extras` 関数を導入。本 issue は同経路の堅牢性補強で、issue 0037 でも `num_ref_frames_in_pic_order_cnt_cycle > 255` の上限検査は追加済み）
