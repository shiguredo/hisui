# [BUG] pbt/tests/prop_h264_sps.rs の ok_path 系 4 テストが bit reader exhausted で確定失敗する

- Priority: High
- Created: 2026-07-03
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-h264-sps-pbt-bit-reader-exhausted
- Polished: 2026-07-03

## 目的

`pbt/tests/prop_h264_sps.rs` の `ok_path::` 系 5 テストのうち、`ok_sps_strategy()` を共有する 4 テストが `parse_sps` の bit reader 枯渇で確定失敗する。真因は `build_sps_for_pbt` が生の RBSP をそのまま返す一方で `parse_sps` は入口の `rbsp_from_sps_nalu` で EBSP から RBSP への変換 (emulation prevention byte 除去) を通す非対称にあり、生成 SPS 内に偶発的に `0x00 0x00 0x03` パターンが発生すると `rbsp_from_sps_nalu` が `0x03` を除去して RBSP が 1 バイト縮み、後段の bit reader が末尾を超えて枯渇する。`build_sps_for_pbt` に emulation prevention byte 挿入を追加して返り値契約を EBSP に揃え、`h264_sample_entry_from_sps_pps_lists` の入力契約と対称にする。

## 優先度根拠

High。ローカル環境で確定的に再現し、develop 単体で失敗する。以下の理由で放置できない。

- `ok_sps_strategy()` は `raw_width` / `raw_height` を最大 65520 (`pbt/tests/prop_h264_sps.rs:33-50` の `raw_width_strategy` / `raw_height_strategy`) まで生成するため、`pic_width_in_mbs_minus1` / `pic_height_in_map_units_minus1` の ue(v) が最大 23 ビット幅になり、生成 SPS バイト列に偶発的に `0x00 0x00 0x03` パターンが発生する経路が広い。shrink で残った 1 seed だけでなく Strategy 値域全体で相応の割合の failing input が潜在しており、seed 分布次第で CI でも一定確率で必ず失敗する。加えて `pbt/tests/prop_h264_sps.proptest-regressions` に保存済みの seed 1 件は実行のたびに再現されるため、少なくとも 1 件は確定失敗する。
- 本番の inbound endpoint / mp4 reader / video decoder 経路は `parse_sps` (`src/video/h264.rs:546-601`、`h264_sample_entry_from_sps_pps_lists` 経由、`src/video/h264.rs:305-365`) を骨格に持つ。本件は PBT ヘルパ (`build_sps_for_pbt`) 側のバグだが、`parse_sps` に対する PBT の Ok 経路カバレッジが実質壊れており、パーサ側のリグレッションを PBT で検出できない状態が続いている。

## 現状

### 症状

`ok_sps_strategy()` (`pbt/tests/prop_h264_sps.rs:77-128`) を共有する 4 テストが確定失敗する。同じ `mod ok_path` 内の 5 番目テストと `mod err_path` の 7 テストは pass する。差の直接原因は各 Strategy が生成する SPS バイト列の値域と長さの違いで、pass 側は SPS バイト列が短く `0x00 0x00 0x03` パターンを含まないため。

- 失敗テスト (4 件、いずれも `params in ok_sps_strategy()` を受ける):
  - `ok_path::prop_h264_sample_entry_round_trips_profile_level_constraint`
  - `ok_path::prop_h264_sample_entry_reflects_high_profile_fields`
  - `ok_path::prop_h264_sample_entry_preserves_sps_pps_lists`
  - `ok_path::prop_h264_sample_entry_visual_matches_frame_size`
- pass するテスト:
  - `ok_path::prop_h264_sample_entry_reflects_cropping_in_visual_and_frame_size` (`ok_sps_with_cropping_strategy()` を使う。小さい固定値のみで SPS が短い)
  - `err_path::` 系 7 件すべて (`baseline_ok_params()` の `raw_width=320, raw_height=240, pic_order_cnt_type=2` で SPS 全 8 バイト `0x67 0x42 0x00 0x1F 0xDA 0x05 0x07 0xC0`。`0x00 0x00 0x03` を含まない)
