# RTMP 経路の avc_sequence_header_to_sample_entry を h264_sample_entry_from_sps_pps_lists の薄いラッパーに統合する

- Priority: Low
- Created: 2026-06-19
- Completed: 2026-06-22
- Model: Opus 4.7
- Branch: feature/refactor-rtmp-avc-sequence-header-from-sps-pps-lists
- Polished: 2026-06-22

## 目的

副次的に外部観測可能な挙動修正 (`tracing::debug!` の "0x0" → SPS 由来実値、保持される avcC 固定値 → SPS 由来実値) を伴う refactor。

`src/rtmp/frame.rs::avc_sequence_header_to_sample_entry` および呼び出し側 `process_video_frame` は、issue 0043 で H.264 経路の sample_entry 構築が `h264_sample_entry_from_sps_pps_lists` に統一された後も独自に `Avc1Box` を組み立てており、次の broken window が残存している。

1. **アーキテクチャの二重化**: `Avc1Box` / `AvccBox` の組み立てロジックが `h264_sample_entry_from_sps_pps_lists` と RTMP 経路の 2 箇所に分散している (issue 0043 で SRT / RTSP / encoder 3 経路は統一済み)。
2. **avcC ヘッダーフィールドの固定値**: `chroma_format: None` / `bit_depth_luma_minus8: None` / `bit_depth_chroma_minus8: None` が固定値。`avc_profile_indication` / `profile_compatibility` / `avc_level_indication` / `length_size_minus_one` は `seq_header` 由来 (RTMP の AVCDecoderConfigurationRecord 経由) で、SPS 由来実値とは別経路。RTMP inbound は decode + re-encode 経路必須のため writer に直接渡らないが、保持される `Avc1Box` の値が SPS と乖離する状態が残る。
3. **`visual.width` / `.height` の 0 固定 + `tracing::debug!` "0x0" 出力**: `process_video_frame` で `let width = 0; let height = 0;` がハードコードされ TODO コメント付き。decode + re-encode 経路を経るため writer に渡る sample_entry はエンコーダが再構築するが、`tracing::debug!("Received H.264 sequence header: {}x{}", 0, 0)` が常に "0x0" を出力する観測ログ問題が残る。

本 issue では `avc_sequence_header_to_sample_entry` を `h264_sample_entry_from_sps_pps_lists` の薄いラッパー (案 A) に置き換え、呼び出し側 `process_video_frame` も追従させることで上記 3 件を一括解消する。

issue 0043 closed の `### 本 issue で触らない経路` セクションで RTMP 経路を「将来別 issue で対応」とした宿題を本 issue で吸収する。本 issue で `avc_sequence_header_to_sample_entry` のシグネチャから `width` / `height` 引数を削除し、`AvcSequenceHeader.sps_list` / `.pps_list` を直接 `h264_sample_entry_from_sps_pps_lists` に渡すことで、issue 0043 が「触らない」とした根拠 (引数構造の差異・入力型の差異) が同時に解消される。

## 優先度根拠

Low。主目的はアーキテクチャ統一。実害として顕在化している不具合は無い (RTMP inbound は `src/rtmp/inbound_endpoint.rs` で必ず `VideoDecoder` を経由する設計で、独自に組み立てた sample_entry は writer に直接渡らない。writer に渡る sample_entry は再エンコード時にエンコーダ側で新規構築される)。

副次的に外部観測可能な挙動変化:

- `tracing::debug!("Received H.264 sequence header: {}x{}", ...)` の出力が "0x0" → SPS 由来実値に変わる (観測ログ修正)。
- 保持される `Avc1Box.avcc_box` の `chroma_format` / `bit_depth_*` が High 系プロファイル時に `None` → `Some(SPS 由来実値)` に変わる。`avc_profile_indication` / `profile_compatibility` / `avc_level_indication` も `seq_header` 由来 → SPS 由来実値に変わる。仕様準拠 publisher では両者は一致するため、Hisui 内部で sample_entry を再利用する経路 (将来追加される可能性) に対する構造的安全性向上。
- `length_size_minus_one` が `seq_header` 由来 (0..=3) → `Uint::new(NALU_HEADER_LENGTH as u8 - 1) = Uint::new(3)` 固定に変わる (`### length_size_minus_one の扱い` 参照)。

## 現状

行番号は実装着手時に関連シンボルを grep で再特定する。

### 改修対象

