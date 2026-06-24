# h265_sample_entry / h265_sample_entry_from_annexb を VPS / SPS / PPS リスト受け取り版にリファクタして hvcC フィールドの固定値と Annex-B 走査の独自実装を解消する

- Priority: Low
- Created: 2026-06-19
- Completed: 2026-06-24
- Model: Opus 4.7
- Branch: feature/refactor-h265-sample-entry-from-vps-sps-pps-lists
- Polished: 2026-06-22

## 目的

副次的に外部観測可能な挙動修正 (hvcC ヘッダ各フィールドが固定値 → SPS / VPS 由来実値、codec_string が固定値ベース → SPS / VPS 由来実値) を伴う refactor。`src/video/h265.rs::h265_sample_entry` および `h265_sample_entry_from_annexb` は次の 3 つの broken window を抱えている。

H.265 経路の現状は H.264 経路 (closed/0043 後) の「中間状態」に相当する。`h265_sample_entry(width, height, fps, vps_list, sps_list, pps_list) -> Result<SampleEntry>` は既にリスト受け取り版だが SPS / VPS をパースせず Sora 録画固定値で hvcC を埋めており、`h265_sample_entry_from_annexb(width, height, fps, data) -> Result<SampleEntry>` は Annex-B 走査ロジックを `src/video/h264.rs::H264AnnexBNalUnits` 相当の汎用イテレーターを使わず独自手書きで実装している。

具体的な broken window:

1. **hvcC ヘッダーフィールドの固定値**: `h265_sample_entry` は次の 9 フィールドを Sora 録画前提の固定値で埋めている (コメントに「Sora の録画ファイルに合わせた値（必要に応じて調整すること）」と明記):
   - `general_profile_compatibility_flags: 0x60000000`
   - `general_constraint_indicator_flags: Uint::new(0xb00000000000)` (48 bit)
   - `general_level_idc: 123`
   - `general_profile_space: Uint::new(0)`
   - `general_tier_flag: Uint::new(0)`
   - `general_profile_idc: Uint::new(1)` (Main 固定)
   - `chroma_format_idc: Uint::new(1)` (4:2:0 固定)
   - `bit_depth_luma_minus8: Uint::new(0)` / `bit_depth_chroma_minus8: Uint::new(0)` (8 bit 固定)

   加えて以下のフィールドも固定値で埋まっているが、本 issue では `num_temporal_layers` / `temporal_id_nested` のみ SPS の `sps_max_sub_layers_minus1` / `sps_temporal_id_nesting_flag` 由来実値に置き換える。残りの `min_spatial_segmentation_idc` / `parallelism_type` / `avg_frame_rate` / `constant_frame_rate` / `length_size_minus_one` は固定値維持 (`### 本 issue で触らない経路` 参照)。
2. **`h265_sample_entry_from_annexb` の Annex-B 走査独自実装**: `H264AnnexBNalUnits` 相当の汎用イテレーターが H.265 用に存在せず、`src/video/h265.rs::h265_sample_entry_from_annexb` 内で start code 検出 (`[0, 0, 0, 1]` / `[0, 0, 1]`) を直接書いている。汎用化欠如により、将来 H.265 経路で SRT / RTSP / RTMP inbound を追加した際に同じ走査ロジックを再実装する余地が生じる。なお、encoder 経路 (nvcodec H.265) では Annex-B 走査自体が 1 回しか発生していないため、closed/0043 で H.264 経路に存在した「呼び出し側 + 関数内側の二重走査」は H.265 経路には実在しない。
3. **encoder 経路でのシグネチャ非対称**: `src/encoder/video_toolbox.rs::handle_encoded` 内で H.264 経路は closed/0043 後に `h264_sample_entry_from_sps_pps_lists(sps_list, pps_list)` を呼ぶが、H.265 経路は `h265::h265_sample_entry(width, height, fps, vps_list, sps_list, pps_list)` を呼ぶ。同じ関数内で並列に呼ばれる 2 つの sample_entry 構築のシグネチャが大きく異なる。

closed/0043 (H.264) と closed/0050 (RTMP H.264) で確立した薄いラッパー化方針 (新ヘルパー関数 + Annex-B ラッパー、タプル戻り値 `(SampleEntry, VideoFrameSize)`、空 list 検査、破壊的シグネチャ変更) を H.265 経路にも適用し、対称性を回復する。

本 issue の改修対象は厳密には以下 2 経路にまたがる:

- **Hev1 経路**: 2025.2.0 以降で release 済み (`Hev1Box + HvccBox` 固定値で hvcC を出していた)。
- **Hvc1 経路**: develop 内の未リリース変更 (`## develop` 内 CHANGE `出力 MP4 ファイルが H.265 ストリームを含む場合は hvc1 ボックスを使用する`)。

ただし develop 時点で `src/video/h265.rs::h265_sample_entry` は既に `SampleEntry::Hvc1(Hvc1Box { hvcc_box: ... })` のみを構築する実装に切り替わっており、`SampleEntry::Hev1` を構築する経路は src/ 配下に残っていない。本 issue の新ヘルパー関数も `SampleEntry::Hvc1` のみを構築する設計 (既存挙動維持)。よって本 issue の改修による外部観測可能挙動変化は **未リリースの hvc1 経路に閉じる**。

それでも video_toolbox H.265 / nvcodec H.265 のエンコード機能自体は 2025.2.0 以降 release 済みのため、hvcC 内部実値化と空 VPS / SPS / PPS skip ガード追加 (`### CHANGES.md` 節参照) を CHANGES.md `## develop` に `[UPDATE]` で記載する。closed/0043 (`## develop` 内未リリースの HLS / RTSP / SRT inbound) と判断軸が異なる点に注意。

## 優先度根拠

Low。主目的は内部リファクタリング (シグネチャ対称性回復、Annex-B 走査ロジック汎用化、broken window 解消) で、副次的に hvcC が H.265 仕様 (ITU-T H.265) + ISO/IEC 14496-15 仕様の SPS / VPS 由来実値に揃う方向の修正。下流プレイヤー / ツールの互換性に対する影響は中立から改善寄り。実害は発生していない (Sora 録画固定値で Main プロファイル 8 bit 4:2:0 のストリームを想定通りに出せている) ため Low を維持する。

副次的な外部観測可能挙動変化を以下に列挙する:

- `general_profile_idc` が固定値 `1` (Main) から SPS 由来実値に変わる。Hisui の入出力前提では Main / Main 10 のいずれかになる想定。
- `general_level_idc` が固定値 `123` (= Level 4.1 相当?) から SPS 由来実値に変わる。
- `general_profile_compatibility_flags` (32 bit) / `general_constraint_indicator_flags` (48 bit) / `general_profile_space` (2 bit) / `general_tier_flag` (1 bit) が固定値から SPS profile_tier_level 由来実値に変わる。
- `chroma_format_idc` (2 bit) / `bit_depth_luma_minus8` (3 bit) / `bit_depth_chroma_minus8` (3 bit) が固定値から SPS 由来実値に変わる。Main 10 ストリーム (10 bit) で現在 8 bit 表示になる挙動が、SPS 由来実値で正しく 10 bit になる。
- `num_temporal_layers` が固定値 `0` から `sps_max_sub_layers_minus1 + 1` 由来実値に変わる。意味的には ISO/IEC 14496-15 §8.3.3.1.2 で `0` = temporal scalability 不明 / 未指定、`1` = temporally not scalable (単一レイヤー)、`N > 1` = N 個のサブレイヤー、と定義されている。Hisui 入出力前提では Single layer (`sps_max_sub_layers_minus1 = 0` → `num_temporal_layers = 1`) になる想定で、「不明」から「単一レイヤー」への意味的変化が下流の MP4 デコーダ / プレイヤーで temporal layer 情報を参照する処理に影響する可能性がある (実用上の影響は低)。
- `temporal_id_nested` が固定値 `0` から SPS の `sps_temporal_id_nesting_flag` 由来実値に変わる。
- `src/codec_string.rs::build_hevc_codec_string` 経由で生成される H.265 codec_string が現在の `hvc1.1.6.L123.B0` 系固定パターンから SPS 由来実値ベースに変わる。`build_hevc_codec_string` は `general_profile_compatibility_flags.reverse_bits()` を 16 進数表現するため、SPS の `general_profile_compatibility_flag[0..32]` を u32 に MSB ファースト詰めしたうえで bit 反転した値が codec_string に出る。例: SPS の `general_profile_compatibility_flag[1] == 1` (Main プロファイル) → u32 値 `0x40000000` → `reverse_bits` 後 `0x00000002` → codec string `hvc1.1.2.L<level>.B<constraints>` 系。HLS マニフェストの `EXT-X-STREAM-INF.CODECS` 属性 / DASH の MPD `RepresentationCodecs` 属性経由で MP4 出力下流に伝播する。
- video_toolbox encoder H.265 経路でのみ、空 VPS / SPS / PPS の frame (非 keyframe 等) に対するサンプルエントリー構築をスキップする挙動が追加される (closed/0043 の H.264 経路と対称化)。nvcodec encoder H.265 経路 (`new_h265`) は keyframe 時に sequence params を取得する構造のため skip ガードは不要。新ヘルパー関数 `h265_sample_entry_from_vps_sps_pps_lists` が空入力で Err を返す前提と整合する (現状の `h265_sample_entry` は空入力でも Err を返さないため video_toolbox 経路の挙動変化となる)。
- `parse_hevc_sps` の Err 化拡張で、HvccBox の `Uint` 型制約と Hisui の Main / Main 10 想定の交差で値域外となる SPS (例: `chroma_format_idc > 3` / `bit_depth_*_minus8 > 7` / `sps_max_sub_layers_minus1 > 6` 等) は従来 Ok だった経路が Err になる。仕様準拠の publisher (x265 / ハードウェアエンコーダ / Sora 等) では発生しない想定で実害は無い。Format Range Extensions プロファイルで合法な `bit_depth_*_minus8 = 8` は Hisui 用途外で、HvccBox の `Uint<u8, 3>` (0..=7) で表現できないため Err になる。
- 本 issue では `avg_frame_rate` に**触らない**。現状の `(fps.numerator.get().div_ceil(fps.denumerator.get())) as u16` (30 fps → 30) は ISO/IEC 14496-15 §8.3.3.1 が定義する単位 `frames in 256 seconds` (30 fps → 7680) と 256 倍異なるが、本 issue では現状値維持。仕様準拠化は別 issue で対応 (`### 将来別 issue` 参照)。