- 枯渇箇所: `src/video/bit_reader.rs:47-51` の `read_bit` (`byte_pos >= data.len()` のとき `bit reader: exhausted before requested read` を返す)。呼び出し元は `parse_sps` (`src/video/h264.rs:546-601`) 経由の `read_ue` / `read_u`。

### 再現手順

```
git checkout develop
SDKROOT=$(xcrun --sdk macosx --show-sdk-path) cargo test -p pbt --test prop_h264_sps
```

3 回連続で同じ 4 テストが失敗することを確認済み (develop 単体 / 本 issue のブランチの両方)。`pbt/tests/prop_h264_sps.proptest-regressions` に保存済みの seed が 1 件あり、実行のたびに再現されるため確定失敗する。

### Minimal failing input

`pbt/tests/prop_h264_sps.proptest-regressions:7` に登録済みの `visual_matches_frame_size` の shrink 結果。他 3 テストも同じ `ok_sps_strategy()` を共有するため、同一分布で failing する:

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

この入力を `build_sps_for_pbt` (`src/video/h264.rs:72-138`) に流すと次の 12 バイトの SPS が生成される (バイト位置 7-9 に `0x00 0x00 0x03` が出現)。

```
0x67 0x42 0x00 0x00 0xE4 0x40 0x01 0x00 0x00 0x03 0x44 0xA0
```

原理は次のとおり。`pic_width_in_mbs_minus1 = 2047` の ue(v) は「11 個の 0 + 12 ビット `100000000000`」 (23 ビット)。続く `pic_height_in_map_units_minus1 = 3345` の ue(v) は「11 個の 0 + 12 ビット `110100010010`」 (23 ビット、`3346 = 0xD12` の上位 2 ビットが両方 1)。この 2 つの ue(v) が連続することでバイト位置 8 の全ビットと位置 9 の上位 6 ビットが 0 に埋まり、位置 9 の下位 2 ビットに ue(3345) の 12 ビット表現の 1・2 番目のビット (両方 1) が並んで `00000011 = 0x03` を作る。結果としてバイト位置 7-9 が `0x00 0x00 0x03` となる。

### 真因

`build_sps_for_pbt` は生の RBSP をそのまま返している (`src/video/h264.rs:72-138`)。一方 `parse_sps` は入口で `rbsp_from_sps_nalu` (`src/video/h264.rs:785-816`) を呼び、「入力は EBSP 形式 (emulation prevention byte 込み)」という契約 (`src/video/h264.rs:289-292` の `h264_sample_entry_from_sps_pps_lists` docstring) のもとで `0x00 0x00 0x03` を検出したら 3 バイト目の `0x03` を除去して RBSP を復元する (ISO/IEC 14496-10 7.4.1.2.3 準拠)。

したがって `build_sps_for_pbt` の出力に偶発的に `0x00 0x00 0x03` が含まれると、`rbsp_from_sps_nalu` がその `0x03` を emulation prevention byte として除去し、RBSP が想定より 1 バイト (8 ビット) 短くなる。Minimal failing input のようにビット位置が SPS 末尾に近い箇所で `0x00 0x00 0x03` が発生した場合、後段の `read_ue` / `read_u` がバッファ末尾を超えて exhausted になる。

真因は `build_sps_for_pbt` が「EBSP を返す」契約を満たしていない (emulation prevention byte 挿入が実装されていない) ことにある。`parse_sps` / `rbsp_from_sps_nalu` の挙動は仕様通りであり、Strategy (`ok_sps_strategy()`) の値域も u16 上限内 (最大 65520) に収まっているため、これらは修正対象ではない。

## 設計方針