| 対象                                                                    | 改修方針                                                                                                                                                              |
| ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/rtmp/frame.rs::avc_sequence_header_to_sample_entry`                | `h264_sample_entry_from_sps_pps_lists` の薄いラッパーに置き換え。`width` / `height` 引数を削除し、`seq_header.sps_list.clone()` / `.pps_list.clone()` を直接渡す。戻り値は `(SampleEntry, VideoFrameSize)` のタプルに変更。 |
| `src/rtmp/frame.rs::process_video_frame` (H.264 SequenceHeader 経路)    | `width = 0; height = 0;` ハードコードと TODO コメント 2 行を削除。改修後の `avc_sequence_header_to_sample_entry(&seq_header)` から `(SampleEntry, VideoFrameSize)` を受け取り、`tracing::debug!` の `{}x{}` を SPS 由来実値で出力する。 |

### `avc_sequence_header_to_sample_entry` の現状コード

```rust
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

### `process_video_frame` の現状コード (SequenceHeader 経路の抜粋)

```rust
if frame.avc_packet_type == Some(shiguredo_rtmp::AvcPacketType::SequenceHeader) {
    let seq_header = shiguredo_rtmp::AvcSequenceHeader::from_bytes(&frame.data)
        .map_err(|e| Error::new(format!("failed to parse AVC sequence header: {e}")))?;

    // いったん解像度は 0 扱いにしておく
    // TODO: SPS から実際の width, height を抽出
    let width = 0;
    let height = 0;

    // SampleEntry を生成
    let sample_entry = avc_sequence_header_to_sample_entry(&seq_header, width, height)?;
    self.video_sample_entry = Some(sample_entry);

    tracing::debug!("Received H.264 sequence header: {}x{}", width, height);
    return Ok(None);
}
```

### avcC フィールドの反映 (改修後)

`h264_sample_entry_from_sps_pps_lists` 内で構築される `Avc1Box.avcc_box` の各フィールドが下記のとおり埋まる。

| avcC フィールド            | 反映値                                                                                |
| -------------------------- | ------------------------------------------------------------------------------------- |
| `avc_profile_indication`   | SPS の `profile_idc`                                                                  |
| `profile_compatibility`    | SPS の `constraint_set0..5_flag + reserved_zero_2bits` (NAL ヘッダ除去後の RBSP byte[1]) |
| `avc_level_indication`     | SPS の `level_idc`                                                                    |
| `chroma_format`            | High 系プロファイル時のみ `Some(Uint::new(SPS の chroma_format_idc))`、それ以外 `None` |
| `bit_depth_luma_minus8`    | High 系プロファイル時のみ `Some(Uint::new(SPS の bit_depth_luma_minus8))`、それ以外 `None` |
| `bit_depth_chroma_minus8`  | High 系プロファイル時のみ `Some(Uint::new(SPS の bit_depth_chroma_minus8))`、それ以外 `None` |
| `length_size_minus_one`    | `Uint::new(NALU_HEADER_LENGTH as u8 - 1)` 固定 (= 3)                                  |
| `sps_ext_list`             | `Vec::new()` 固定                                                                     |
| `sps_list`                 | `seq_header.sps_list.clone()` を move                                                 |
| `pps_list`                 | `seq_header.pps_list.clone()` を move                                                 |

`seq_header.avc_profile_indication` / `profile_compatibility` / `avc_level_indication` / `length_size_minus_one` は捨てる (SPS 由来一択)。これは `parse_avcc_sps_pps_lists` が AVCDecoderConfigurationRecord の byte 1..=3 を捨てて SPS 由来一択を採っている既存方針と整合。

## 設計方針

`avc_sequence_header_to_sample_entry` を `h264_sample_entry_from_sps_pps_lists` の薄いラッパーにする。`src/rtmp/frame.rs` の use 宣言に `use crate::video::VideoFrameSize;` を追加する。

```rust
fn avc_sequence_header_to_sample_entry(
    seq_header: &shiguredo_rtmp::AvcSequenceHeader,
) -> crate::Result<(SampleEntry, VideoFrameSize)> {
    crate::video::h264::h264_sample_entry_from_sps_pps_lists(
        seq_header.sps_list.clone(),
        seq_header.pps_list.clone(),
    )
}
```

呼び出し側 `process_video_frame` (H.264 SequenceHeader 経路) の追従例:

```rust
if frame.avc_packet_type == Some(shiguredo_rtmp::AvcPacketType::SequenceHeader) {
    let seq_header = shiguredo_rtmp::AvcSequenceHeader::from_bytes(&frame.data)
        .map_err(|e| Error::new(format!("failed to parse AVC sequence header: {e}")))?;

    // SampleEntry と SPS 由来の解像度を取得
    let (sample_entry, frame_size) = avc_sequence_header_to_sample_entry(&seq_header)?;
    self.video_sample_entry = Some(sample_entry);

    tracing::debug!(
        "Received H.264 sequence header: {}x{}",
        frame_size.width,
        frame_size.height,
    );
    return Ok(None);
}
```

`VideoFrameSize` は `pub struct VideoFrameSize { pub width: usize, pub height: usize }` (`src/video.rs::VideoFrameSize`) でフィールドアクセス可能。`usize` の `Display` 実装により `tracing::debug!` の `{}` フォーマットに渡せる。

### 案 B (`parse_sps` を `pub(crate)` 化して RTMP 独自組み立てを残す案) は不採用

issue 0043 closed で `parse_sps` / `SpsParams` / `HighProfileSpsParams` をいずれも非 pub と確定済み (`### parse_sps と SpsParams (案 A 確定)`)。案 B は本 issue でこの設計境界を破り、`SpsParams` / `HighProfileSpsParams` の両型および全フィールドも `pub(crate)` 化が必要になる。`length_size_minus_one` を `seq_header` 由来で維持できる利点はあるが、Hisui の全体方針 (`### length_size_minus_one の扱い` 参照) と整合しない。

### `length_size_minus_one` の扱い

Hisui の全体方針は `NALU_HEADER_LENGTH = 4` バイト固定で、`parse_avcc_sps_pps_lists` が `length_size_minus_one != 3` を Err 化している (コメント「Hisui の MP4 出力は NALU_HEADER_LENGTH = 4 固定で、`AvccBox.length_size_minus_one` との乖離があると下流 muxer 出力後にプレイヤーが NAL を切り出せない」)。デコーダ系 (openh264 / nvcodec / video_toolbox) も NAL 長 prefix を 4 バイト固定で読む。

shiguredo_rtmp 2026.1.0-canary.6 の `AvcSequenceHeader::from_bytes` は `data[4] & 0x03` で 0..=3 のいずれの値も受け入れるが、typical publisher (OBS / ffmpeg / Sora) は 3 で運用し、shiguredo_rtmp 同梱テストフィクスチャも `length_size_minus_one: 3`。

案 A 採用時は `seq_header.length_size_minus_one` を捨てて avcC は `Uint::new(3)` 固定。3 以外の publisher は本 issue 改修の有無に関わらず現状コードでデコード失敗するため、本 issue は新規 broken window を作らない。3 以外の publisher 対応 (受信フレーム payload を 4 バイト prefix に変換する経路) は将来別 issue 候補だが現時点で起票しない。

### スコープ外

- **AV1 / H.265 RTMP 受信**: 現状 Hisui の RTMP inbound は H.264 のみ対応。
- **RTMP 送信側 (`src/rtmp/frame.rs::create_video_sequence_header`)**: 本 issue では触らない。受信側 sample_entry の変更は送信側に影響しない (`AvcSequenceHeader` に `chroma_format` / `bit_depth_*` フィールドが無く、`length_size_minus_one` も 3 固定で送信側 `convert_annexb_to_nalu` の 4 バイト prefix 生成と整合する)。
- **`VideoFrame.size` への SPS 由来値伝播**: 本 issue では `process_video_frame` の `VideoFrame.size = None` (現状コメント「RTMP inbound では payload を解析せずに H.264 を受け渡すため、フレームサイズは常に未知扱いにする」) を維持し、`tracing::debug!` への反映のみ実施。

## 完了条件

### `avc_sequence_header_to_sample_entry` の薄いラッパー化

- `src/rtmp/frame.rs` の use 宣言に `use crate::video::VideoFrameSize;` が追加されている。
- `avc_sequence_header_to_sample_entry` のシグネチャが `fn(seq_header: &shiguredo_rtmp::AvcSequenceHeader) -> Result<(SampleEntry, VideoFrameSize)>` に変更されている。
- 関数本体が `h264_sample_entry_from_sps_pps_lists(seq_header.sps_list.clone(), seq_header.pps_list.clone())` の 1 行呼び出しのみで構成され、独自の `Avc1Box` / `AvccBox` 組み立てロジックが削除されている。
- 関数の docstring が「`AvcSequenceHeader.sps_list` / `.pps_list` を `h264_sample_entry_from_sps_pps_lists` に委譲する薄いラッパー」と書き直されている。
- シグネチャ変更と呼び出し側 (`process_video_frame`) の追従は型整合上同一コミットで実施する。コミットメッセージ例 (`shiguredo-git` 80 文字以内): `0050 RTMP 受信側 avc_sequence_header_to_sample_entry を薄いラッパー化する`。
- 実装コード (本体 + コメント + docstring + エラーメッセージ + テストコメント) には issue 番号 (0043 / 0048 / 0050 等) や `closed/0043` 等の他 issue 由来表現を書かない (`shiguredo-issues` 規約。issue 番号は issue ファイル管理と git 履歴のためのもの)。

