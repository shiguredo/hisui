# h265_sample_entry を VPS / SPS / PPS リスト受け取り版にリファクタして NAL 走査の二重化と hvcC フィールドの固定値を解消する

- Priority: Low
- Created: 2026-06-19
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-h265-sample-entry-from-vps-sps-pps-lists
- Polished: {YYYY-MM-DD}

## 目的

`src/video/h265.rs::h265_sample_entry` および `h265_sample_entry_from_annexb` は、issue 0043 で H.264 経路を整理する前の `h264_sample_entry` / `h264_sample_entry_from_annexb` と同じ構造を持つ broken window を抱えている。

1. **NAL 走査の二重化 (推定)**: `h265_sample_entry_from_annexb` は内部で Annex-B バイト列を走査して VPS / SPS / PPS を抽出するが、呼び出し側がすでに NAL リストを持つ経路 (encoder 系) でも同じ Annex-B 経路を通すと走査が二重化する。
2. **hvcC ヘッダーフィールドの固定値**: `h265_sample_entry` は次のフィールドを Sora 録画前提の固定値で埋めている (コメントに「Sora の録画ファイルに合わせた値（必要に応じて調整すること）」と明記):
   - `general_profile_compatibility_flags: 0x60000000`
   - `general_constraint_indicator_flags: 0xb00000000000`
   - `general_level_idc: 123`
   - `general_profile_space: 0`
   - `general_tier_flag: 0`
   - `general_profile_idc: 1` (Main 固定)
   - `chroma_format_idc: 1` (4:2:0 固定)
   - `bit_depth_luma_minus8: 0` / `bit_depth_chroma_minus8: 0` (8 bit 固定)
3. **encoder 経路でのシグネチャ非対称**: `src/encoder/video_toolbox.rs::handle_encoded` 内で H.264 経路は issue 0043 後に `h264_sample_entry_from_sps_pps_lists(sps_list, pps_list)` を呼ぶが、H.265 経路は `h265::h265_sample_entry(width, height, fps, vps_list, sps_list, pps_list)` を呼ぶ。同じ関数内で並列に呼ばれる 2 つの sample_entry 構築のシグネチャが大きく異なる。

issue 0043 の H.264 経路リファクタを H.265 経路にも適用し、対称性を回復する。

## 優先度根拠

Low。issue 0043 と同等の判断:

- 主目的は内部効率化 (NAL 走査 1 回化) と broken window 解消 (Sora 録画前提の固定値 / encoder シグネチャ非対称)。
- 副次的に外部観測可能な挙動変化が発生する可能性が高い (hvcC ヘッダーが H.265 仕様 + ISO/IEC 14496-15 仕様の VPS / SPS 由来の実値に揃う方向の修正で、下流プレイヤー / ツールの互換性に対する影響は中立から改善寄り)。
- 実害は発生していないため (Sora 録画固定値で Main プロファイル 8 bit 4:2:0 のストリームを想定どおりに出せている) Low を維持。

## 現状

行番号は実装着手時に grep で再特定する。

### 改修対象

- `src/video/h265.rs::h265_sample_entry(width, height, fps, vps_list, sps_list, pps_list)`: hvcC の固定値多数 (上記目的節参照)。
- `src/video/h265.rs::h265_sample_entry_from_annexb(width, height, fps, data)`: Annex-B バイト列から VPS / SPS / PPS を走査して抽出し、`h265_sample_entry` を呼ぶ薄いラッパー。
- `src/encoder/video_toolbox.rs::VideoToolboxEncoder::handle_encoded`: H.265 経路で `h265::h265_sample_entry(self.width, self.height, self.fps, frame.vps_list.clone(), frame.sps_list.clone(), frame.pps_list.clone())` を呼ぶ。
- `src/encoder/nvcodec.rs::NvcodecEncoder::new_h265`: `h265::h265_sample_entry_from_annexb(width, height, options.frame_rate, &seq_params)` を呼ぶ。
- 必要に応じて `src/decoder/` の H.265 経路、その他テストフィクスチャ。

### 既存の VPS / SPS パーサの有無

H.264 経路は issue 0043 で `parse_sps` (内部関数) を整備済み。H.265 では VPS / SPS パーサが既存にあるかどうかは実装着手時に確認する。無ければ新規追加する。

## 設計方針 (案)

issue 0043 と同じ 3 段構えにする。

### §1 `h265_sample_entry_from_vps_sps_pps_lists` (新ヘルパー関数、pub)

```rust
pub fn h265_sample_entry_from_vps_sps_pps_lists(
    vps_list: Vec<Vec<u8>>,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
    fps: FrameRate,
) -> crate::Result<(SampleEntry, VideoFrameSize)>
```

- 戻り値タプル: `SampleEntry` + `VideoFrameSize` (SPS 由来の解像度)。issue 0043 の H.264 経路と同じ構造。
- 内部で SPS をパースして hvcC の各フィールド (`general_profile_idc` / `general_level_idc` / `chroma_format_idc` / `bit_depth_*_minus8` / `general_profile_compatibility_flags` / `general_constraint_indicator_flags` / `general_tier_flag` / 解像度) を SPS / VPS 由来の実値に置き換える。
- 入力契約は ISO/IEC 14496-15 §8.3.3.1 で定義された EBSP 形式 (NAL ヘッダ 2 バイト含む、start code なし) に揃える。
- `vps_list[0]` / `sps_list[0]` / `pps_list[0]` がそれぞれ VPS / SPS / PPS NAL タイプであることを検査する (issue 0043 で追加した防御的検査と同方針)。