`build_sps_for_pbt` の返り値契約を「EBSP 形式 (emulation prevention byte 込み)」に揃える。以下 3 点で対応する。

1. `build_sps_for_pbt` (`src/video/h264.rs:72-138`) の末尾 `w.into_bytes()` を、EBSP 変換ループを通した `Vec<u8>` の返却に差し替える。`w.into_bytes()` が返す生バイト列は先頭 1 バイトが NAL header (`0x67`)、その直後に生 RBSP payload が続く形で、RBSP trailing bits は含まない (`src/video/h264.rs:136` のコメントを参照。`parse_sps` は cropping まで読み終えるとそこで停止するため trailing bits がなくても bit reader 経路は完走できる)。この生バイト列に対し、出力ストリームベースの走査で emulation prevention byte を挿入する。以下の Rust コード (擬似ではなくそのままコピペで動作) をそのまま置き換える。

   ```rust
   let raw = w.into_bytes();
   // raw は NAL header 1 バイト + 生 RBSP payload (trailing bits なし)。
   // build_sps_for_pbt 先頭で NAL header の 8 ビットを無条件に書くため raw.len() >= 1 が保証される。
   // 最悪ケースでは raw.len() の約 1/2 が emulation prevention byte として挿入され得るため、
   // 近似の初期容量として raw.len() + raw.len() / 2 + 1 を使う (それを超える場合は動的拡張に任せる)。
   let mut out = Vec::with_capacity(raw.len() + raw.len() / 2 + 1);
   out.push(raw[0]);
   for &b in &raw[1..] {
       let n = out.len();
       // Rust の && 短絡評価に依存: n >= 2 のときのみ out[n - 2] / out[n - 1] にアクセスする。
       if n >= 2 && out[n - 2] == 0x00 && out[n - 1] == 0x00 && b <= 0x03 {
           out.push(0x03);
       }
       out.push(b);
   }
   out
   ```

   出力ストリームベース (すでに `out` に書いた末尾 2 バイトと次入力バイトを見る) にする理由は、入力ベースの単純な `i += 3` 走査だと `0x00 0x00 0x00 0x00 0x03` のような跨ぎパターンで 2 個目の `0x00 0x00 0x03` を検出できず、`rbsp_from_sps_nalu` を通したときに元の RBSP に戻せなくなるため。本方式は `rbsp_from_sps_nalu` (`src/video/h264.rs:785-816`) の厳密な逆写像となる (writer が挿入した `0x03` の位置は必ず「出力の直前 2 バイトが `0x00 0x00`」であり、reader はその `0x03` を検出して除去する)。

2. `build_sps_for_pbt` の docstring (`src/video/h264.rs:64-71`) の末尾行「0 を渡すと `raw_width / 16 - 1` が u32 underflow して panic する。」の直後に、以下の 1 段落を追加する。

   > 戻り値は ISO/IEC 14496-10 7.4.1.2.3 に従う EBSP 形式 (emulation prevention byte 込み) で、`h264_sample_entry_from_sps_pps_lists` の入力契約 (`src/video/h264.rs:289-292`) と対称な形式となる。

3. `parse_sps` / `rbsp_from_sps_nalu` / Strategy (`ok_sps_strategy` / `ok_sps_with_cropping_strategy`) には手を入れない。修正対象を `build_sps_for_pbt` 1 箇所に絞る。

`build_sps_for_pbt` は PBT (`pbt/tests/prop_h264_sps.rs`) と本体 `cfg(test)` の `SpsBuilder` (`src/video/h264.rs:1082-1180`) の両方から呼ばれる。本番経路には露出していないため本番動作への影響はない。`src/video/h264.rs:1182-1587` に `#[test]` は 24 件あり、そのうち `SpsBuilder` 経由で `build_sps_for_pbt` を通るのは 23 件 (残る 1 件 `h264_sample_entry_from_sps_pps_lists_returns_err_on_empty_sps_list` は空 `sps_list` を渡す Err 検証で `build_sps_for_pbt` を経由しない)。23 件はいずれも `parse_sps` 経路を通り、`build_sps_for_pbt` が返す EBSP は `rbsp_from_sps_nalu` で必ず元の RBSP に戻される (writer と reader は逆写像として対称)。したがって RBSP レベルでは差分がなく、23 件全てと `SpsBuilder` 未経由の 1 件も修正後に引き続き pass する。