## 現状

行番号は実装着手時に関連シンボルを grep で再特定する。本文では原則として関数名・型名で参照する。

### 既存の H.265 SPS / VPS パーサの有無

`grep -rn "parse_hevc\|parse_h265\|extract_dimensions_from_hevc\|hevc_sps\|h265_sps\|HevcSpsParams\|H265_HIGH_PROFILES" src/ pbt/` の結果、H.265 SPS / VPS の RBSP パーサは現コードベースに存在しない (該当するヒットは Sora 系の `parse_h265_encode_params` / `parse_h265_decode_params` のみで、これらは Sora 録画 layout JSON パーサで SPS / VPS の RBSP は読まない)。本 issue で `parse_hevc_sps` を新規追加する。VPS パースは不要 (`### 本 issue で触らない経路` 参照、`num_temporal_layers` / `temporal_id_nested` 等の VPS 共有部分は単一レイヤー前提で SPS の `sps_max_sub_layers_minus1` / `sps_temporal_id_nesting_flag` から取り出せる)。

### 改修対象 (本番経路 2 箇所) と追従対象 (テストフィクスチャ 2 箇所 + 新規テストモジュール 1 箇所)

| 呼び出し側                                                            | 種別             | 現状                                                                                                                                              | 改修方針                                                                                                                                                                                                       |
| --------------------------------------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/encoder/video_toolbox.rs::VideoToolboxEncoder::handle_encoded`   | 本番経路         | H.265 経路で `h265::h265_sample_entry(self.width, self.height, self.fps, frame.vps_list.clone(), frame.sps_list.clone(), frame.pps_list.clone())` を呼ぶ。空 VPS / SPS / PPS の skip ガードが無い (H.264 経路にはあり)。 | `frame.vps_list.clone()` / `frame.sps_list.clone()` / `frame.pps_list.clone()` を `h265_sample_entry_from_vps_sps_pps_lists` に直接渡し、戻り値タプルの `SampleEntry` を既存通り `SharedSampleEntry::new(...)` で wrap する。空 VPS / SPS / PPS のときはサンプルエントリー構築をスキップする (H.264 経路と対称)。`VideoFrameSize` は H.264 経路と同じく捨て、`VideoFrame.size` は encoder 設定値を維持。 |
| `src/encoder/nvcodec.rs::NvcodecEncoder::new_h265`                    | 本番経路         | nvcodec が返す `seq_params` (Annex-B 形式バイト列を想定) を `h265::h265_sample_entry_from_annexb(width, height, options.frame_rate, &seq_params)` に渡す。 | 薄いラッパー `h265_sample_entry_from_annexb(&seq_params, options.frame_rate)` を引き続き呼ぶ (引数 `width` / `height` を削除した形に追従)。戻り値の `SampleEntry` を既存通り `SharedSampleEntry::new(...)` で wrap する。`shiguredo_nvcodec::Encoder::get_sequence_params()` (H.265) が VPS + SPS + PPS + start code prefix 込みの Annex-B 形式を返すことを実装着手時に確認する。 |
| `src/codec_string.rs::tests::video_codec_string_from_hvc1_sample_entry` | テストフィクスチャ | `src/video/h265.rs::h265_sample_entry()` と同じ固定値 (`general_profile_compatibility_flags: 0x60000000` / `general_level_idc: 123` 等) で `HvccBox` を構築する。assertion 自体は `codec_str.starts_with("hvc1.")` / `codec_str.starts_with("hvc1.1.")` の緩い形なので、fixture 値の更新のみで pass する。 | fixture コメント (「`src/video/h265.rs の h265_sample_entry() が生成する値と一致すること`」) を新関数名 `h265_sample_entry_from_vps_sps_pps_lists` に更新する。fixture の `HvccBox` フィールド値も Main プロファイル + Level 3.1 (例: `general_level_idc: 93` / `general_profile_idc: Uint::new(1)` 等) の SPS 由来実値ベースに更新する。緩い assert は変更不要。 |
| `src/codec_string.rs::tests::from_sample_entries_hvc1_aac`              | テストフィクスチャ | 上記同様、`h265_sample_entry()` 固定値ベースの `HvccBox` fixture を持つ。assertion は `cs.video.starts_with("hvc1.")` の緩い形。 | 上記同様の追従。 |
| `src/video/h265.rs::tests` (新規)                                       | テスト追加       | `grep -rn "#\[cfg(test)\]" src/video/h265.rs` の結果、現状 `#[cfg(test)] mod tests` は存在しない。 | `parse_hevc_sps` / `h265_sample_entry_from_vps_sps_pps_lists` / `H265AnnexBNalUnits` (新設) に対する単体テスト群を新規追加する。詳細は `### テスト追加` 参照。 |

**encoder 注記** (video_toolbox / nvcodec の 2 経路に共通):

- encoder 自身が出力する SPS / VPS は仕様内のため、新ヘルパー関数の `parse_hevc_sps` Err 化拡張で Err になるケースは実用上発生しない。
- 新ヘルパー関数の戻り値タプルの `VideoFrameSize` は使わず捨てる (`let (entry, _) = h265_sample_entry_from_vps_sps_pps_lists(...)?;`)。`VideoFrame.size` は引き続き encoder 設定値 (video_toolbox: `self.width.get()` / `self.height.get()`、nvcodec: 既存ラッパー経由) を使う既存挙動を維持する。
- `SharedSampleEntry::new(...)` で wrap する既存挙動も維持する。

### テストフィクスチャ追従

`src/codec_string.rs::tests` 内の 2 関数 (`video_codec_string_from_hvc1_sample_entry` / `from_sample_entries_hvc1_aac`) は既に assertion が緩い形 (`codec_str.starts_with("hvc1.")` / `codec_str.starts_with("hvc1.1.")`) になっており、fixture の `HvccBox` フィールド値そのものは assert していない。よって追従は以下の最小限で足りる:

- fixture の `HvccBox` フィールド値を `h265_sample_entry_from_vps_sps_pps_lists` の戻り値ベースに更新する (代表値: Main プロファイル / Level 3.1 / 4:2:0 / 8-bit。例: `general_level_idc: 93` / `general_profile_idc: Uint::new(1)` 等)。
- fixture コメント「`src/video/h265.rs の h265_sample_entry() が生成する値と一致すること`」を新関数名 `h265_sample_entry_from_vps_sps_pps_lists` に更新する。
- 緩い assert (`codec_str.starts_with("hvc1.")` / `codec_str.starts_with("hvc1.1.")`) は変更不要 (現状 pass する範囲を維持する)。

### hvcC 固定値の発生箇所

- `src/video/h265.rs::h265_sample_entry`:
  ```rust
  // 以下はSora の録画ファイルに合わせた値（必要に応じて調整すること）
  general_profile_compatibility_flags: 0x60000000,
  general_constraint_indicator_flags: shiguredo_mp4::Uint::new(0xb00000000000),
  general_level_idc: 123,
  general_profile_space: shiguredo_mp4::Uint::new(0),
  general_tier_flag: shiguredo_mp4::Uint::new(0),
  num_temporal_layers: shiguredo_mp4::Uint::new(0),
  temporal_id_nested: shiguredo_mp4::Uint::new(0),
  ...
  // 色空間 (4:2:0)
  chroma_format_idc: shiguredo_mp4::Uint::new(1),
  // kVTProfileLevel_HEVC_Main_AutoLevel に対応する値
  general_profile_idc: shiguredo_mp4::Uint::new(1), // Main
  bit_depth_luma_minus8: shiguredo_mp4::Uint::new(0), // 8 ビット深度
  bit_depth_chroma_minus8: shiguredo_mp4::Uint::new(0), // 8 ビット深度
  ```

  本 issue 完了後はこのコメント群 (「Sora の録画ファイルに合わせた値」「色空間 (4:2:0)」「kVTProfileLevel_HEVC_Main_AutoLevel に対応する値」「8 ビット深度」) は削除し、SPS / VPS 由来実値である旨を新コメントで記述する。

### 既存のテスト