### §2 `h265_sample_entry_from_annexb` (薄いラッパー、破壊的シグネチャ変更)

```rust
pub fn h265_sample_entry_from_annexb(data: &[u8], fps: FrameRate) -> crate::Result<SampleEntry>
```

- 内部で Annex-B 走査して VPS / SPS / PPS を抽出し、`h265_sample_entry_from_vps_sps_pps_lists` を呼ぶ。
- 引数 `width` / `height` を削除する (破壊的変更)。SPS から実値を取り出すため。
- 全呼び出し側 (nvcodec encoder) を新シグネチャに追従する。

### §3 H.265 SPS / VPS パーサ

H.265 SPS パーサが既存にあるかは実装着手時に確認。無ければ追加する。

- ITU-T H.265 (HEVC) 仕様 7.3.2.2 (Sequence Parameter Set RBSP syntax) に従い、profile_tier_level / chroma_format_idc / bit_depth_*_minus8 / pic_width_in_luma_samples / pic_height_in_luma_samples / conformance_window_flag と cropping を読み取る。
- VPS は profile_tier_level の共有部分 (`vps_max_layers_minus1` 等) を持つが、Hisui の入力前提では SPS のみで hvcC に必要な値を構築できる可能性が高い (要確認)。

### encoder 経路の追従

- `video_toolbox.rs::handle_encoded`: H.265 経路で `h265_sample_entry_from_vps_sps_pps_lists(frame.vps_list.clone(), frame.sps_list.clone(), frame.pps_list.clone(), self.fps)` を呼ぶ。戻り値タプルの `VideoFrameSize` は H.264 経路と同じく捨て、`VideoFrame.size` は encoder 設定値を維持。空 VPS / SPS / PPS のときはサンプルエントリー構築をスキップする (H.264 経路と対称、issue 0043 の致命的-1 修正と同方針)。
- `nvcodec.rs::new_h265`: 薄いラッパー `h265_sample_entry_from_annexb(&seq_params, options.frame_rate)` を引き続き呼ぶ。

### スコープ外

- H.265 decoder 経路 (`src/decoder/` 配下) はスコープ外。issue 0043 と同じく decoder の sample_entry 構築は別経路。
- AV1 経路 (`src/video/av1.rs::av1_sample_entry`) も類似の固定値構造を持つ可能性があるが、本 issue では触らない。将来別 issue。

## 完了条件

- `src/video/h265.rs` に `h265_sample_entry_from_vps_sps_pps_lists(vps_list, sps_list, pps_list, fps) -> Result<(SampleEntry, VideoFrameSize)>` が追加されている。
- `h265_sample_entry_from_annexb` のシグネチャから `width` / `height` 引数が削除され、内部が新ヘルパーの薄いラッパーになっている。
- `h265_sample_entry` 単独関数は削除され、新ヘルパーに統合されている。
- hvcC の以下フィールドが SPS / VPS 由来の実値に置き換わる:
  - `general_profile_compatibility_flags` / `general_constraint_indicator_flags` / `general_level_idc` / `general_profile_space` / `general_tier_flag` / `general_profile_idc` / `chroma_format_idc` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8`
- video_toolbox encoder と nvcodec encoder の H.265 経路が新シグネチャに追従している。
- video_toolbox encoder で H.265 経路にも空 VPS / SPS / PPS でのサンプルエントリー構築 skip ガードが追加されている (H.264 経路と対称)。
- 新ヘルパーと H.265 SPS パーサに対する単体テストが追加されている。
- 既存テストが全て pass する。
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test && cargo fmt --all -- --check` がパスする (macOS の `shiguredo_video_toolbox` cfg dependency 環境含む)。

### CHANGES.md

issue 0043 と同じく記載しない方針が妥当か実装着手時に確認する。本 issue が触る経路 (video_toolbox H.265 / nvcodec H.265) はリリース済み機能であるため、CHANGES.md 記載が必要になる可能性がある (issue 0043 の HLS / RTSP / SRT inbound が `## develop` 内未リリース機能だったのとは状況が異なる)。

## 関連

- issue 0043 (closed): H.264 経路で同型のリファクタを実施した前提 issue。本 issue は H.265 経路への横展開。
- issue 0044 (open): H.264 SPS パーサの堅牢性補強 (`pic_order_cnt_type` 仕様外値 Err 化)。H.265 SPS パーサを新規追加する場合は同様の堅牢性補強を初日から組み込む。
- issue 0047 (open): WebM リーダーの AV1 / H264AnnexB sample_entry 構築。H.265 の WebM リーダー経路もスコープ候補。
- 将来別 issue: AV1 経路 (`src/video/av1.rs::av1_sample_entry`) の固定値解消。

## 解決方法

実装着手後にここに記述する。