## 完了条件

- `cargo test -p pbt --test prop_h264_sps` が develop で 3 回連続 pass する (seed 差揺らぎ検出)。
- `cargo test -p hisui --lib video::h264` が pass する (`src/video/h264.rs:1182-1587` の cfg(test) テスト 24 件、うち `SpsBuilder` 経由 23 件の非回帰確認)。
- `cargo test --workspace` が pass する (他モジュールへの副作用がないことの確認)。
- `pbt/tests/prop_h264_sps.proptest-regressions` の既存 seed 行を維持し、修正後に同 seed が pass することで proptest 経路の回帰防止とする。この seed は `visual_matches_frame_size` の shrink 結果で、修正後にこの 1 seed が pass すれば少なくとも同 input で当該テスト経路が失敗しないことは確定する。他 3 テストは今後別の seed で shrink 収束する可能性を残すが、下記の明示的単体テストで代表点をカバーする。
- `src/video/h264.rs::tests` の `h264_sample_entry_from_sps_pps_lists` 単体テスト群 (`src/video/h264.rs:1446-1587` の末尾) に、Minimal failing input と同一値の `SpsBuildParams` を直接ハードコードした `#[test]` を 1 件追加する。命名候補は `h264_sample_entry_from_sps_pps_lists_handles_sps_with_embedded_emulation_prevention_pattern`。検証内容は次の 3 点で、EBSP 変換が RBSP を破壊せず正しく元 RBSP に戻ることを直接確認する:
  - `h264_sample_entry_from_sps_pps_lists` が Ok を返すこと。
  - 戻り値タプルの `VideoFrameSize` が `width = 32768`, `height = 53536` (Minimal failing input の `raw_width` / `raw_height`) と一致すること。
  - `Avc1Box.avcc_box.avc_profile_indication = 66`, `avc_level_indication = 0`, `profile_compatibility = 0` が Minimal failing input の `profile_idc` / `level_idc` / `constraint_set_flags` と一致すること。
- proptest-regressions と明示的単体テストは補完関係にあり、両方残す (proptest は Strategy 全体を確率的に舐めて広範なリグレッションを検出、単体テストは Minimal failing input 相当の 1 点を確定的に保証)。
- CHANGES.md への追記は不要。修正対象は cfg(test) と PBT からのみ呼ばれる `build_sps_for_pbt` (テストヘルパ) で、利用者から見える挙動は変わらない。

## 解決方法

「設計方針」節の 3 点を次の順に実装する。

1. `build_sps_for_pbt` (`src/video/h264.rs:72-138`) の末尾 `w.into_bytes()` を、設計方針 1 の Rust コード (EBSP 変換ループ) に差し替える。
2. `build_sps_for_pbt` の docstring 末尾に、設計方針 2 の 1 段落を追加する。
3. `src/video/h264.rs::tests` の `h264_sample_entry_from_sps_pps_lists` 単体テスト群末尾に、Minimal failing input を投入する `#[test]` を追加する (`frame_size` と avcC フィールドの一致まで検証)。
4. `cargo test -p pbt --test prop_h264_sps` を 3 回連続実行し、`ok_path::` 全 5 テスト・`err_path::` 全 7 テストが pass することを確認する。加えて `cargo test -p hisui --lib video::h264` および `cargo test --workspace` を通す。

## 参考

- ISO/IEC 14496-10 7.4.1.2.3 (Encapsulation of an SODB within an RBSP、emulation prevention byte の挿入規則)