- `src/video/h265.rs` には現状 `#[cfg(test)] mod tests` が存在しない (実装着手時に再確認)。本 issue で `parse_hevc_sps` / 新ヘルパー関数のテストモジュールを新規追加する。
- `src/codec_string.rs::tests::video_codec_string_from_hvc1_sample_entry` / `from_sample_entries_hvc1_aac`: 上記 `### テストフィクスチャ追従` 参照。
- nvcodec / video_toolbox の H.265 経路に対する `#[cfg(test)] mod tests` は実機依存のため CI で実行されない (closed/0043 と同じ判断)。

## 設計方針

### 関数構成

以下 3 段構えにする (closed/0043 と同型)。

#### §1 `h265_sample_entry_from_vps_sps_pps_lists` (新ヘルパー関数、pub)

```rust
pub fn h265_sample_entry_from_vps_sps_pps_lists(
    vps_list: Vec<Vec<u8>>,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
    fps: FrameRate,
) -> crate::Result<(SampleEntry, VideoFrameSize)>
```

- 戻り値: `SampleEntry::Hvc1` と `VideoFrameSize` のタプル。後者は SPS 由来の cropping 適用後解像度。
- 引数 `fps` は HvccBox.avg_frame_rate を計算するために必要 (H.264 の avcC には対応フィールドが無いため `h264_sample_entry_from_sps_pps_lists` には fps 引数が無いが、HvccBox には必要)。
- 入力契約: `vps_list[i]` / `sps_list[i]` / `pps_list[j]` は **NAL ヘッダ 2 バイトを含む raw NAL バイト列** (start code は含まない、EBSP 形式 = emulation prevention byte 込み)。H.264 と異なり H.265 の NAL ヘッダは 2 バイト (forbidden_zero_bit 1 bit + nal_unit_type 6 bit + nuh_layer_id 6 bit + nuh_temporal_id_plus1 3 bit、ITU-T H.265 仕様 7.3.1.2)。新ヘルパー関数の入力契約は `H265AnnexBNalUnits` (新設、`### §2 と H265AnnexBNalUnits` 参照) が返す `H265NalUnit.data` の形式と `HvccBox.nalu_arrays[*].nalus[*]` の格納形式に揃える。仕様節番号は ISO/IEC 14496-15 §8.3.3.1 (HEVC decoder configuration record) で、本 issue 内の他の参照箇所 (`### hvcC フィールドの反映` 表、`### 本 issue で触らない経路`、CHANGES.md ドラフト) でも統一して §8.3.3.1 を引く。
- 所有権は `Vec<Vec<u8>>` で取る (closed/0043 と同方針。`HvccBox.nalu_arrays` に move して再 clone を防ぐ)。
- NAL タイプ検査: `vps_list` / `sps_list` / `pps_list` の **各リストの全要素** の先頭バイトから `(nalu[0] >> 1) & 0x3F` で nal_unit_type (6 bit) を取り出し、`H265_NALU_TYPE_VPS` (32) / `H265_NALU_TYPE_SPS` (33) / `H265_NALU_TYPE_PPS` (34) と比較する。H.264 の `& 0x1F` (NAL ヘッダ 1 バイトの下位 5 bit) とは異なるビット操作になることに注意。closed/0043 の H.264 経路 (PPS に対して全要素検査) と同じ防御レベルに揃える (各 list ごとに for ループで全要素を検査、実装規模はほぼ増えない)。
- 内部で `parse_hevc_sps(sps_list[0].as_slice())?` を 1 回呼んで `HevcSpsParams` を取り出してから、`vps_list` / `sps_list` / `pps_list` をそれぞれ `HvccBox.nalu_arrays` に move する (`parse_hevc_sps` の戻り値 `HevcSpsParams` は値を持ち借用を保持しないため、move 順序の borrow 制約は無い)。複数 SPS は先頭 SPS のパラメータのみを採用する (Hisui の入力前提では複数 SPS は同一内容を想定)。
- `vps_list.is_empty()` のときは `Err("missing H.265 VPS")`、`sps_list.is_empty()` のときは `Err("missing H.265 SPS")`、`pps_list.is_empty()` のときは `Err("missing H.265 PPS")` を返す (現行 `h265_sample_entry_from_annexb` の 3 つの空チェック Err と同型エラーメッセージ)。
- SPS 由来 width / height (cropping 適用後) が偶数とは限らない (H.265 の cropping offset 計算ルール次第)。本 issue では `VideoFrameSize::new(width as usize, height as usize)` を `parse_hevc_sps` 内で `width > 0 && height > 0` と `u16::MAX 上限` を保証した上で呼ぶ (closed/0043 の H.264 経路と同じ無効化保証)。`EvenUsize` 経由の偶数チェックは新ヘルパー関数の戻り値からは削除する (現行 `h265_sample_entry_from_annexb` の `EvenUsize::new` ガードは破壊的シグネチャ変更で消える)。`Hvc1Box.visual.width / .height` は新ヘルパー関数内で SPS 由来 (cropping 適用後) になる。encoder 設定値との乖離は通常発生しない想定。

#### §2 `h265_sample_entry_from_annexb` (薄いラッパー、破壊的シグネチャ変更)

```rust
pub fn h265_sample_entry_from_annexb(data: &[u8], fps: FrameRate) -> crate::Result<SampleEntry>
```

- 内部で `H265AnnexBNalUnits` (新設、後述) を 1 回走査して VPS / SPS / PPS NAL タイプのみを抽出し、`nalu.data.to_vec()` で `Vec<Vec<u8>>` に詰めて `h265_sample_entry_from_vps_sps_pps_lists` を呼ぶ (SEI / IDR / Filler 等の NAL タイプは現コードと同じく無視する)。
- 戻り値は `SampleEntry` のみ (タプルの片側だけを返す薄いラッパー)。nvcodec 経路は `VideoFrameSize` を必要としないため、シグネチャを軽くする (H.264 の `h264_sample_entry_from_annexb` と同型)。
- 引数 `width` / `height` は削除する (破壊的変更)。本 issue 内で全呼び出し側 (nvcodec encoder のみ、現状 1 箇所) を同一 PR で追従する。

#### §2.5 `H265AnnexBNalUnits` (新設、pub、Annex-B 走査の汎用イテレーター)

```rust
pub struct H265AnnexBNalUnits<'a> { ... }
impl<'a> H265AnnexBNalUnits<'a> {
    pub fn new(data: &'a [u8]) -> Self { ... }
}
impl<'a> Iterator for H265AnnexBNalUnits<'a> {
    type Item = crate::Result<H265NalUnit<'a>>;
    fn next(&mut self) -> Option<Self::Item> { ... }
}

#[derive(Debug)]
pub struct H265NalUnit<'a> {
    pub ty: u8,
    pub data: &'a [u8],
}
```

- H.264 の `H264AnnexBNalUnits` / `H264NalUnit` と対称の API。
- `ty` は NAL ヘッダ第 1 バイトから `(byte >> 1) & 0x3F` で抽出した nal_unit_type (6 bit) (`H265_NALU_TYPE_*` と比較するため u8 で保持)。
- `forbidden_zero_bit` の検査は現行 `h265_sample_entry_from_annexb` には無いが、closed/0043 の H.264 経路と対称に新設する。`forbidden_zero_bit` は H.265 でも byte 0 の MSB (bit 7) にあり、H.264 と同じく `(nalu[0] >> 7) != 0` で Err 化する (H.265 で NAL ヘッダが 2 バイトになるのは下位の nuh_layer_id / nuh_temporal_id_plus1 の追加によるもので、forbidden_zero_bit / nal_unit_type の bit 位置は H.264 / H.265 で同じ)。
- `data` は NAL ヘッダ 2 バイトを含む raw NAL バイト列 (start code は含まない、EBSP 形式)。

#### §3 H.265 SPS パーサ (`parse_hevc_sps` 内部関数、非 pub)

`parse_hevc_sps(sps: &[u8]) -> crate::Result<HevcSpsParams>` を内部関数 (非 `pub`) として実装する。`HevcSpsParams` も非 `pub` のモジュール内 `struct` とする。本番経路から `parse_hevc_sps` を直接呼ぶ場面が無く、かつ本 issue では PBT も新設しない (`### PBT の追加` 参照) ため、`extract_dimensions_from_sps` 相当の pub API は H.265 では設けない。