### `process_video_frame` の追従

- `process_video_frame` 内の `let width = 0; let height = 0;` と先行する TODO コメント 2 行が削除されている。直後の「`// SampleEntry を生成`」コメントは「`// SampleEntry と SPS 由来の解像度を取得`」に書き換えられている。
- `avc_sequence_header_to_sample_entry(&seq_header)` の戻り値タプルから `frame_size` を受け取り、`tracing::debug!("Received H.264 sequence header: {}x{}", frame_size.width, frame_size.height)` で SPS 由来実値を出力している。
- エラーは既存通り `?` で上位 (`RtmpInboundEndpoint` の接続ハンドラ) へ伝播し、最終的に接続切断される (fail-fast 維持)。`h264_sample_entry_from_sps_pps_lists` 内の Err 経路 (空 SPS / 空 PPS / PPS NAL タイプ不正 / `parse_sps` 内 SPS 不正) はそのまま伝播する。

### テスト追加

`src/rtmp/frame.rs` 末尾に `#[cfg(test)] mod tests` を新設する。RTMP 受信経路は本 issue 着手前時点でテストモジュールが存在しないため、本 issue で初めて `tests` モジュールを作る。

事前準備として `src/video/h264.rs::tests` 内の `const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2];` を `pub(crate) const PPS_NAL` に変更する (現状非 pub のため `src/rtmp/frame.rs::tests` から参照不可)。`SPS_320X240` は issue 0043 で `pub(crate) const` として公開済み。

テストケース (Baseline のみで本 issue 改修の主要効果を検証する。High プロファイル経路の chroma_format / bit_depth_* 反映は `h264_sample_entry_from_sps_pps_lists_maps_high_sps_to_avcc` (`src/video/h264.rs::tests`) で既に担保済みのため重複させない):

