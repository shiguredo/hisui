# RTMP 経路の avc_sequence_header_to_sample_entry を h264_sample_entry_from_sps_pps_lists に統合して chroma_format / bit_depth_* の固定値を解消する

- Priority: Low
- Created: 2026-06-19
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-rtmp-avc-sequence-header-from-sps-pps-lists
- Polished: {YYYY-MM-DD}

## 目的

`src/rtmp/frame.rs::avc_sequence_header_to_sample_entry` は、issue 0043 で H.264 経路の sample_entry 構築が `h264_sample_entry_from_sps_pps_lists` に統一された後も、独自に Avc1Box を組み立てている。固定値で残った broken window:

- `chroma_format: None`
- `bit_depth_luma_minus8: None`
- `bit_depth_chroma_minus8: None`
- `sps_ext_list: Vec::new()` (Hisui の入力前提では妥当)

issue 0043 (closed) で「スコープ外の関連経路」として「将来別 issue で対応」と明示済み。

本 issue では `avc_sequence_header_to_sample_entry` を `h264_sample_entry_from_sps_pps_lists` の薄いラッパーに置き換えるか、内部で SPS 由来の `chroma_format` / `bit_depth_*` を埋める形にして、broken window を解消する。

## 優先度根拠

Low。issue 0043 と同等の判断:

- 主目的は内部実装の二重化解消 (Avc1Box 組み立てロジックを `h264_sample_entry_from_sps_pps_lists` に一本化)。
- 副次的に hvcC ではなく avcC の `chroma_format` / `bit_depth_*` が High 系プロファイル時に SPS 由来の実値で埋まる方向の修正。RTMP 経由で受け取る H.264 ストリームが High 系プロファイルの場合に下流の MP4 互換性が改善する。
- 実害は発生していない (現状は Baseline / Main / Extended ストリームが多いため `None` 固定で動いている) ため Low。

## 現状

```rust
// src/rtmp/frame.rs
fn avc_sequence_header_to_sample_entry(
    seq_header: &shiguredo_rtmp::AvcSequenceHeader,
    width: usize,
    height: usize,
) -> crate::Result<SampleEntry> {
    use shiguredo_mp4::{Uint, boxes::Avc1Box, boxes::AvccBox};

    Ok(SampleEntry::Avc1(Avc1Box {
        visual: crate::video::sample_entry_visual_fields(width, height),
        avcc_box: AvccBox {
            sps_list: seq_header.sps_list.clone(),
            pps_list: seq_header.pps_list.clone(),
            avc_profile_indication: seq_header.avc_profile_indication,
            avc_level_indication: seq_header.avc_level_indication,
            profile_compatibility: seq_header.profile_compatibility,
            length_size_minus_one: Uint::new(seq_header.length_size_minus_one),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext_list: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    }))
}
```

issue 0043 後の `h264_sample_entry_from_sps_pps_lists` との差分:

- `avc_profile_indication` / `avc_level_indication` / `profile_compatibility` は `seq_header` 由来の実値で埋まっている (issue 0043 後の H.264 経路と同じ)。
- `chroma_format` / `bit_depth_*` だけが `None` 固定で残っている。
- `length_size_minus_one` は `seq_header.length_size_minus_one` 由来 (H.264 経路は `NALU_HEADER_LENGTH - 1` 固定)。
- `width` / `height` を引数で受ける (RTMP ハンドシェイク由来)。SRT / RTSP のように Annex-B から SPS を抽出する経路とは制御フローが異なる。

### issue 0043 後の `h264_sample_entry_from_sps_pps_lists` シグネチャ

```rust
pub fn h264_sample_entry_from_sps_pps_lists(
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
) -> crate::Result<(SampleEntry, VideoFrameSize)>
```

- 入力: SPS / PPS の EBSP 形式 NAL バイト列リスト (NAL ヘッダ含む、start code なし)。
- 戻り値タプルの `VideoFrameSize` は SPS 由来の cropping 適用後解像度。
- 内部で `parse_sps` を呼んで `chroma_format` / `bit_depth_*` を High 系プロファイル時に SPS 由来の実値で埋める。

### `length_size_minus_one` の取り扱い差異