```rust
struct HevcSpsParams {
    // profile_tier_level の general_profile_* 群 (HvccBox の同名フィールドに対応)
    general_profile_space: u8,        // u(2)、ITU-T H.265 仕様 7.4.4、HvccBox.general_profile_space (Uint<u8, 2, 6>) に対応
    general_tier_flag: u8,            // u(1)、HvccBox.general_tier_flag (Uint<u8, 1, 5>) に対応
    general_profile_idc: u8,          // u(5)、HvccBox.general_profile_idc (Uint<u8, 5, 0>) に対応
    general_profile_compatibility_flags: u32, // SPS の general_profile_compatibility_flag[0..32] を MSB ファーストで u32 に詰めた値、HvccBox.general_profile_compatibility_flags (u32) にそのまま代入
    general_constraint_indicator_flags: u64,  // 48 bit (general_progressive_source_flag 以下の連続ビット領域を u64 の bit 47..0 に MSB ファーストで詰めた値)、HvccBox.general_constraint_indicator_flags (Uint<u64, 48>) に対応
    general_level_idc: u8,            // u(8)、HvccBox.general_level_idc (u8) に対応

    // SPS 由来のサブレイヤー / chroma / bit_depth
    sps_max_sub_layers_minus1: u8,    // u(3)、仕様 7.4.3.2.1 で 0..=6 規定 (7 は仕様外)、HvccBox.num_temporal_layers (Uint<u8, 3, 3>) に対応 (num_temporal_layers = sps_max_sub_layers_minus1 + 1、1..=7)
    sps_temporal_id_nesting_flag: u8, // u(1)、HvccBox.temporal_id_nested (Uint<u8, 1, 2>) に対応
    chroma_format_idc: u8,            // 0..=3 (parse_hevc_sps で範囲検証済み、HvccBox の Uint<u8, 2> 制約に整合)
    bit_depth_luma_minus8: u8,        // 0..=7 (parse_hevc_sps で範囲検証済み、HvccBox の Uint<u8, 3> 制約に整合)
    bit_depth_chroma_minus8: u8,      // 0..=7 (parse_hevc_sps で範囲検証済み、HvccBox の Uint<u8, 3> 制約に整合)

    // cropping 適用後の最終解像度 (parse_hevc_sps 内で width / height > 0 と u16::MAX 上限を保証)
    width: u16,
    height: u16,
}
```

- 入力契約は `parse_sps` (H.264) と同じで、**NAL ヘッダ 2 バイトを含む raw NAL バイト列**。NAL タイプ検査 (`(nalu[0] >> 1) & 0x3F == H265_NALU_TYPE_SPS`) は内部ヘルパー `rbsp_from_hevc_sps_nalu` (命名は実装者裁量) で実施する。
- `rbsp_from_hevc_sps_nalu` は NAL ヘッダ **2 バイト** を skip してから emulation prevention byte (`0x00 0x00 0x03`) を除去して RBSP を取り出す (H.264 の `rbsp_from_sps_nalu` の **1 バイト skip** とは異なるが、emulation prevention byte 除去ロジックは同型、ITU-T H.265 仕様 7.3.1.1)。
- H.264 経路の汎用ビットリーダー (`H264BitReader`) を H.265 でも流用する (`### H264BitReader の共有方針` 参照、案 A を採用)。
- ITU-T H.265 仕様 7.3.2.2.1 / 7.3.3 / 7.4.3 に従って Exp-Golomb / 固定長ビットフィールドを読み取る。読み取り順序は仕様準拠で:
  1. `sps_video_parameter_set_id` u(4) - `skip_u(4)`
  2. `sps_max_sub_layers_minus1` u(3) - `read_u(3)` で取り出して `HevcSpsParams.sps_max_sub_layers_minus1` に格納
  3. `sps_temporal_id_nesting_flag` u(1) - `read_u(1)` で取り出して `HevcSpsParams.sps_temporal_id_nesting_flag` に格納
  4. `profile_tier_level(1, sps_max_sub_layers_minus1)` 呼び出し (仕様 7.3.3):
     - `general_profile_space` u(2) / `general_tier_flag` u(1) / `general_profile_idc` u(5) を取り出す
     - `general_profile_compatibility_flag[0..32]` を 1 回の `read_u(32)` でまとめて読み出し u32 として保持する (`BitReader::read_u` (ステップ 0 で共有化) は MSB ファーストなので `flag[j]` の j=0 が u32 の bit 31 に対応)
     - 48 bit の `general_constraint_indicator_flags` 領域は `BitReader::read_u` (ステップ 0 で共有化) が `n > 32 で Err` 制約を持つため一度には読めない。`read_u(32)` で上位 32 bit (bit 47..16) を読み、続けて `read_u(16)` で下位 16 bit (bit 15..0) を読み、`(upper as u64) << 16 | (lower as u64)` で u64 に組み立てる。仕様 7.3.3 で 48 bit 連続領域は `general_progressive_source_flag` u(1) + `general_interlaced_source_flag` u(1) + `general_non_packed_constraint_flag` u(1) + `general_frame_only_constraint_flag` u(1) + Format Range Extensions 等の追加 constraint flag 群 + `general_reserved_zero_*_bits` で構成され、本 issue では仕様構造を解釈せず 48 bit を素のビット列として u64 に詰める。`BitReader` の MSB ファースト前提と `shiguredo_mp4::HvccBox::encode` の `Uint<u64, 48>.get().to_be_bytes()[2..]` 出力 (上位 6 バイト出力) は整合し、u64 の bit 47 = SPS の `general_progressive_source_flag` (= 48 bit 領域の MSB) として `HvccBox.general_constraint_indicator_flags` に詰まる
     - `general_level_idc` u(8) を取り出す
     - サブレイヤー毎の `sub_layer_profile_present_flag[i]` u(1) と `sub_layer_level_present_flag[i]` u(1) を `i = 0..sps_max_sub_layers_minus1` 個分読む (合計 `2 * sps_max_sub_layers_minus1` bit)
     - `sps_max_sub_layers_minus1 > 0` のときのみ `reserved_zero_2bits[i]` を `i = sps_max_sub_layers_minus1..8` 個分読み skip (合計 `(8 - sps_max_sub_layers_minus1) * 2 = 16 - 2 * sps_max_sub_layers_minus1` bit)。`sps_max_sub_layers_minus1 == 0` のときは `if (maxNumSubLayersMinus1 > 0)` 条件で reserved_zero_2bits を読まない (0 bit)
     - 各 `i = 0..sps_max_sub_layers_minus1` で `sub_layer_profile_present_flag[i] == 1` のときは 88 bit の sub_layer profile 領域を skip、`sub_layer_level_present_flag[i] == 1` のときは 8 bit の `sub_layer_level_idc[i]` を skip する
     - Hisui の Single layer 前提 (`sps_max_sub_layers_minus1 == 0`) では reserved_zero_2bits と sub_layer ループの両方が実行されない
  5. `sps_seq_parameter_set_id` ue(v) - `skip_ue()`
  6. `chroma_format_idc` ue(v) - 取り出して範囲検証 (`> 3` で Err)
  7. `separate_colour_plane_flag` u(1) (`chroma_format_idc == 3` のときのみ) - `skip_u(1)`
  8. `pic_width_in_luma_samples` ue(v) - 取り出して `width` に格納
  9. `pic_height_in_luma_samples` ue(v) - 取り出して `height` に格納
  10. `conformance_window_flag` u(1) - 1 のとき 4 つの `conf_win_*_offset` を ue(v) で読み、cropping を `width` / `height` に適用 (`SubWidthC` / `SubHeightC` を `chroma_format_idc` から決定、仕様 6.2 / 7.4.3.2.1)
  11. `bit_depth_luma_minus8` ue(v) - 取り出して範囲検証 (`> 7` で Err、HvccBox の `Uint<u8, 3>` 制約 0..=7 と整合)
  12. `bit_depth_chroma_minus8` ue(v) - 取り出して範囲検証 (`> 7` で Err)
- `parse_hevc_sps` 内の堅牢性 Err 化 (closed/0044 の H.264 SPS パーサと同方針):
  - `chroma_format_idc > 3` → `Err`
  - `bit_depth_luma_minus8 > 7` → `Err` (H.264 の `> 6` とは異なる。ITU-T H.265 仕様で Format Range Extensions プロファイルは 0..=8 を許容するが、HvccBox の `Uint<u8, 3>` (0..=7) と Hisui の Main / Main 10 想定の交差で `> 7` を Err 化する)
  - `bit_depth_chroma_minus8 > 7` → `Err` (同上)
  - `sps_max_sub_layers_minus1 > 6` → `Err` (ITU-T H.265 仕様 7.4.3.2.1 で 0..=6 規定、7 は仕様外)
  - `general_profile_idc` の許容値: 仕様 A.3 の主要プロファイル群 `{1 (Main), 2 (Main 10), 3 (Main Still Picture), 4 (Format Range Extensions), 5 (High Throughput), 6 (Multiview Main), 7 (Scalable Main), 9 (Screen Content Coding)}` の和集合を許容する。Hisui の入力前提 (video_toolbox: `kVTProfileLevel_HEVC_Main_AutoLevel` → Main = 1 / nvcodec: `shiguredo_nvcodec::HevcEncoderConfig::profile` に `AutoSelect` (= NVENC の出力 profile 依存) / `Main` / `Main10` / `Frext` (Format Range Extensions = 4) のいずれか) を考えると、許容リスト `{1, 2, 3, 4, 5, 6, 7, 9}` で nvcodec の `Frext` 出力も含めて全てカバーされる。リスト外は Err 化する (closed/0043 の H.264 `{66, 77, 88} ∪ H264_HIGH_PROFILES` 方針と同型)。
  - 解像度関連: `width == 0 || height == 0` (cropping 適用後) → `Err` / `width > u16::MAX || height > u16::MAX` → `Err`
- 実装コードのコメント・docstring・エラーメッセージには issue 番号や他 issue 由来の比喩 (`closed/0043 と同方針` 等) を書かない。エラーメッセージは「`invalid H.265 SPS: chroma_format_idc out of spec range (0..=3): ...`」のように仕様参照のみで記述する (closed/0050 で確立した shiguredo-issues 規約)。

#### H264BitReader の共有方針 (案 A 採用)