テストモジュール冒頭に必要な `use` 宣言を追加する:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::tests::{PPS_NAL, SPS_320X240};
    // ...
}
```

`shiguredo_mp4::Uint` 型名は下記テストコードでは `length_size_minus_one.get()` のメソッド呼び出しのみで `Uint::new(...)` のような型名直接参照が無いため import 不要 (clippy `unused_imports` 回避)。

テストケース 3 件:

- **Baseline SPS 反映** (関数名例: `avc_sequence_header_to_sample_entry_maps_baseline_sps_to_avcc`): フィクスチャの `seq_header.avc_profile_indication` / `profile_compatibility` / `avc_level_indication` / `length_size_minus_one` に SPS 由来実値 (66 / 0xc0 / 13 / 3) とは異なる値 (例: 0xff / 0x00 / 0xff / 0) を入れて、改修後の avcC が SPS 由来実値で埋まる (= `seq_header` 由来は捨てられる) ことを検証する。`SPS_320X240` の RBSP byte[0..3] は `0x42 / 0xc0 / 0x0d` で profile_idc=66 / constraint_set_flags=0xc0 / level_idc=13。

  ```rust
  let seq_header = shiguredo_rtmp::AvcSequenceHeader {
      sps_list: vec![SPS_320X240.to_vec()],
      pps_list: vec![PPS_NAL.to_vec()],
      avc_profile_indication: 0xff,
      profile_compatibility: 0x00,
      avc_level_indication: 0xff,
      length_size_minus_one: 0,
  };
  let (entry, frame_size) = avc_sequence_header_to_sample_entry(&seq_header).expect("Baseline");
  let SampleEntry::Avc1(b) = entry else { panic!("expected Avc1, got {entry:?}") };
  // avcC が SPS 由来実値で埋まり、seq_header 由来は捨てられている
  assert_eq!(b.avcc_box.avc_profile_indication, 66);
  assert_eq!(b.avcc_box.profile_compatibility, 0xc0);
  assert_eq!(b.avcc_box.avc_level_indication, 13);
  assert_eq!(b.avcc_box.length_size_minus_one.get(), 3);
  // Baseline プロファイルでは chroma_format / bit_depth_* は None
  assert!(b.avcc_box.chroma_format.is_none());
  assert!(b.avcc_box.bit_depth_luma_minus8.is_none());
  assert!(b.avcc_box.bit_depth_chroma_minus8.is_none());
  // sps_list / pps_list が seq_header からクローンされて AvccBox に詰められる
  assert_eq!(b.avcc_box.sps_list, vec![SPS_320X240.to_vec()]);
  assert_eq!(b.avcc_box.pps_list, vec![PPS_NAL.to_vec()]);
  // visual.width / .height が SPS 由来実値で埋まる (0 ではない)
  assert_eq!(b.visual.width, 320);
  assert_eq!(b.visual.height, 240);
  // 戻り値タプルの VideoFrameSize も SPS 由来実値
  assert_eq!(frame_size.width, 320);
  assert_eq!(frame_size.height, 240);
  ```

- **空 SPS で Err** (関数名例: `avc_sequence_header_to_sample_entry_returns_err_on_empty_sps_list`): `sps_list: vec![]` で `format!("{err:?}")` が `missing H.264 SPS` を含むことを assert する (既存 `h264_sample_entry_from_sps_pps_lists_returns_err_on_empty_sps_list` テストと同形式)。`crate::Error` は `Display` を実装しておらず `err.to_string()` は使えないため、必ず `format!("{err:?}")` (Debug 経由) を使う。
- **空 PPS で Err** (関数名例: `avc_sequence_header_to_sample_entry_returns_err_on_empty_pps_list`): 同様、`format!("{err:?}")` が `missing H.264 PPS` を含むことを assert する。

テスト関数内のコメントは日本語 (CLAUDE.md「テストはコメントを重視すること」)、`expect()` / `assert!` メッセージは英語または日本語のどちらでも可 (`src/video/h264.rs::tests` 内の既存テスト群と同形式で揃える)。テスト内コメント・メッセージ・テスト関数名に issue 番号 (0043 / 0048 / 0050 等) や他 issue 由来表現を書かない。

テスト追加はリファクタ本体とは別コミットで実施する (`PPS_NAL` の `pub(crate)` 化はテスト追加コミットに含める。リファクタ本体コミット時点では `PPS_NAL` は非 pub のまま、`src/rtmp/` 配下に既存テストがゼロのため `cargo test` 全体が pass する)。テスト追加コミット完了時点では新規追加テストも含めて pass する。コミットメッセージ例 (`shiguredo-git` 80 文字以内): `0050 RTMP 受信側 avc_sequence_header_to_sample_entry の単体テストを追加する`。

### CHANGES.md

`## develop` 内未リリースのため記載しない (issue 0043 closed と同方針。RTMP Inbound Endpoint 本体の `[ADD]` エントリは存在せず、`## develop` 内には `[ADD] 依存ライブラリに shiguredo_rtmp を追加する` のみ)。実装着手時に released 節へ移っていれば `[UPDATE]` で記載する。

### CI

`cargo check && cargo clippy --all-targets -- --deny warnings && cargo test && cargo fmt --all -- --check` が pass する (RTMP 経路は feature gate されておらず default build に含まれる)。

## 関連

- **issue 0043 (closed, `issues/closed/0043-feature-refactor-h264-sample-entry-from-sps-pps-lists.md`)**: H.264 SRT / RTSP / encoder 3 経路を `h264_sample_entry_from_sps_pps_lists` に統一した前提 issue。本 issue は 0043 の `### 本 issue で触らない経路` で「将来別 issue で対応」とした RTMP 経路の宿題を吸収する。
- **issue 0048 (open, `issues/0048-feature-refactor-h265-sample-entry-from-vps-sps-pps-lists.md`)**: H.265 経路の同型リファクタ (`h265_sample_entry` / `h265_sample_entry_from_annexb` の本体リファクタ + video_toolbox / nvcodec encoder の呼び出し側追従)。本 issue (H.264 RTMP) と 0048 (H.265 encoder 経路 + Annex-B 薄いラッパー) は対象コーデックが異なるため互いに依存せず、いずれも closed/0043 で確立した薄いラッパー化方針 (タプル戻り値 `(SampleEntry, VideoFrameSize)`、空 list 検査、破壊的シグネチャ変更) を踏襲する。closed/0043 の `### 本 issue で触らない経路` 4 項目のうち、本 issue が RTMP H.264 経路を、0048 が H.265 経路を吸収する (残る `extract_video_dimensions` 削除判定、`src/codec_string.rs::from_codec_pair` 内 `"avc1.42e01f"` 固定リテラル、および AV1 経路 `src/video/av1.rs::av1_sample_entry` は本 issue / 0048 のいずれもカバーせず、未起票で残る)。