`h264_sample_entry_from_sps_pps_lists` は `length_size_minus_one: Uint::new(NALU_HEADER_LENGTH as u8 - 1)` (= 3、4 バイト固定) で埋める設計。一方 RTMP の `seq_header.length_size_minus_one` は RTMP 経由で受け取る AVCDecoderConfigurationRecord 由来の値で、必ずしも 3 とは限らない。`h264_sample_entry_from_sps_pps_lists` をそのまま使うと RTMP 由来の `length_size_minus_one` が捨てられる可能性がある。

実装着手時に、RTMP の `seq_header.length_size_minus_one` が実用上常に 3 (4 バイト prefix) かを確認する。3 以外があり得るなら、`h264_sample_entry_from_sps_pps_lists` 側に `length_size_minus_one` の上書きオプションを追加するか、RTMP 経路は `parse_sps` だけ呼んで Avc1Box の組み立ては独自に行う形にする。

## 設計方針 (案)

### 案 A: `h264_sample_entry_from_sps_pps_lists` の薄いラッパー化

```rust
fn avc_sequence_header_to_sample_entry(
    seq_header: &shiguredo_rtmp::AvcSequenceHeader,
    width: usize,
    height: usize,
) -> crate::Result<SampleEntry> {
    let (entry, _frame_size) = crate::video::h264::h264_sample_entry_from_sps_pps_lists(
        seq_header.sps_list.clone(),
        seq_header.pps_list.clone(),
    )?;
    // visual を RTMP ハンドシェイク由来の width / height で上書きするか、
    // SPS 由来値 (戻り値タプルの VideoFrameSize) を採用するかを設計時に決める。
    Ok(entry)
}
```

- `length_size_minus_one` が常に 3 なら案 A で済む。
- `visual.width / .height` の上書き要否は設計時に決める (RTMP ハンドシェイク由来 vs SPS 由来)。
- `avc_profile_indication` / `avc_level_indication` / `profile_compatibility` は `seq_header` 由来も SPS 由来も実値で、通常一致するはず。差異が出る場合の挙動を確認する。

### 案 B: 内部で `parse_sps` を呼んで chroma_format / bit_depth_* だけ補完

`h264_sample_entry_from_sps_pps_lists` を使わず、現状の Avc1Box 組み立てを残しつつ `chroma_format` / `bit_depth_*` だけ `parse_sps` 由来で埋める。`length_size_minus_one` は `seq_header` 由来を維持できる。

```rust
fn avc_sequence_header_to_sample_entry(...) -> crate::Result<SampleEntry> {
    let params = crate::video::h264::parse_sps(&seq_header.sps_list[0])?; // pub(crate) 化要
    // chroma_format / bit_depth_* を params.high_profile_params から埋める
    // ...
}
```

ただし `parse_sps` は現状非 pub。`pub(crate)` 化が必要。

### 設計時の確認事項

- `seq_header.length_size_minus_one` の実用上の値範囲。
- RTMP `seq_header` 由来の profile / level と SPS パース結果の整合性 (通常一致するはず)。
- `visual.width / .height` を RTMP ハンドシェイク由来と SPS 由来のどちらにするか。

### スコープ外

- AV1 / H.265 経路の RTMP 受信 (現状サポートしていない場合)。
- RTMP 送信側 (`sample_entry_to_avc_sequence_header` 等) の整合性。本 issue は受信側のみ。

## 完了条件

- `src/rtmp/frame.rs::avc_sequence_header_to_sample_entry` が `h264_sample_entry_from_sps_pps_lists` の薄いラッパー、または `parse_sps` 経由で `chroma_format` / `bit_depth_*` を SPS 由来実値で埋める形に変更されている。
- 設計時の確認事項 (`length_size_minus_one` / profile/level 整合性 / `visual.width / .height` の扱い) が確定し、docstring または PR 本文に記録されている。
- RTMP 経路のテストが pass する。
- 既存テスト (`src/rtmp/`) への影響がない。
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test && cargo fmt --all -- --check` がパスする。

### CHANGES.md

実装着手時に判断する。RTMP 受信機能がリリース済みなら `[UPDATE]` で記載 (High 系プロファイルの avcC が SPS 由来実値に変わる)。`## develop` 内未リリースなら記載しない (issue 0043 と同方針)。

## 関連

- issue 0043 (closed): H.264 SRT / RTSP / encoder 3 経路を `h264_sample_entry_from_sps_pps_lists` に統一した前提 issue。
- issue 0048 (open): H.265 経路の同型リファクタ。
- 将来別 issue: AV1 経路の `av1_sample_entry` 固定値解消。

## 解決方法

実装着手後にここに記述する。