H.264 と H.265 の SPS パースには同じ Exp-Golomb / 固定長ビット読み取りが必要。`H264BitReader` は H.264 専用ロジック (scaling_list の skip 等) を含まない汎用ビットリーダーなので、本 issue では **案 A を採用** する:

- 案 A (採用): `H264BitReader` を `src/video/bit_reader.rs` (新設) に移動し、`BitReader` 等の codec 中立名にリネームして共有する。H.264 経路 (`src/video/h264.rs::parse_sps`) と H.265 経路 (`src/video/h265.rs::parse_hevc_sps`) の両方から `use crate::video::bit_reader::BitReader` で参照する。
- 案 B (不採用): `H265BitReader` を別途新設。コード重複が将来の負債になるため不採用。

`H264BitReader` のリネーム・移動は推奨パッチ順序のステップ 0 (`### 推奨パッチ順序` 参照) で別コミットとして実施する。H.264 経路の本番テスト・PBT を一切壊さない非破壊的リファクタリング。

#### PBT の追加

closed/0044 後の `pbt/tests/prop_h264_sps.rs` (H.264 SPS パーサのクラッシュフリー PBT) と対称に、`pbt/tests/prop_h265_sps.rs` を **新設しない**。理由:

- 現在 open の issue 0049 (`feature-refactor-prop-h264-sps-structured-strategy`、polish 済み) で H.264 SPS PBT を構造化 Strategy に置き換える方針が確定済み。H.265 SPS PBT を本 issue で新設すると 0049 と同じ問題を再生する。
- H.265 SPS の PBT は 0049 完了後に別 issue で対応する (`### 将来別 issue` 参照)。
- 本 issue の `parse_hevc_sps` のクラッシュフリー保証は、共有化された `BitReader` (`### H264BitReader の共有方針` 参照) の境界エラー検査 (バッファ末尾超過で Err、`leading_zeros > 31` で Err、`read_u(n > 32)` で Err) で担保する。これらは H.264 経路で既に確立済み。

### hvcC フィールドの反映 (`h265_sample_entry_from_vps_sps_pps_lists` 内の対応関係)

| hvcC フィールド                            | 反映値                                                                                                                   |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `general_profile_space`                    | `HevcSpsParams.general_profile_space` (SPS profile_tier_level)                                                           |
| `general_tier_flag`                        | `HevcSpsParams.general_tier_flag` (SPS profile_tier_level)                                                               |
| `general_profile_idc`                      | `HevcSpsParams.general_profile_idc` (SPS profile_tier_level)                                                             |
| `general_profile_compatibility_flags`      | `HevcSpsParams.general_profile_compatibility_flags` (u32、SPS の `general_profile_compatibility_flag[0..32]` を `(flag[0] << 31) \| (flag[1] << 30) \| ... \| (flag[31] << 0)` の順で MSB ファースト詰めした u32 値。`shiguredo_mp4::HvccBox` の encode は u32 をそのまま big-endian で書き出すため、`build_hevc_codec_string` の `reverse_bits` 入力としても整合する) |
| `general_constraint_indicator_flags`       | `Uint::new(HevcSpsParams.general_constraint_indicator_flags)` (48 bit、SPS の `general_progressive_source_flag` 以下の連続ビット領域を u64 の bit 47..0 に MSB ファースト詰めした値。`shiguredo_mp4::HvccBox::encode` は `get().to_be_bytes()[2..]` で上位 6 バイトを出力するため、bit 47 = `general_progressive_source_flag` が先頭バイトの MSB として出力される) |
| `general_level_idc`                        | `HevcSpsParams.general_level_idc`                                                                                        |
| `chroma_format_idc`                        | `Uint::new(HevcSpsParams.chroma_format_idc)`                                                                             |
| `bit_depth_luma_minus8`                    | `Uint::new(HevcSpsParams.bit_depth_luma_minus8)`                                                                         |
| `bit_depth_chroma_minus8`                  | `Uint::new(HevcSpsParams.bit_depth_chroma_minus8)`                                                                       |
| `num_temporal_layers`                      | `Uint::new(HevcSpsParams.sps_max_sub_layers_minus1 + 1)` (Hisui の単一レイヤー前提では 1)                                |
| `temporal_id_nested`                       | `Uint::new(HevcSpsParams.sps_temporal_id_nesting_flag)`                                                                  |
| `length_size_minus_one`                    | `Uint::new(NALU_HEADER_LENGTH as u8 - 1)` (現状値維持、Hisui の MP4 出力時固定値)                                        |
| `avg_frame_rate`                           | `(fps.numerator.get().div_ceil(fps.denumerator.get())) as u16` (現状値維持。ISO/IEC 14496-15 §8.3.3.1 の 256 倍仕様への準拠は別 issue で対応、`### 将来別 issue` 参照) |
| `constant_frame_rate`                      | `Uint::new(1)` (現状値維持、Hisui は CFR 前提)                                                                          |
| `min_spatial_segmentation_idc`             | `Uint::new(0)` (現状値維持、SPS VUI 由来の正確な値抽出は別 issue で対応、`### 将来別 issue` 参照)                       |
| `parallelism_type`                         | `Uint::new(0)` (現状値維持、PPS 由来の正確な値抽出は別 issue で対応、`### 将来別 issue` 参照)                            |
| `nalu_arrays`                              | `vec![hvcc_nalu_array(VPS, vps_list), hvcc_nalu_array(SPS, sps_list), hvcc_nalu_array(PPS, pps_list)]` (move、現状ロジック維持) |

### 本 issue で触らない経路

下記経路は本 issue のスコープ外。「将来別 issue で扱う可能性」までで止めて issue 番号は立てない (`### 将来別 issue` で個別に予告)。

- **H.265 decoder 経路 (`src/decoder/` 配下)**: closed/0043 と同方針で本 issue では触らない。decoder の sample_entry 構築は別経路 (`src/decoder/video_toolbox.rs::get_h265_vps_sps_pps` 等は HvccBox の `nalu_arrays` から VPS / SPS / PPS を読み出す側で、本 issue の改修で生成された HvccBox でも引き続き動くこと確認のみ)。
- **MP4 demuxer の Hvc1 経路 (`src/mp4/demuxer.rs::Hvc1`)**: HvccBox 読み出し側。本 issue の改修で生成された HvccBox でも `visual.width / .height` の取り出しは既存通り動くため変更不要。
- **RTMP H.265 経路 (`src/rtmp/`)**: `grep -rn 'h265\|H265\|HEVC\|hevc' src/rtmp/` で 0 件 (現状 RTMP H.265 経路は未実装)。将来 RTMP で H.265 受信が追加されたら別 issue で対応。
- **WebM H.265 経路 (`src/webm/reader.rs`)**: WebM 規格として H.265 は標準外 (WebM は VP8 / VP9 / AV1 のみ)。closed/0047 で WebM AV1 / H264AnnexB を扱ったが H.265 は対象外。本 issue でも触らない。
- **AV1 経路 (`src/video/av1.rs::av1_sample_entry`)**: 同型の Hisui 内エンコーダ前提固定値 (Main profile / 4:2:0 / 8-bit) を持つ broken window。closed/0047 で本 issue を AV1 固定値解消の予告先として参照済み。本 issue では触らず、将来別 issue で対応する (`### 将来別 issue` で一覧化)。
- **`avg_frame_rate` の ISO/IEC 14496-15 §8.3.3.1 仕様 (256 倍単位) への準拠**: 現状の `(fps.numerator.get().div_ceil(fps.denumerator.get())) as u16` (整数切り上げの fps をそのまま入れる) は仕様の「frames in 256 seconds」とは 256 倍異なる。本 issue では現状値維持 (encoder 経路の fps 引数経由の整数値をそのまま埋める)。
- **`min_spatial_segmentation_idc` / `parallelism_type` の SPS VUI / PPS 由来実値抽出**: SPS VUI / PPS のパースが必要で実装範囲が広がるため本 issue では現状値維持 (`Uint::new(0)`)。

### 将来別 issue

本 issue 完了後に別 issue として起票候補:

- AV1 経路 (`src/video/av1.rs::av1_sample_entry`) の Hisui 固定値解消 (closed/0047 が予告済み)
- `avg_frame_rate` の ISO/IEC 14496-15 §8.3.3.1 仕様 (256 倍単位) への準拠
- `min_spatial_segmentation_idc` / `parallelism_type` の SPS VUI / PPS 由来実値抽出 (実害低、優先度 Low)
- H.265 SPS パーサの PBT 追加 (open/0049 の構造化 Strategy 完了後)

### 推奨パッチ順序

実装者は以下の 6 ステップで作業し、各ステップ完了時点で `cargo check && cargo test` が pass する原子コミットを作る。

0. **`H264BitReader` の共有化** (非破壊リファクタ):
   - `src/video/h264.rs` の `H264BitReader` を `src/video/bit_reader.rs` (新設) に移動し、`BitReader` (codec 中立名) にリネームする。
   - `src/video/h264.rs::parse_sps` で `use crate::video::bit_reader::BitReader;` に変更。本番テスト・PBT を一切壊さない非破壊的リファクタリングであることを `cargo test` で確認。