## 解決方法

完了条件に従い、4 コミットで対応した。

1. `src/rtmp/frame.rs::avc_sequence_header_to_sample_entry` を `h264_sample_entry_from_sps_pps_lists(seq_header.sps_list.clone(), seq_header.pps_list.clone())` の 1 行委譲のみで構成される薄いラッパーに置き換えた。シグネチャは `fn(seq_header: &shiguredo_rtmp::AvcSequenceHeader) -> Result<(SampleEntry, VideoFrameSize)>` に変更し、独自の `Avc1Box` / `AvccBox` 組み立てロジックを削除した。`src/rtmp/frame.rs` の use 宣言に `use crate::video::VideoFrameSize;` を追加した。
2. `src/rtmp/frame.rs::process_video_frame` の H.264 SequenceHeader 経路で `let width = 0; let height = 0;` と TODO コメント 2 行を削除し、改修後の `avc_sequence_header_to_sample_entry(&seq_header)` の戻り値タプルから `frame_size` を受け取って `tracing::debug!` に SPS 由来実値で出力するよう更新した。
3. `src/video/h264.rs::tests::PPS_NAL` を `pub(crate)` 化し、`src/rtmp/frame.rs` 末尾に `#[cfg(test)] mod tests` を新設して `avc_sequence_header_to_sample_entry` の単体テストを 3 件追加した (Baseline SPS 反映 + 空 SPS Err + 空 PPS Err)。
4. `src/video/h264.rs::tests` の `SPS_320X240` / `SPS_1920X1080` 定義コメントに NAL ヘッダ直後 3 バイトの SPS パラメータ (profile_idc=66 / constraint_set_flags=0xc0 / level_idc) を追記し、フィクスチャ提供元にバイト解析を集約した。

副次的な外部観測可能な挙動変化:

- `tracing::debug!("Received H.264 sequence header: {}x{}", ...)` の出力が "0x0" → SPS 由来実値に変わった。
- 保持される `Avc1Box.avcc_box` の `chroma_format` / `bit_depth_*` が High 系プロファイル時に `None` → `Some(SPS 由来実値)` に変わった。`avc_profile_indication` / `profile_compatibility` / `avc_level_indication` も `seq_header` 由来 → SPS 由来実値に変わった。
- `length_size_minus_one` が `seq_header` 由来 (0..=3) → `Uint::new(3)` 固定に変わった。
- `parse_sps` 厳格化により、改修前は通っていた仕様外 publisher (profile_idc が `{66, 77, 88} ∪ H264_HIGH_PROFILES` 外 / High 系 + chroma_format_idc > 3 / bit_depth_*_minus8 > 6 / 解像度 0 / u16::MAX 超 等) は SequenceHeader 受信時点で Err になり接続切断される (既存 fail-fast 方針と整合)。

CHANGES.md は `## develop` 内未リリースのため記載しない (issue 0043 closed と同方針)。

### 残懸念 (別 issue 起票候補)

- **PBT 化**: ラッパー固有の振る舞い (seq_header の上層 4 フィールドが結果不変、sps_list / pps_list のパススルー) は PBT 向きで、現状の単体テスト 3 件は PBT で代替可能。`pbt/tests/prop_rtmp_frame.rs` 新設を別 issue 候補とする。
- **ラッパー存在意義の再考**: 薄いラッパー (1 行委譲) を残すか、`process_video_frame` 内にインライン化するか。issue 0048 (H.265 同型) 着手時に「H.264 RTMP / H.265 全経路を同じ判断で揃える」観点で再評価する候補。
- **3 以外の `length_size_minus_one` publisher 対応**: 受信フレーム payload を 4 バイト prefix に変換する経路、または受信時 Err 化。現状コードでもデコード失敗するため新規 broken window ではないが、明示的な早期検出は将来別 issue 候補。
- **issue 0048 への双方向リンク追加**: 0048 側 `## 関連` に本 issue 0050 への参照を追記する作業 (コード変更なし、develop ブランチで直接対応可)。
- **closed/0043 残懸念のうち未起票項目**: `extract_video_dimensions` 削除判定、`src/codec_string.rs::from_codec_pair` 内 `"avc1.42e01f"` 固定リテラル、AV1 経路 `av1_sample_entry` の固定値解消は本 issue / 0048 のいずれもカバーせず、未起票で残る。
