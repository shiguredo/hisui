# WebM リーダーの AV1 / H264AnnexB 映像経路に sample_entry 構築を追加する

- Priority: Low
- Created: 2026-06-18
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/add-webm-reader-av1-h264-sample-entry
- Polished:

## 目的

issue 0031 で WebM リーダーに sample_entry 構築を追加したが、AV1 / H264AnnexB は WebM CodecPrivate のパーサ実装規模が大きく、暫定的に `WebmVideoReader::new` で `sample_entry: None` のまま開く形（旧版互換）に留めた。Sora 録画では AV1 / H264AnnexB の WebM 録画が普通にあるため、本 issue でこれらに正規に sample_entry を構築して `src/video.rs::VideoFrame.sample_entry` の不変条件 docstring から「現時点で未適用の経路: WebM リーダーの AV1 / H264AnnexB 映像」例外節を削除する。

## 優先度根拠

Low。compose 経路では再エンコードで sample_entry が確定するため実害ゼロ（writer 入口の `resolve_*_sample_entry` も二重防護で動く）。issue 0030 の broken window 解消の延長で、不変条件を「全 WebM 経路」と言い切れる状態にする位置づけ。

## 現状

`src/webm/reader.rs::WebmVideoReader::new` の match で AV1 / H264AnnexB は `=> None`（issue 0031 で暫定対応）。`src/video.rs::VideoFrame.sample_entry` の docstring に「現時点で未適用の経路: WebM リーダーの AV1 / H264AnnexB 映像」例外節が残る。

既存ヘルパ:

- `src/video/av1.rs::av1_sample_entry(width: EvenUsize, height: EvenUsize, config_obus: &[u8]) -> SampleEntry`
- `src/video/h264.rs::h264_sample_entry_from_annexb(width: usize, height: usize, data: &[u8]) -> crate::Result<SampleEntry>`（Annex-B 入力）

WebM CodecPrivate のフォーマット（Matroska Codec Mappings 参照）:

- V_AV1: AV1CodecConfigurationRecord（先頭 4 バイトヘッダ + configOBUs。AOM Codecs ISO Media File Format Binding §2.3）
- V_MPEG4/ISO/AVC: AVCDecoderConfigurationRecord (avcC、ISO/IEC 14496-15 §5.2.4.1)

## 設計方針

### 1. AV1 経路

WebM CodecPrivate (AV1CodecConfigurationRecord) の先頭 4 バイトヘッダ（marker / version / seq_profile / seq_level_idx_0 / seq_tier_0 / high_bitdepth / twelve_bit / monochrome / chroma_subsampling_* / chroma_sample_position / initial_presentation_delay_present 等）の後ろにある configOBUs（Sequence Header OBU を含む OBU 列）を抽出して `av1_sample_entry(width: EvenUsize, height: EvenUsize, config_obus: &[u8])` に渡す。

`VideoTrackHeader.width` / `height` は `usize` のため `EvenUsize::truncating_new` 等で変換する。`width == 0 || height == 0` 時の扱い（`EvenUsize::ZERO` でフォールバックするか `Err` を返すか）は polish-issue で確定する。

`parse_av1_config_obus(data: &[u8]) -> crate::Result<&[u8]>`（先頭 4 バイト以降を返すヘルパ）を新設する。配置は `src/video/av1.rs` か `src/webm/reader.rs` のどちらにするか polish-issue で確定する（既存の av1 関連を `src/video/av1.rs` に集約する方針なら前者）。

### 2. H264AnnexB 経路

WebM CodecPrivate (AVCDecoderConfigurationRecord / avcC) から SPS / PPS を抽出して Annex-B 形式に変換する。avcC の構造:

```
configurationVersion (1)
AVCProfileIndication (1)
profile_compatibility (1)
AVCLevelIndication (1)
lengthSizeMinusOne (1) (下位 2 ビット)
numOfSequenceParameterSets (1) (下位 5 ビット)
SPS list: each 2 バイト長 + SPS NAL bytes
numOfPictureParameterSets (1)
PPS list: each 2 バイト長 + PPS NAL bytes
```

抽出した SPS / PPS を Annex-B 形式（`0x00 0x00 0x00 0x01` start code prefix + NAL bytes）に連結して `h264_sample_entry_from_annexb(width, height, data)` に渡す。または avcC 直接受理関数 `h264_sample_entry_from_avcc(width, height, data)` を新設する案も polish-issue で検討する。

`src/video/h264.rs` への配置を想定するが、issue 0043（`h264-sample-entry-from-sps-pps-lists` refactor）と関連する可能性があるため polish 時に整合を取る。

### 3. `WebmVideoReader::new` の match 更新

```rust
VideoFormat::Av1 => Some(SharedSampleEntry::new(av1_sample_entry(...))),
VideoFormat::H264AnnexB => Some(SharedSampleEntry::new(h264_sample_entry_from_avcc(...)?)),
```

CodecPrivate を `WebmVideoReader::new` で取得する必要があるため、`VideoTrackHeader::read` を拡張して既存の `codec` / `width` / `height` に加えて `codec_private: Vec<u8>` を持たせる（VP8 / VP9 では空 `Vec` でも可）。

### 4. 不変条件 docstring の例外節削除

`src/video.rs::VideoFrame.sample_entry` の docstring から「現時点で未適用の経路: WebM リーダーの AV1 / H264AnnexB 映像」例外節を削除する。issue 0031 と同じ完了条件パターン。

## 完了条件

- AV1 / H264AnnexB の WebM が `WebmVideoReader::new` で `sample_entry: Some(...)` を構築すること
- `src/video.rs` の docstring から「現時点で未適用の経路: WebM リーダーの AV1 / H264AnnexB 映像」例外節が削除されること
- 既存テスト + 新規追加テスト（AV1 / H264AnnexB の sample_entry 構築検証）が通ること
- compose 経路で AV1 / H264AnnexB の WebM 録画にリグレッションが無いこと
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `fdk-aac` / `nvcodec` / `video_toolbox` を含む）

### CHANGES.md

記載しない（内部リファクタ・公開 API 変化なし・利用者挙動変化なし）。0017 / 0027 / 0030 / 0031 と同方針。

## 関連

- issue 0031（直接の前提。WebM リーダーに sample_entry 構築を追加した）
- issue 0030（不変条件起点）
- issue 0034（writer 入口の違反検知 + fallback）
- issue 0043（h264_sample_entry_from_sps_pps_lists の refactor。設計方針 2 で配置・流用関数を確定する際に整合を取る）