1. **§2.5 `H265AnnexBNalUnits` 追加** (非破壊):
   - `src/video/h265.rs` に `H265AnnexBNalUnits` / `H265NalUnit` を追加 (closed/0043 の `H264AnnexBNalUnits` と対称、`forbidden_zero_bit` 検査 + NAL タイプ抽出 `(byte >> 1) & 0x3F` 込み)。
   - 既存 `h265_sample_entry_from_annexb` 内の手書き走査ロジックを `H265AnnexBNalUnits` に置き換える (引数 / シグネチャは現状維持で内部実装のみ差し替え)。
   - 単体テストを `src/video/h265.rs::tests` (新規モジュール) に追加。
2. **§3 `parse_hevc_sps` 追加** (非破壊):
   - `parse_hevc_sps` / `HevcSpsParams` / `rbsp_from_hevc_sps_nalu` を `src/video/h265.rs` に追加。
   - ステップ 0 で共有化した `BitReader` を `use crate::video::bit_reader::BitReader;` で参照。
   - `parse_hevc_sps` の Err 化拡張 (`chroma_format_idc > 3` / `bit_depth_*_minus8 > 7` / `sps_max_sub_layers_minus1 > 6` / `general_profile_idc` 許容リスト外 等) を初日から組み込む (closed/0044 と同方針)。
3. **テスト拡張**:
   - 仕様準拠の合成 SPS バイト列ビルダー (`HevcSpsBuilder`、closed/0043 の `SpsBuilder` と対称) を `src/video/h265.rs::tests` に新規追加する (Err 境界テストで `bit_depth_luma_minus8 = 8` 等を踏ませる必要があるため、合成 SPS は必須)。
   - 加えて実機 x265 / ffmpeg で生成した実機 SPS バイト列を `pub(crate) const HEVC_SPS_*` で埋め込む (closed/0043 で H.264 は `SpsBuilder` + `SPS_320X240` / `SPS_1920X1080` の両方を持つのと対称)。
   - 必須テストケース: Main / Main 10 の正常系 / 仕様外プロファイル Err / `chroma_format_idc > 3` Err / `bit_depth_luma_minus8 > 7` Err / `bit_depth_chroma_minus8 > 7` Err / `sps_max_sub_layers_minus1 > 6` Err / 1920x1088 raw + crop_bottom で 1920x1080 になる経路。
4. **§1 + §2 + 全 2 呼び出し側追従 + テストフィクスチャ追従** (破壊的、原子コミット):
   - `h265_sample_entry_from_vps_sps_pps_lists` を追加。
   - `h265_sample_entry_from_annexb` を新シグネチャ (`fn(data: &[u8], fps: FrameRate) -> Result<SampleEntry>`) の薄いラッパーに変更。
   - 全 2 呼び出し側 (video_toolbox / nvcodec) を新シグネチャに追従。video_toolbox 側に空 VPS / SPS / PPS skip ガードを追加。
   - `src/codec_string.rs::tests::video_codec_string_from_hvc1_sample_entry` / `from_sample_entries_hvc1_aac` のフィクスチャ追従 (`### テストフィクスチャ追従` 参照)。
5. **dead code 削除**:
   - 旧 `h265_sample_entry` 関数削除。
   - `src/video/h265.rs` の `use crate::types::EvenUsize;` を削除 (新シグネチャで `EvenUsize` を引数に取らない)。
   - `src/encoder/video_toolbox.rs` の `use crate::video::h265;` 由来の `h265::h265_sample_entry` 参照を `h265::h265_sample_entry_from_vps_sps_pps_lists` 参照に置き換える。
   - `src/video/h265.rs` のコメント「Sora の録画ファイルに合わせた値」「色空間 (4:2:0)」「kVTProfileLevel_HEVC_Main_AutoLevel に対応する値」「8 ビット深度」を削除し、SPS / VPS 由来実値である旨を新コメントで記述する。

### 実装コードへの issue 番号埋め込み禁止

closed/0050 で確立した shiguredo-issues 規約に従い、実装コード (本体 + コメント + docstring + エラーメッセージ + テストコメント) には issue 番号 (0048 / 0043 / 0050 等) や `closed/0043` 等の他 issue 由来表現を書かない。issue 番号は issue ファイル管理と git 履歴のためのもので、コード本体には残さない。エラーメッセージは「`invalid H.265 SPS: chroma_format_idc out of spec range (0..=3): ...`」のように仕様参照のみで記述する。本 issue 内の「closed/0043 と対称」「closed/0050 で確立した規約」等の他 issue 参照は issue 本文限定の表現で、実装コードに持ち込まない。

## 完了条件

設計方針の §1〜§3 と完了条件を 1:1 対応で整理する。

### §1 `h265_sample_entry_from_vps_sps_pps_lists` 新設

- `src/video/h265.rs` に `pub fn h265_sample_entry_from_vps_sps_pps_lists(vps_list: Vec<Vec<u8>>, sps_list: Vec<Vec<u8>>, pps_list: Vec<Vec<u8>>, fps: FrameRate) -> Result<(SampleEntry, VideoFrameSize)>` が追加されている。
- 内部で `parse_hevc_sps(sps_list[0].as_slice())` を 1 回呼び、その後 `vps_list` / `sps_list` / `pps_list` を `HvccBox.nalu_arrays` に move する。
- `vps_list.is_empty()` で `Err("missing H.265 VPS")` / `sps_list.is_empty()` で `Err("missing H.265 SPS")` / `pps_list.is_empty()` で `Err("missing H.265 PPS")` を返す。
- `vps_list[0]` / `sps_list[0]` / `pps_list[0]` の先頭バイトから `(nalu[0] >> 1) & 0x3F` で NAL タイプを抽出し、それぞれ `H265_NALU_TYPE_VPS` / `H265_NALU_TYPE_SPS` / `H265_NALU_TYPE_PPS` と一致しなければ Err を返す。
- 構築した `SampleEntry::Hvc1` の `hvcc_box` フィールドが SPS / VPS 由来の実値を持つ (`### hvcC フィールドの反映` 表の通り)。
- `Hvc1Box.visual.width / .height` が SPS 由来の cropping 適用後解像度に変わる。
- 戻り値タプルの `VideoFrameSize` が SPS の cropping 適用後の値と一致する。

### §2 `h265_sample_entry_from_annexb` 薄いラッパー化 (破壊的シグネチャ変更)

- `h265_sample_entry_from_annexb` のシグネチャから `width` / `height` 引数が削除され、内部が `H265AnnexBNalUnits` を 1 回走査して VPS / SPS / PPS のみ `nalu.data.to_vec()` で抽出 → `h265_sample_entry_from_vps_sps_pps_lists` 呼び出し → `SampleEntry` のみ返す薄いラッパーになっている。

### §2.5 `H265AnnexBNalUnits` 新設

- `src/video/h265.rs` に `H265AnnexBNalUnits` / `H265NalUnit` が追加され、`Iterator<Item = Result<H265NalUnit>>` を実装している。
- start code 検出 (`[0, 0, 0, 1]` / `[0, 0, 1]`) / forbidden_zero_bit 検査 / NAL タイプ抽出 (`(byte >> 1) & 0x3F`) を担う。
- `h265_sample_entry_from_annexb` 内の手書き Annex-B 走査が削除され、`H265AnnexBNalUnits` の利用に置き換わっている。

### §3 `parse_hevc_sps` 内部関数追加

- `src/video/h265.rs` に `fn parse_hevc_sps(sps: &[u8]) -> Result<HevcSpsParams>` (非 pub) と `HevcSpsParams` (非 pub のモジュール内 struct) が追加されている。
- `parse_hevc_sps` の入力契約は NAL ヘッダ 2 バイト含む raw NAL バイト列で、内部で RBSP 抽出 (NAL ヘッダ 2 バイト skip + emulation prevention byte 除去) と NAL タイプ検査 (`(nalu[0] >> 1) & 0x3F == H265_NALU_TYPE_SPS`) を実施する。
- `parse_hevc_sps` 内で次の Err 化が実装されている (closed/0044 の H.264 SPS パーサと同方針の堅牢性):
  - `chroma_format_idc > 3` → `Err`
  - `bit_depth_luma_minus8 > 7` → `Err`
  - `bit_depth_chroma_minus8 > 7` → `Err`
  - `sps_max_sub_layers_minus1 > 6` → `Err`
  - `general_profile_idc` が許容リスト `{1, 2, 3, 4, 5, 6, 7, 9}` (Main / Main 10 / Main Still Picture / Format Range Extensions / High Throughput / Multiview Main / Scalable Main / Screen Content Coding) の外 → `Err`
  - `width == 0 || height == 0` (cropping 適用後) → `Err`
  - `width > u16::MAX || height > u16::MAX` → `Err`
- エラーメッセージは仕様参照のみで記述する (issue 番号や他 issue 由来の比喩を持ち込まない)。

### ステップ 0: `BitReader` 共有化

- `src/video/bit_reader.rs` (新設) に `BitReader` 構造体 (旧 `H264BitReader`) が移動・リネームされている。
- `src/video/h264.rs::parse_sps` および `src/video/h265.rs::parse_hevc_sps` の両方から `use crate::video::bit_reader::BitReader;` で参照する。
- H.264 経路の既存テスト / PBT が全て pass する (非破壊的リファクタリング)。

### 呼び出し側追従

- `src/encoder/video_toolbox.rs::handle_encoded` の H.265 経路で `h265::h265_sample_entry` 呼び出しが消え、`h265_sample_entry_from_vps_sps_pps_lists` 呼び出しに置き換わっている。
- 同上経路で空 VPS / SPS / PPS の frame に対するサンプルエントリー構築 skip ガードが追加されている (H.264 経路と対称)。
- `src/encoder/nvcodec.rs::new_h265` で `h265_sample_entry_from_annexb(&seq_params, options.frame_rate)` (新シグネチャ) を呼ぶ。

### テストフィクスチャ追従

- `src/codec_string.rs::tests::video_codec_string_from_hvc1_sample_entry` / `from_sample_entries_hvc1_aac` の fixture が `h265_sample_entry_from_vps_sps_pps_lists` の出力 (Main プロファイル / Level 3.1 等の SPS 由来実値ベース) に追従している。fixture コメントは新関数名 `h265_sample_entry_from_vps_sps_pps_lists` を反映している。緩い assert (`codec_str.starts_with("hvc1.")` / `codec_str.starts_with("hvc1.1.")`) は変更不要で pass する。

### 関数削除

- 旧 `src/video/h265.rs::h265_sample_entry` 関数が削除されている。
- 関連コメント (「Sora の録画ファイルに合わせた値」「色空間 (4:2:0)」「kVTProfileLevel_HEVC_Main_AutoLevel に対応する値」「8 ビット深度」) が削除または更新されている。
- `src/encoder/video_toolbox.rs` の `h265::h265_sample_entry` の use 行が削除されている。

### テスト追加

- 新ヘルパー関数と SPS パーサの単体テストが `src/video/h265.rs::tests` モジュール (新規) に追加されている。closed/0043 の H.264 経路と対称に、合成 SPS ビルダー `HevcSpsBuilder` と実機 SPS バイト列 `pub(crate) const HEVC_SPS_*` の両方を整備する (Err 境界テストでは合成 SPS ビルダーが必須)。
- 最低限以下のテストケースが追加されている:
  - Main プロファイル (`general_profile_idc = 1`) で hvcC の各フィールド (`general_profile_idc` / `general_level_idc` / `general_profile_compatibility_flags` / `general_constraint_indicator_flags` / `chroma_format_idc` / `bit_depth_*_minus8` / `num_temporal_layers` / `temporal_id_nested`) が SPS 由来値を反映
  - Main 10 プロファイル (`general_profile_idc = 2` + `bit_depth_luma_minus8 = 2`) で hvcC の `bit_depth_luma_minus8` が 2 を反映
  - `general_constraint_indicator_flags` の正確性 (代表値 `0xb00000000000` 等を SPS RBSP に埋め込んで `HvccBox.general_constraint_indicator_flags.get() == 0xb00000000000` を assert)
  - `vps_list` 空 → `Err("missing H.265 VPS")`
  - `sps_list` 空 → `Err("missing H.265 SPS")`
  - `pps_list` 空 → `Err("missing H.265 PPS")`
  - `vps_list[i]` の NAL タイプが VPS でない → Err / `sps_list[i]` の NAL タイプが SPS でない → Err / `pps_list[i]` の NAL タイプが PPS でない → Err (各 list の全要素検査)
  - `parse_hevc_sps` 内 Err 化境界値テスト (合成 SPS ビルダー必須): `chroma_format_idc = 4` / `bit_depth_luma_minus8 = 8` / `bit_depth_chroma_minus8 = 8` / `sps_max_sub_layers_minus1 = 7` / `general_profile_idc` 許容リスト外値 (例: 10)
  - 戻り値タプルの `VideoFrameSize` が SPS の cropping 適用後の値と一致 (代表値: 1920x1088 raw + crop で 1920x1080、Main プロファイルで実機 x265 が出すパターン)
- `H265AnnexBNalUnits` の単体テスト (start code 形式 `[0, 0, 0, 1]` / `[0, 0, 1]` / `forbidden_zero_bit != 0` で Err / NAL タイプ抽出 `(byte >> 1) & 0x3F`) も追加されている。

### 既存テスト維持

- `src/codec_string.rs::tests` の H.265 関連テスト (`video_codec_string_from_hvc1_sample_entry` / `from_sample_entries_hvc1_aac`) が fixture 追従後に pass する。
- nvcodec / video_toolbox 経路の既存統合テスト (もしあれば) が pass する (実機依存テストは CI スコープ外)。

### CI / feature gate

以下が pass する。feature gate ごとに分けて確認する:

- デフォルト build: `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test && cargo fmt --all -- --check`
- `fdk-aac` feature: `cargo check --features fdk-aac && cargo clippy --features fdk-aac --all-targets -- --deny warnings`
- `nvcodec` feature (CUDA SDK 利用可能環境): `cargo check --features nvcodec && cargo clippy --features nvcodec --all-targets -- --deny warnings && cargo test --features nvcodec`
- macOS 限定 `shiguredo_video_toolbox` 経路 (cfg dependency): macOS 上で `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test`

### CHANGES.md

本 issue は **リリース済み機能** (video_toolbox H.265 encoder / nvcodec H.265 encoder は 2025.2.0 以降 release 済み機能) の hvcC 内部実値化と空 VPS / SPS / PPS skip ガード追加を伴うため、`## develop` 内に `[UPDATE]` で記載する。closed/0043 (`## develop` 内未リリースの HLS / RTSP / SRT inbound) とは状況が異なる点に注意。

ただし `## develop` 内には先行 CHANGE `出力 MP4 ファイルが H.265 ストリームを含む場合は hvc1 ボックスを使用する` (hev1 → hvc1 切り替え) が既に存在し、現 develop の `h265_sample_entry` は `SampleEntry::Hvc1` のみを構築する。本 issue の改修は `SampleEntry::Hvc1` 経路にのみ影響するため、CHANGES.md エントリは「hvc1 経路の hvcC フィールド実値化」として記載する。

`shiguredo-changelog` スキルで規約を確認した上で、本文ドラフト:

```
- [UPDATE] H.265 ストリームの hvcC ボックスの各フィールドを SPS / VPS 由来の実値で埋めるように変更する
  - Video Toolbox / NVIDIA Video Codec の H.265 エンコード経路で生成される MP4 ファイル (hvc1 ボックス) に影響する
  - 現状は Sora 録画前提の固定値 (`general_profile_idc: 1` / `general_level_idc: 123` / `chroma_format_idc: 1` / `bit_depth_*_minus8: 0` 等) で埋めていたが、SPS / VPS から取り出した実値に置き換える
  - HLS マニフェスト / MPD マニフェストの H.265 codec_string も追従して SPS / VPS 由来実値ベースに変わる (`build_hevc_codec_string` 経由)
  - Video Toolbox の H.265 エンコード経路で、空 VPS / SPS / PPS の frame に対するサンプルエントリー構築をスキップする挙動が追加される (H.264 経路と対称化、NVIDIA Video Codec 経路の挙動は変わらない)
  - @sile
```

タイトル文面・段落数は実装着手時に `shiguredo-changelog` スキルの規約を再確認して微調整する。本文ドラフトの「@sile」は CHANGES.md の作業者欄慣習に従う。

## 関連

- closed/0043: H.264 経路で同型のリファクタを実施した前提 issue。本 issue は H.265 経路への横展開。新ヘルパー関数のシグネチャ・タプル戻り値・空 list 検査・破壊的シグネチャ変更を踏襲する。
- closed/0044: H.264 SPS パーサの堅牢性補強 (`pic_order_cnt_type` 仕様外値 Err 化)。本 issue で H.265 SPS パーサを新規追加する際に、同様の堅牢性補強 (`chroma_format_idc > 3` 等) を初日から組み込む。
- closed/0047: WebM リーダーの AV1 / H264AnnexB sample_entry 構築。本 issue で AV1 経路の固定値解消を「将来別 issue」として予告 (closed/0047 から本 issue を AV1 予告先として参照済み)。
- closed/0050: RTMP H.264 経路の `avc_sequence_header_to_sample_entry` を `h264_sample_entry_from_sps_pps_lists` の薄いラッパーに統合した先行 issue。本 issue でも薄いラッパー化・タプル戻り値・空 list 検査・実装コードへの issue 番号埋め込み禁止規約を踏襲する。closed/0050 の `### 残懸念` で予告された「issue 0048 への双方向リンク追加」を本 issue 関連節への追加で対応する。
- open/0049 (polish 済み): `prop_h264_sps` の PBT を構造化 Strategy に置き換える issue。H.265 SPS PBT は 0049 完了後に別 issue で対応する。
- 「将来別 issue」候補一覧は `### 将来別 issue` 節を参照。

## 解決方法

推奨パッチ順序の 6 ステップを踏みつつ、`/review-diff-code` で指摘された致命的・重要・改善を順次反映する形で対応した。

### 推奨パッチ順序の実装 (6 ステップ)

1. **BitReader 共有化**: `src/video/h264.rs::H264BitReader` を `src/video/bit_reader.rs::BitReader` に切り出し、codec 中立名にリネームした。エラーメッセージから H.264 固有の語彙を外し `bit reader: ...` プレフィックスに統一した。`skip_scaling_list` 等の H.264 固有ロジックは h264.rs に残した。
2. **H265AnnexBNalUnits 追加**: H.264 経路の `H264AnnexBNalUnits` と対称の汎用イテレーターを新設し、`forbidden_zero_bit` 検査と `(byte >> 1) & 0x3F` で nal_unit_type を抽出する形にした。既存 `h265_sample_entry_from_annexb` 内の手書き Annex-B 走査を置き換えた。
3. **parse_hevc_sps + HevcSpsBuilder 追加**: `parse_hevc_sps` / `HevcSpsParams` / `rbsp_from_hevc_sps_nalu` / `chroma_subsampling_factors` を `src/video/h265.rs` に追加し、`H265_ALLOWED_PROFILE_IDCS` (`{1, 2, 3, 4, 5, 6, 7, 9}`) / `chroma_format_idc > 3` / `bit_depth_*_minus8 > 7` / `sps_max_sub_layers_minus1 > 6` / 解像度 0 / `u16::MAX` 超の Err 化を初日から組み込んだ。テストモジュールに合成 SPS ビルダー `HevcSpsBuilder` を追加し、`parse_hevc_sps` の正常系・Err 経路を網羅した。
4. **新ヘルパー関数 + 全 2 呼び出し側追従**: `h265_sample_entry_from_vps_sps_pps_lists(vps_list, sps_list, pps_list, fps) -> Result<(SampleEntry, VideoFrameSize)>` を新設し、`HvccBox` の各フィールドを SPS / VPS 由来実値で埋めるようにした。`h265_sample_entry_from_annexb` を破壊的シグネチャ変更 (`fn(data: &[u8], fps: FrameRate) -> Result<SampleEntry>`) の薄いラッパーに変更した。`src/encoder/video_toolbox.rs::handle_encoded` の H.265 経路を新ヘルパー呼び出しに切り替え、空 VPS / SPS / PPS frame の skip ガードを追加した (H.264 経路と対称)。`src/encoder/nvcodec.rs::new_h265` を新シグネチャに追従させた。`src/codec_string.rs::tests` の H.265 fixture 2 箇所を Main プロファイル + Level 3.1 / Single layer ベース (`general_level_idc: 123 → 93`, `num_temporal_layers: 0 → 1`, `temporal_id_nested: 0 → 1`) に更新した。
5. **dead code 削除**: 旧 `h265_sample_entry` 関数と Sora 録画固定値コメント (「Sora の録画ファイルに合わせた値」「色空間 (4:2:0)」「kVTProfileLevel_HEVC_Main_AutoLevel に対応する値」「8 ビット深度」) を削除した。`src/video/h265.rs` の `use crate::types::EvenUsize;` も削除した。

### レビュー指摘の反映 (致命的)

- **CHANGES.md エントリ追加**: `## develop` 内に `[UPDATE]` で「H.265 エンコード時の hvcC ボックスを SPS / VPS 由来実値で埋めるように変更する」を追記した。
- **新ヘルパー関数の単体テスト 11 件追加**: `h265_sample_entry_from_vps_sps_pps_lists` に対する空 list Err 3 件 / 各 list 全要素 NAL タイプ検査 Err 3 件 / Main hvcC マッピング / Main 10 bit_depth / 複数 NAL 順序保持 / cropping VideoFrameSize / Annex-B 薄ラッパー統合の各テストを追加した。
- **実機 HEVC SPS バイト列定数追加**: `HEVC_SPS_640X480` (Main / Level-3, emulation prevention byte 5 個) と `HEVC_SPS_1920X1080` (Main / Level-4, conformance window 経路, emulation prevention byte 4 個) を `pub(crate) const` で追加し、`parse_hevc_sps` と `h265_sample_entry_from_vps_sps_pps_lists` の結合担保テスト 3 件を追加した。

### レビュー指摘の反映 (重要・改善)

- Annex-B イテレーターの末尾 3 バイト start code 検出漏れを H.264 / H.265 両経路で修正した (共通ヘルパー `find_next_annexb_start_code` を h264.rs に新設して共有)。
- `sub_layer_present_flags` を `Vec` から `[(u8, u8); 6]` 固定配列に置き換えた (parse_hevc_sps で 0..=6 に制限済みのため heap 確保不要)。
- `H265NalUnit` に docstring を追加して入力契約 (NAL ヘッダ 2 バイト含む EBSP 形式) を明示した。
- `BitReader` の単体テスト 5 件を h264.rs から bit_reader.rs に移動した (共有モジュールに責務を集約)。
- `general_profile_idc` 許容リストの穴 (`profile_idc=8`) と全許容値 8 個 (`{1, 2, 3, 4, 5, 6, 7, 9}`) の境界テストを追加した。
- `chroma_subsampling_factors` の Table 6-1 マッピング (4 ケース + `separate_colour_plane_flag=1` の特例) 単体テストを追加した。
- `H265AnnexBNalUnits` の 3 バイト / 4 バイト start code 混在パターンのテストを追加した。
- `HevcSpsBuilder::build()` の NAL ヘッダハードコードを `SPS_HEADER` 定数経由に統一した (DRY)。
- `NalUnitArray` 型エイリアスを削除して `Vec<Vec<u8>>` 直書きに揃えた (H.264 経路との対称性回復)。
- `parse_hevc_sps` 内の 3 箇所で繰り返されていた ue(v) 値域検査 + u8 キャストのパターンを `read_ue_as_u8_bounded` ヘルパー関数に共通化した。
- `chroma_subsampling_factors::unreachable!()` の長文コメントを自明として削った。
- 冗長な `sps_list[0].as_slice()` を `&sps_list[0]` に簡略化した。
- `SPS VUI / PPS 由来抽出` のコメントから issue 由来表現を除き中立表現に書き換えた。

### CHANGES.md

`## develop` 内 `[UPDATE]` で記載した (リリース済み機能の hvcC 内部実値化と空 VPS / SPS / PPS skip ガード追加を伴うため)。本文ドラフトを issue で提示した内容を簡潔化し、タイトル 1 行 + `@sile` の最小エントリにした。

### 副次的な外部観測可能挙動変化

- `HvccBox` の各フィールド (`general_profile_idc` / `general_level_idc` / `general_profile_compatibility_flags` / `general_constraint_indicator_flags` / `chroma_format_idc` / `bit_depth_*_minus8` / `num_temporal_layers` / `temporal_id_nested`) が Sora 録画固定値 → SPS / VPS 由来実値に変わる。
- `build_hevc_codec_string` 経由で生成される H.265 codec_string (HLS / MPEG-DASH マニフェスト含む) が SPS / VPS 由来実値ベースに変わる。
- `src/encoder/video_toolbox.rs::handle_encoded` の H.265 経路で、空 VPS / SPS / PPS の frame に対するサンプルエントリー構築をスキップする挙動が追加される (H.264 経路と対称化、nvcodec 経路の挙動は変わらない)。
- `parse_hevc_sps` 内の Err 化拡張で、仕様値域外 SPS (`chroma_format_idc > 3` / `bit_depth_*_minus8 > 7` / `sps_max_sub_layers_minus1 > 6` / `general_profile_idc` 許容リスト外 / 解像度 0 / `u16::MAX` 超) は SPS 受信時点で Err になり、上位 fail-fast 経路に伝播する。

### 残懸念 (別 issue 起票候補)

- **`src/rtsp/subscriber.rs::BitReader` の統合**: `src/video/bit_reader.rs::BitReader` と同名・同等機能の独自実装が subscriber 側に残っており、`BitReader` という汎用名前空間を 2 箇所が確保している。`open/0058` として起票済み。
- **`NALU_HEADER_LENGTH` の横方向依存**: `src/video/h265.rs` で `pub use crate::video::h264::NALU_HEADER_LENGTH;` として H.264 モジュールから再エクスポートしている。実害ゼロのため起票しないと判断したが、broken window として残置。
- **`parse_hevc_sps` の関数分割**: 約 180 行のフラット関数。H.264 経路 (`parse_sps`) のような `read_high_profile_sps_fields` 相当のヘルパーへの切り出し余地がある。現状のテスト網羅で品質は担保されているため起票しないと判断したが、将来 H.265 仕様拡張時に再評価候補。
- **AV1 経路 (`src/video/av1.rs::av1_sample_entry`) の Hisui 固定値解消**: closed/0047 が本 issue を AV1 固定値解消の予告先として参照済み。将来別 issue として起票候補。
- **`avg_frame_rate` の ISO/IEC 14496-15 §8.3.3.1 仕様 (256 倍単位) への準拠**: 現状の `(fps.numerator.get().div_ceil(fps.denumerator.get())) as u16` は仕様の `frames in 256 seconds` と 256 倍ずれている。将来別 issue として起票候補。
- **`min_spatial_segmentation_idc` / `parallelism_type` の SPS VUI / PPS 由来実値抽出**: 本 issue では固定値 0 を維持。実装範囲が広がるため将来別 issue として起票候補。
- **H.265 SPS パーサの PBT 追加**: closed/0049 の構造化 Strategy 完了済み。本 issue では新設しない判断としたが、H.264 経路と対称な PBT 追加は将来別 issue として起票候補。
- **video_toolbox H.265 経路の 3 連 `clone`**: `frame.vps_list.clone()` 等は `std::mem::take` で move 可能。ただし H.264 経路も同じパターンのため、対称化を保つには両経路を一括対応する別 issue として起票候補。
