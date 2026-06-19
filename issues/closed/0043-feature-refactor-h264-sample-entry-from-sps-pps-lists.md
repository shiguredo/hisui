# h264_sample_entry_from_annexb を SPS / PPS リスト受け取り版にリファクタして NAL 走査の二重化と avcC フィールドの固定値を解消する

- Priority: Low
- Created: 2026-06-18
- Completed: 2026-06-19
- Model: Opus 4.7
- Branch: feature/refactor-h264-sample-entry-from-sps-pps-lists
- Polished: 2026-06-19

## 目的

副次的に外部観測可能な挙動修正 (RTSP visual 0 → 実値、avcC 固定値 → SPS 由来実値、codec_string 固定 → SPS 由来実値) を伴う refactor。`src/video/h264.rs::h264_sample_entry_from_annexb` は次の 3 つの broken window を抱えている。

1. **NAL 走査の二重化**: 同じバイト列に対する `H264AnnexBNalUnits` 走査が呼び出し側 + 関数内側で 2 回行われる経路がある (SRT inbound / RTSP subscriber 経路)。
2. **avcC ヘッダーフィールドの固定値**: `avc_profile_indication` / `avc_level_indication` が Baseline / Level 3.1 固定で「TODO: 実際の値に合わせる」コメント付き。`profile_compatibility` は 0 固定で「いったん 0 を指定しているが、もし支障があれば調整する」コメント付き。`chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` も `None` 固定で TODO コメントは無いが同じ broken window。
3. **video_toolbox 経路のコードクローン**: `src/encoder/video_toolbox.rs::h264_sample_entry` 関数が `h264_sample_entry_from_annexb` の avcC 組み立てロジックを丸ごと複製しており、固定値が同じ構造で 2 重管理になっている。

issue 0037 (closed) で SPS 解像度抽出ユーティリティ `extract_dimensions_from_sps` が整い、SPS バイト列から profile_idc / level_idc / constraint_set_flags / chroma_format_idc / bit_depth_* / cropping 適用後 width / height のすべてが取り出せる状態になった。本 issue では下記 3 つを実施する (上記 broken window 1 / 2 / 3 と 1:1 対応):

1. `h264_sample_entry_from_annexb` を「SPS / PPS リスト受け取り版」の新ヘルパー関数 `h264_sample_entry_from_sps_pps_lists` の薄いラッパーにリファクタリングして、NAL 走査の二重化を解消する (broken window 1 解消)。
2. SPS バイト列から取り出した実値を avcC ボックスに反映し、固定値を解消する (broken window 2 解消)。
3. video_toolbox 経路の独自関数も新ヘルパー関数に統合して、Avc1Box 組み立てロジックの二重化を解消する (broken window 3 解消)。

本 issue は `## develop` 内中間状態の修正のため CHANGES.md は更新しない (closed 0030 / 0031 / 0032 / 0033 / 0034 / 0037 / 0044 と同様、`## CHANGES.md` 節自体を立てない方針)。

## 優先度根拠

Low。主目的は内部効率化 (NAL 走査 1 回化) と broken window 解消 (固定値 / コードクローン)。

ただし副次的に下記の外部観測可能な挙動変化が発生する。いずれも「avcC が ITU-T H.264 仕様 + ISO/IEC 14496-15 仕様の値に揃う」方向の修正で、下流プレイヤー / ツールの互換性に対する影響は中立から改善寄り。実害は発生していないため Low 優先度を維持する。

- `avc_profile_indication` / `avc_level_indication` / `profile_compatibility` が固定値 (66 / 31 / 0) から SPS 由来の実値に変わる。
- `chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` が High 系プロファイル時に `None` から `Some(SPS 由来実値)` に変わる。
- `src/rtsp/subscriber.rs` 経路で `Avc1Box.visual.width` / `.height` が現状の 0 から SPS 由来の実値に変わる (事実上 bug fix 相当。下流プレイヤーが visual.width = 0 を解像度 0x0 として扱う可能性があるため、MP4 出力経路で実害顕在化前に対処する位置づけ)。
- `src/codec_string.rs::from_sample_entries` 経路で生成される H.264 `codec_string` が「`avc1.42001f` 固定」から SPS 由来の実値 (Baseline ストリーム例: `avc1.42c01f` 等、libx264 が `constraint_set0_flag` / `constraint_set1_flag` を立てる場合。High ストリーム例: `avc1.640028`) に変わる。HLS マニフェストの `EXT-X-STREAM-INF.CODECS` 属性経由で MP4 出力下流に伝播する。`src/codec_string.rs::video_codec_string_from_sample_entry` の出力フォーマットは `format!("avc1.{:02x}{:02x}{:02x}", profile, compat, level)` でアンダースコアは入らない (実装着手時に再確認する)。
- `extract_dimensions_from_sps` / `h264_sample_entry_from_sps_pps_lists` の戻り値が、ITU-T H.264 仕様 7.4.2.1.1 の値域外 SPS で従来 Ok から Err に変わる。仕様準拠の publisher (libx264 / ハードウェアエンコーダ / Sora 等) では発生しない想定で実害は無い。詳細な Err 化対象は `### parse_sps と SpsParams` 参照。

## 現状

行番号は実装着手時に関連シンボルを grep で再特定する。本文では原則として関数名・型名で参照する。

### 改修対象の関数・呼び出し側 (全 7 箇所)

| 呼び出し側                                                            | 種別           | 現状                                                                                              | 改修方針                                                                                                       |
| --------------------------------------------------------------------- | -------------- | ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `src/srt/inbound_endpoint.rs::SrtTsDemuxer::build_video_sample`       | 本番経路       | IDR 判定 + SPS NAL 収集ループ (`sps_nal: Option<&[u8]>`) で `H264AnnexBNalUnits` を 1 回走査。`extract_dimensions_from_sps(sps_nal)` で width / height を取り出して `h264_sample_entry_from_annexb(width, height, &pending.data)` を呼ぶ。関数内側で再度走査して SPS / PPS リスト抽出。**NAL 走査二重化の発生源**。 | 既存ループで SPS NAL に加えて PPS NAL も `Vec<Vec<u8>>` に収集し (`nalu.data.to_vec()`)、走査終了後 `if keyframe` ブロック内で新ヘルパー関数を直接呼ぶ。`extract_dimensions_from_sps` 呼び出しは削除し、新ヘルパー関数の戻り値タプルから `VideoFrameSize` を直接受け取る (`### last_video_frame_size の取得経路` 参照)。`sps_list` / `pps_list` が空のとき新ヘルパー関数が `Err("missing H.264 SPS")` / `Err("missing H.264 PPS")` を返し、既存の `missing H.264 SPS in IDR PES` Err と等価な fail-fast を維持する (エラーメッセージ末尾の ` in IDR PES` が消えるが、SRT 側テスト `srt_h264_returns_err_on_idr_with_only_sps` / `srt_h264_returns_err_on_idr_with_only_pps` は `assert!(result.is_err(), ...)` のみで文言を直接 assert していないため追従修正不要)。 |
| `src/rtsp/subscriber.rs::VideoRtpReceiver::apply_sample_entry`        | 本番経路       | `has_idr / has_sps / has_pps` の bool 判定ループで `H264AnnexBNalUnits` を 1 回走査するが、**NAL 本体 (`nalu.data`) は捨てている**。3 条件成立時に `h264_sample_entry_from_annexb(0, 0, &frame.data)` を呼ぶ。関数内側で再度走査して SPS / PPS を収集。**NAL 走査二重化の発生源**。`Avc1Box.visual.width / .height` に 0 が埋まる。 | 判定ループを SPS / PPS NAL 本体収集 (`nalu.data.to_vec()`) に変更し、3 条件成立時に `h264_sample_entry_from_sps_pps_lists` を直接呼ぶ。`Avc1Box.visual` が SPS 由来の実値に変わる。 |
| `src/rtsp/subscriber.rs::extract_sample_entry_from_sprop`             | 本番経路       | SDP `sprop-parameter-sets` を Base64 デコードして組み立てた Annex-B に対して `H264AnnexBNalUnits` を 1 回走査。`has_sps / has_pps` の bool 判定後 `h264_sample_entry_from_annexb(0, 0, &annexb)` を呼ぶ。**NAL 本体は捨てている**。関数内側で再度走査。**NAL 走査二重化の発生源**。 | 判定ループを SPS / PPS NAL 本体収集 (`nalu.data.to_vec()`) に変更し、両方揃ったら `h264_sample_entry_from_sps_pps_lists` を直接呼ぶ。`Avc1Box.visual` が SPS 由来の実値に変わる。 |
| `src/encoder/openh264.rs::Openh264Encoder::encode`                    | 本番経路       | `encoded.sps_list` / `encoded.pps_list` を既に保持しながら `create_sequence_header_annexb(&...)` で Annex-B に詰め直して `h264_sample_entry_from_annexb(size.width, size.height, &annexb)` を呼ぶ。関数内側で再度走査。**Annex-B 中間構築 + 関数内側走査の両方が無駄**。 | `encoded.sps_list.clone()` / `encoded.pps_list.clone()` を `h264_sample_entry_from_sps_pps_lists` に直接渡す。`create_sequence_header_annexb` 呼び出しは削除 (encoder 注記参照)。 |
| `src/encoder/video_toolbox.rs::VideoToolboxEncoder::handle_encoded`   | 本番経路       | `h264_sample_entry_from_annexb` を呼ばずに **独自の `h264_sample_entry(width, height, sps_list, pps_list)` 関数** を呼んで Avc1Box を組み立てる。`avc_profile_indication: H264_PROFILE_BASELINE` / `avc_level_indication: H264_LEVEL_3_1` / `profile_compatibility: 0` / `chroma_format: None` / `bit_depth_*: None` が同じ構造で残っている。**コードクローン**。 | `frame.sps_list.clone()` / `frame.pps_list.clone()` を `h264_sample_entry_from_sps_pps_lists` に直接渡し、戻り値の `SampleEntry` を既存通り `SharedSampleEntry::new(...)` で wrap する。独自 `h264_sample_entry` 関数は削除 (encoder 注記参照)。 |
| `src/encoder/nvcodec.rs::NvcodecEncoder::new_h264`                    | 本番経路       | nvcodec が返す `seq_params` (Annex-B 形式バイト列を想定) を `h264_sample_entry_from_annexb(width, height, &seq_params)` に渡す。呼び出し側は SPS / PPS リストを持たない。 | 薄いラッパー `h264_sample_entry_from_annexb(&seq_params)` を引き続き呼ぶ (引数 width / height を削除した形に追従)。戻り値の `SampleEntry` を既存通り `SharedSampleEntry::new(...)` で wrap する (encoder 注記参照)。`shiguredo_nvcodec::Encoder::get_sequence_params()` が start code prefix 込みの Annex-B 形式を返すことを実装着手時に確認する。 |
| `src/decoder/openh264.rs::tests::build_annexb_input_*` (2 関数)      | テストフィクスチャ | Annex-B 形式 / AVCC 形式の偽 SPS バイト列 (NAL ヘッダ込みで 8 バイト: `0x67, 0x42, 0x00, 0x1f, 0xe5, 0x88, 0x68, 0x54`) を直接埋め込んで `h264_sample_entry_from_annexb(320, 320, &annexb)` を呼ぶ。 | 偽 SPS バイト列を `parse_sps` が完走できる実 SPS (`SPS_320X240` の 24 バイト) に差し替え、薄いラッパー `h264_sample_entry_from_annexb(&annexb)` を呼ぶ形に追従。`SPS_320X240` を 2 モジュール間で共有するため、`#[cfg(test)] pub(crate) const SPS_320X240` 化する (関連: `src/video/h264.rs::tests` モジュールも `pub(crate)` 化が必要)。`build_annexb_input_keeps_existing_sps_pps` は AVCC 形式 (NAL 長 prefix 付き) の入力を使うため、SPS バイト長 prefix `[0, 0, 0, 8]` を SPS_320X240 の長さに合わせて `[0, 0, 0, 24]` に同時更新する (PPS 長 prefix `[0, 0, 0, 4]` は不変)。assertion は `nalu_types` 系のみで `Avc1Box.visual` を直接見ないため、解像度 320x320 (引数値) → 320x240 (SPS 由来値) の差異は assertion に影響しない。詳細は `### テストフィクスチャ追従` 参照。 |

**encoder 注記** (openh264 / video_toolbox / nvcodec の 3 経路に共通):

- encoder 自身が出力する SPS は仕様内のため、新ヘルパー関数の `parse_sps` Err 化拡張で Err になるケースは実用上発生しない。
- 新ヘルパー関数の戻り値タプルの `VideoFrameSize` は使わず捨てる (`let (entry, _) = h264_sample_entry_from_sps_pps_lists(...)?;`)。`VideoFrame.size` は引き続き encoder 設定値 (openh264: `frame.size()`、video_toolbox: `self.width.get()` / `self.height.get()`、nvcodec: 既存ラッパー経由) を使う既存挙動を維持する (encoder 設定 width/height と SPS 由来 width/height は通常一致するが、本 issue では encoder 経路を主目的の対象外として既存挙動に揃える)。
- `SharedSampleEntry::new(...)` で wrap する既存挙動も維持する。

### テストフィクスチャ追従

`src/decoder/openh264.rs::tests` 内の 2 関数を以下の手順で追従する。

- 共通: `crate::video::h264::SPS_320X240` (新規 `pub(crate) const` 化) を `use` して 24 バイト SPS バイト列をインライン展開バイト配列の `0x67, 0x42, 0xc0, 0x0d, ...` 8 バイト分の差し替え先として使う (現状の 8 バイト偽 SPS が SPS NAL 1 個分なので、その位置に 24 バイトを差し込む)。
- `build_annexb_input_prepends_missing_sps_pps_from_sample_entry` (Annex-B 形式入力): 入力は `[0, 0, 0, 1, ...SPS..., 0, 0, 0, 1, ...PPS...]` の形式。SPS 部分のバイト列だけを 24 バイトに差し替えれば、start code prefix の固定 4 バイトはそのままで動く。
- `build_annexb_input_keeps_existing_sps_pps` (AVCC 形式入力): 入力は `[NAL 長 4 バイト, ...SPS..., NAL 長 4 バイト, ...PPS..., NAL 長 4 バイト, ...IDR...]` の形式。SPS バイト列差し替えに合わせて先頭 NAL 長 prefix `[0, 0, 0, 8]` を `[0, 0, 0, 24]` に同時更新する。PPS 長 prefix `[0, 0, 0, 4]` と IDR 長 prefix `[0, 0, 0, 2]` は不変。

### avcC 固定値 / `None` 固定の発生箇所

- `src/video/h264.rs::h264_sample_entry_from_annexb`:
  ```rust
  avc_profile_indication: H264_PROFILE_BASELINE, // TODO: 実際の値に合わせる
  avc_level_indication: H264_LEVEL_3_1,          // TODO: 実際の値に合わせる
  profile_compatibility: 0, // いったん 0 を指定しているが、もし支障があれば調整する
  chroma_format: None,
  bit_depth_luma_minus8: None,
  bit_depth_chroma_minus8: None,
  ```
- `src/encoder/video_toolbox.rs::h264_sample_entry`: 上記と同じ構造のコードクローン (TODO コメントは付いていない)。
- `H264_PROFILE_BASELINE = 66` / `H264_LEVEL_3_1 = 31` は `src/video/h264.rs` 内に `pub const` で定義され、参照箇所は上記 2 関数のみ (コメント「H.264 のプロファイルとレベル（Hisui では固定）」)。本 issue 完了後はいずれの参照も消える。

### 既存のテスト

- `src/video/h264.rs` の `#[cfg(test)] mod tests`: `extract_dimensions_from_sps` と内部ヘルパーの単体テスト群。`SpsBuilder` でビット単位の合成 SPS を作るテストユーティリティ (Baseline + 1920x1080 / High + scaling_matrix / cropping / interlaced 等) を持つ。
- `src/srt/inbound_endpoint.rs` の `#[cfg(test)] mod tests`: SRT inbound の `build_video_sample` テスト (`srt_h264_sample_entry_and_size_reflect_sps_dimensions` 等で `Avc1Box.visual.width / .height` を直接 assert する)。
- `src/decoder/openh264.rs` の `#[cfg(test)] mod tests`: `build_annexb_input_*` 系。`nalu_types` の検証のみで `Avc1Box.visual` 直接 assert は無いが、上記表のとおり SPS バイト列と NAL 長 prefix の差し替えが必要。
- `pbt/tests/prop_h264_sps.rs`: `extract_dimensions_from_sps` のクラッシュフリー PBT (proptest, 1024 cases)。

## 設計方針

### 関数構成

以下 3 段構えにする。

#### §1 `h264_sample_entry_from_sps_pps_lists` (新ヘルパー関数、pub)

```rust
pub fn h264_sample_entry_from_sps_pps_lists(
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
) -> crate::Result<(SampleEntry, VideoFrameSize)>
```

- 戻り値: `SampleEntry` と `VideoFrameSize` のタプル。後者は SPS 由来の cropping 適用後解像度で、呼び出し側 (特に SRT inbound) が `last_video_frame_size` 等の設定にそのまま使える。現状の `extract_dimensions_from_sps` + `VideoFrameSize::new` の 2 段呼び出しを 1 段に統合する。
- 入力契約: `sps_list[i]` / `pps_list[j]` は **NAL ヘッダ 1 バイトを含む raw NAL バイト列** (start code は含まない)。これは `H264AnnexBNalUnits::next` が返す `H264NalUnit.data` の形式と `AvccBox.sps_list / pps_list` の格納形式に揃える。
- 所有権は `Vec<Vec<u8>>` で取る。動機: (a) SRT inbound / RTSP 経路で `nalu.data.to_vec()` で作った Vec を `AvccBox.sps_list / pps_list` に move して再 clone を防ぐ。(b) encoder 経路 (openh264 / video_toolbox) では `encoded.sps_list.clone()` を呼び出し側で渡す形でも、新ヘルパー関数内で `AvccBox.sps_list` に move して関数内側での再 clone を防ぐ。
- 内部で `parse_sps(sps_list[0].as_slice())` を 1 回呼んで `SpsParams` を取り出してから、`sps_list` を `AvccBox.sps_list` に move する (`parse_sps` の戻り値 `SpsParams` は値を持ち借用を保持しないため、move 順序の borrow 制約は無い)。複数 SPS は先頭 SPS のパラメータのみを採用し、`AvccBox.sps_list` には全 SPS を move する (SRT inbound 既存方針「複数 SPS は最初の SPS を採用する」と同じ。`sps_list[1..]` の profile / level 一致は検証しない。Hisui の入力前提では同一内容を想定)。
- `sps_list.is_empty()` のときは `Err("missing H.264 SPS")`、`pps_list.is_empty()` のときは `Err("missing H.264 PPS")` を返す。

#### §2 `h264_sample_entry_from_annexb` (薄いラッパー関数、pub、破壊的シグネチャ変更)

```rust
pub fn h264_sample_entry_from_annexb(data: &[u8]) -> crate::Result<SampleEntry>
```

- 内部で `H264AnnexBNalUnits` を 1 回走査して SPS / PPS NAL タイプ (`H264_NALU_TYPE_SPS` / `H264_NALU_TYPE_PPS`) のみを抽出し、`nalu.data.to_vec()` で `Vec<Vec<u8>>` に詰めて `h264_sample_entry_from_sps_pps_lists` を呼ぶ (SEI / IDR / Filler 等の NAL タイプは現コードと同じく無視する)。`to_vec()` の alloc + copy は現行コードと同じで、NAL 走査二重化解消の効率向上効果は「`H264AnnexBNalUnits` の 1 周走査削減のみ」であり、SPS / PPS のコピー自体は減らない。
- 戻り値は `SampleEntry` のみ (タプルの片側だけを返す薄いラッパー)。nvcodec 経路と decoder テストフィクスチャ経路は `VideoFrameSize` を必要としないため、シグネチャを軽くする。
- 引数 `width` / `height` は削除する (破壊的変更)。本 issue 内ですべての呼び出し側を同一 PR で追従する。シグネチャ変更と呼び出し側追従は Rust の型整合上不可分のため同一コミットで実施する。**この破壊的変更は `h264_sample_entry_from_annexb` のみ。`extract_dimensions_from_sps` のシグネチャは §3 に従って維持する。**

#### §3 `extract_dimensions_from_sps` (薄いラッパー化、pub シグネチャ維持)

```rust
pub fn extract_dimensions_from_sps(sps: &[u8]) -> crate::Result<(usize, usize)>
```

- `pub` API シグネチャは維持し、内部実装を `parse_sps(sps).map(|p| (p.width as usize, p.height as usize))` の薄いラッパーにする。これにより `pbt/tests/prop_h264_sps.rs` のクラッシュフリー PBT は無修正で動く。
- pub のまま残す理由は PBT の維持。本 issue で SRT inbound の本番呼び出しを削除すると本関数の本番呼び出し側は消えるが、PBT 経由で `parse_sps` のクラッシュフリー性質を引き続き担保するために pub のまま残す。
- `parse_sps` 内で profile_idc / chroma_format_idc / bit_depth_* の Err 化を新規追加するため、`extract_dimensions_from_sps` 経由でも `(プロファイル指定値, 仕様外値の chroma_format / bit_depth)` の組合せで Err が増える可能性がある。PBT (Ok か Err を返すクラッシュフリー保証) には影響しない。

### parse_sps と SpsParams (案 A 確定)

`parse_sps(sps: &[u8]) -> crate::Result<SpsParams>` を内部関数 (非 `pub`) として実装し、`extract_dimensions_from_sps` はその薄いラッパーにする。`SpsParams` / `HighProfileSpsParams` も非 `pub` のモジュール内 `struct` とする (`h264_sample_entry_from_sps_pps_lists` の戻り値や引数には露出しないため、両型を `pub` にする必要は無い)。

`parse_sps` の入力契約は `extract_dimensions_from_sps` と同じで、**NAL ヘッダ 1 バイトを含む raw NAL バイト列**。NAL タイプ検査 (`!= H264_NALU_TYPE_SPS` で Err) は `parse_sps` 内で実施する。

#### 既存ヘルパー関数の責務分担

現行 `extract_dimensions_from_sps` は `rbsp_from_sps_nalu` / `read_chroma_array_type` / `skip_pic_order_cnt_type_extras` / `read_dimensions_with_cropping` の 4 つのヘルパーに分解されている。本 issue では下記方針で再配置する。

- `rbsp_from_sps_nalu`: 残す。`parse_sps` から呼び出し、NAL タイプ検査 + emulation prevention byte 除去を引き続き担う。
- `read_chroma_array_type`: 削除し、`parse_sps` 本体に統合する。理由: High プロファイル判定 + `HighProfileSpsParams` の構築 + chroma_array_type 算出が密結合しており、現行の `u32` 戻り値だけでは `HighProfileSpsParams` を埋められないため、関数として残すと戻り値型がさらに複雑になる。
- `skip_pic_order_cnt_type_extras`: 残す。`parse_sps` から呼び出して既存挙動を維持。0044 が手を入れる関数なので関数として残すことで 0044 とのコンフリクト範囲を最小化する。
- `read_dimensions_with_cropping`: 残す。`parse_sps` から呼び出す。

#### SpsParams の構造体定義

```rust
struct SpsParams {
    // avcC の avc_profile_indication / profile_compatibility / avc_level_indication にそれぞれ
    // 1 バイトずつ詰める SPS 先頭 3 バイト由来の値。
    profile_idc: u8,
    constraint_set_flags: u8, // constraint_set0..5_flag + reserved_zero_2bits の 1 バイト全体
                              // NAL ヘッダ 1 バイト除去後の RBSP として 0-indexed の byte[1]
                              // (byte[0] = profile_idc / byte[1] = constraint_set + reserved / byte[2] = level_idc)
    level_idc: u8,

    // High 系プロファイル時のみ Some。それ以外のプロファイルでは None。
    // avcC の chroma_format / bit_depth_luma_minus8 / bit_depth_chroma_minus8 の Some / None を
    // この Option の有無で 1 対 1 対応させ、`h264_sample_entry_from_sps_pps_lists` 側で再判定しない。
    high_profile_params: Option<HighProfileSpsParams>,

    // cropping 適用後の最終解像度 (parse_sps 内で width / height > 0 と u16::MAX 上限を保証)。
    width: u16,
    height: u16,
}

struct HighProfileSpsParams {
    chroma_format_idc: u8,        // 0..=3、parse_sps で範囲検証済み
    bit_depth_luma_minus8: u8,    // 0..=6、parse_sps で範囲検証済み
    bit_depth_chroma_minus8: u8,  // 0..=6、parse_sps で範囲検証済み
}
```

- `seq_scaling_matrix_present_flag` 経路は `HighProfileSpsParams` に含めない (avcC への反映先がないため)。`parse_sps` 内ではビット位置を進めるために `skip` するだけ。
- `SpsParams.width` / `.height` は `u16` 型 (現行 `extract_dimensions_from_sps` の u16::MAX 上限チェックを `parse_sps` 内に移動し、戻り値型で範囲を保証する)。`Avc1Box.visual.width / .height` の `u16` 型と素直に対応する。`extract_dimensions_from_sps` の薄いラッパーでは `p.width as usize` / `p.height as usize` で u16 → usize キャストして既存 pub シグネチャ `(usize, usize)` に揃える。
- `VideoFrameSize::new` は `u16 → usize` キャスト後の値を受け取る (`### last_video_frame_size の取得経路` 参照)。
- `level_idc` / `constraint_set_flags` は現行 `extract_dimensions_from_sps` で `reader.skip_u(8)` していたものを `reader.read_u(8)` に変更して取り出す。
- High 系プロファイル判定は `H264_HIGH_PROFILES.contains(&profile_idc)` を `parse_sps` 内 1 箇所に集約する。`HighProfileSpsParams` の各フィールドは ITU-T H.264 仕様 7.4.2.1.1 の値域逸脱 (`chroma_format_idc > 3` / `bit_depth_luma_minus8 > 6` / `bit_depth_chroma_minus8 > 6`) を `parse_sps` 内で `Err` 化する。これにより `h264_sample_entry_from_sps_pps_lists` 側では `Uint::new(value)` を `u8` 範囲確定で安全に書ける。
- 実装コードのコメント・docstring・エラーメッセージには issue 番号や他 issue 由来の比喩 (`pic_order_cnt_type` Err 化と同方針 等) を書かない。エラーメッセージは「`invalid H.264 SPS: chroma_format_idc out of spec range (0..=3): ...`」のように仕様参照のみで記述する。

案 B (`parse_sps_profile_level` のような小さなヘルパーを別途切り出して `extract_dimensions_from_sps` と新ヘルパー関数で個別に呼ぶ案) は SPS パース処理が分散して保守性が下がるため不採用。案 A では `extract_dimensions_from_sps` 経由の SPS パースでも `chroma_format_idc > 3` 等の新 Err 化が走るが、既存呼び出し側 (SRT inbound + PBT) では実機 SPS は仕様内のため既存挙動は変わらない (PBT は Ok / Err どちらでもクラッシュフリー保証は維持される)。

### avcC フィールドの反映 (`h264_sample_entry_from_sps_pps_lists` 内の対応関係)

| avcC フィールド            | 反映値                                                     |
| -------------------------- | ---------------------------------------------------------- |
| `avc_profile_indication`   | `SpsParams.profile_idc`                                    |
| `profile_compatibility`    | `SpsParams.constraint_set_flags`                           |
| `avc_level_indication`     | `SpsParams.level_idc`                                      |
| `chroma_format`            | `SpsParams.high_profile_params.map(|h| Uint::new(h.chroma_format_idc))` |
| `bit_depth_luma_minus8`    | 同上 (`h.bit_depth_luma_minus8`)                           |
| `bit_depth_chroma_minus8`  | 同上 (`h.bit_depth_chroma_minus8`)                         |
| `length_size_minus_one`    | `Uint::new(NALU_HEADER_LENGTH as u8 - 1)` (現状値維持。`NALU_HEADER_LENGTH = 4` は Hisui の MP4 出力時固定値) |
| `sps_ext_list`             | `Vec::new()` を維持 (ITU-T H.264 仕様の sequence_parameter_set_extension_rbsp / subset_seq_parameter_set_rbsp NAL は Hisui の入力前提では発生しないと判断。将来要件が出たら別 issue。仕様の正確な節番号は実装着手時に確認する) |

### `H264_HIGH_PROFILES` と `shiguredo_mp4` のエンコード前提との整合

`shiguredo_mp4` 2026.3.0 の `boxes_sample_entry.rs::AvccBox::encode` は `!matches!(self.avc_profile_indication, 66 | 77 | 88)` のときに `chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` が `Some` でないとエンコードエラーを返す。一方 `H264_HIGH_PROFILES = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135]` (ITU-T H.264 仕様 7.3.2.1.1) は `66 / 77 / 88` を含まないため、両者の和集合 `{66, 77, 88} ∪ H264_HIGH_PROFILES` 以外の `profile_idc` 値 (例: 壊れた SPS で profile_idc=99 / 140 等) では、`H264_HIGH_PROFILES` に該当しないため `chroma_format = None` を埋め、shiguredo_mp4 がエンコード時に Err を返す可能性がある。

これを防ぐため、`parse_sps` 内で `profile_idc` が `{66, 77, 88} ∪ H264_HIGH_PROFILES` のいずれにも含まれない値の場合は `Err("invalid H.264 SPS: unsupported profile_idc N")` を返す (Hisui の入力前提では実機 SPS で発生しないはずの値)。

仕様準拠の publisher が出す各種プロファイルは以下のように包含される:

- Baseline (66) / Constrained Baseline (66 + constraint_set1_flag=1) / Main (77) / Extended (88): `{66, 77, 88}` でカバー。Constrained Baseline / Constrained High 等のサブプロファイルは `profile_idc` が元プロファイルと同じため、追加の判定不要。
- High (100) / High 10 (110) / High 4:2:2 (122) / High 4:4:4 Predictive (244) / Scalable / Multiview 系 (44 / 83 / 86 / 118 / 128 / 138 / 139 / 134 / 135): `H264_HIGH_PROFILES` でカバー。
- MVC subset SPS (`profile_idc = 118 / 128 / 134 / 135`) のビット構造は AVC SPS と先頭部分が同一 (`subset_seq_parameter_set_rbsp` の先頭は `seq_parameter_set_data` と同じ構造、仕様 7.3.2.1.2 / 7.3.2.1.3 のいずれかは実装着手時に refs/ で確認) なので `parse_sps` で扱えるが、Hisui の入力前提では発生しない想定。

### `last_video_frame_size` の取得経路

SRT inbound `build_video_sample` では新ヘルパー関数の戻り値タプル `(entry, frame_size)` から `frame_size` を直接受け取り、`self.last_video_frame_size = Some(frame_size)` に格納する。現状の `extract_dimensions_from_sps` + `VideoFrameSize::new` の 2 段呼び出しを 1 段に統合する。`parse_sps` 内で `width / height > 0` チェック (cropping 適用後) を行うため `VideoFrameSize::new(p.width as usize, p.height as usize)` は infallible (`VideoFrameSize::new` 自体は width == 0 / height == 0 のみ Err を返す仕様。u16::MAX 上限は `SpsParams.width: u16` の型レベルで保証され、`VideoFrameSize::new` 自身はチェックしない)。この前提条件をテストで担保する (既存テスト `extract_dimensions_rejects_zero_dimensions_after_cropping` / `extract_dimensions_rejects_width_exceeding_u16_max` が `parse_sps` 経由でも Err を返すことを確認)。

### `H264_PROFILE_BASELINE` / `H264_LEVEL_3_1` 定数の削除

本 issue の改修後は両定数とも参照箇所が消える (`src/video/h264.rs::h264_sample_entry_from_annexb` と `src/encoder/video_toolbox.rs::h264_sample_entry` の両方が新ヘルパー関数に置き換わるため)。`pub const` 定義とコメント「H.264 のプロファイルとレベル（Hisui では固定）」を削除する。`src/encoder/video_toolbox.rs` の import (`H264_LEVEL_3_1` / `H264_PROFILE_BASELINE` / `Avc1Box` / `AvccBox` / `Uint` の use 行) を整理する。実装着手時の最終 grep `grep -rn 'H264_PROFILE_BASELINE\|H264_LEVEL_3_1' src/ examples/ tests/ pbt/` で本 issue で削除した 2 関数以外に参照が 0 件であることを確認する。なお `src/codec_string.rs::from_codec_pair` には `"avc1.42e01f"` という固定リテラルがあるが、これは定数を参照していない別経路で本 issue では触らない (`### 本 issue で触らない経路` 参照)。

### エラー処理の境界条件

`parse_sps` の Err 化拡張 (profile_idc 範囲外 / chroma_format_idc > 3 / bit_depth_*_minus8 > 6) が発生した場合の取り扱いは、既存の fail-fast 方針と整合する:

- SRT inbound: `build_video_sample` は Err 上位伝播で接続切断 (既存方針)。
- RTSP `apply_sample_entry`: `SessionError::Fatal` に変換されて接続打ち切り (既存方針)。
- RTSP `extract_sample_entry_from_sprop`: 壊れた SDP として Err 伝播 (既存方針)。
- encoder 3 経路 (openh264 / video_toolbox / nvcodec): `?` で上位伝播 (既存方針)。

writer 入口の `resolve_*_sample_entry` (issue 0034 で導入された fallback 補完経路) とは独立。本 issue の Err 経路は「frame が下流に流れる前に processor が止まる」シナリオで、writer 側 fallback は「frame が流れたが sample_entry が None だった」シナリオを扱う。両者は責務が異なり、issue 0030 / 0031 / 0032 / 0033 で確立した「圧縮フレームは常に sample_entry を持つ」不変条件 (src/video.rs::VideoFrame.sample_entry の docstring 参照、WebM リーダー H264AnnexB / AV1 経路は issue 0047 で対応中) の有無は本 issue では変えず、sample_entry の中身を SPS 由来実値に置き換えるのみ。

### 本 issue で触らない経路

下記経路は本 issue のスコープ外。「将来別 issue で扱う可能性」までで止めて issue 番号は立てない。

- **`src/rtmp/frame.rs::avc_sequence_header_to_sample_entry`**: RTMP 経路にも独自の Avc1Box 組み立てロジックが存在し、`chroma_format` / `bit_depth_*` が `None` 固定 (`avc_profile_indication` / `avc_level_indication` / `profile_compatibility` は `seq_header` 由来の実値を使う)。video_toolbox 経路と同種の broken window だが、RTMP 経路は width / height を別途引数で受ける構造で SRT / RTSP とは制御フローが異なり、入力は SPS バイト列ではなく `shiguredo_rtmp::AvcSequenceHeader` 構造体のため、本 issue では触らない。
- **`src/codec_string.rs::from_codec_pair`**: `CodecName::H264` のフォールバック値 `"avc1.42e01f"` 固定リテラル。`from_sample_entries` 経由とは別の代表値経路で、本 issue では触らない。
- **`src/video/h264.rs::extract_video_dimensions`**: `pub` 関数だが現状の呼び出し側は無い (本 issue 着手時点で grep `extract_video_dimensions` のヒットは定義のみ)。本 issue では削除しない (本関数の delete 判定は本 issue のスコープから外す)。
- **H.265 / AV1 系の同種リファクタ**: `h265_sample_entry_from_annexb` / `av1_sample_entry` 経路にも類似の固定値 / 走査二重化が存在する可能性。本 issue で `src/encoder/video_toolbox.rs::handle_encoded` 内の H.264 経路だけを新ヘルパー関数に統一すると、同じ `handle_encoded` 内で並列に呼ばれる `h265::h265_sample_entry(width, height, fps, vps_list, sps_list, pps_list)` (シグネチャに width / height / fps を持つ既存形式) とコード対称性が崩れるが、H.265 経路の改修は本 issue のスコープ外。

### 推奨パッチ順序

実装者は以下の 4 ステップで作業し、各ステップ完了時点で `cargo check && cargo test` が pass する原子コミットを作る。

1. **§3 + parse_sps 追加** (非破壊):
   - `parse_sps` / `SpsParams` / `HighProfileSpsParams` を `src/video/h264.rs` に追加。
   - `extract_dimensions_from_sps` を `parse_sps` の薄いラッパーに変更 (`pub` シグネチャ維持)。
   - `read_chroma_array_type` を削除し `parse_sps` 本体に統合。`skip_pic_order_cnt_type_extras` / `read_dimensions_with_cropping` / `rbsp_from_sps_nalu` は残す。
2. **テスト拡張**:
   - `SpsBuilder` を拡張 (`with_profile_idc(u32)` / `with_chroma_format_idc(u32)` / `with_bit_depth_luma_minus8(u32)` / `with_bit_depth_chroma_minus8(u32)` 等、内部は既存 `write_u` の `u32` に揃える)。`SpsBuilder::raw` のデフォルト値で `chroma_format_idc = 1` / `bit_depth_luma_minus8 = 0` / `bit_depth_chroma_minus8 = 0` を初期化し、`build` 内 `is_high` 分岐のハードコード値をフィールド参照に置換。
   - `parse_sps` 内 Err 化テスト 4 ケースと Baseline / Main / High / High10 の avcC 反映テストを追加。
   - `src/video/h264.rs::tests` モジュールを `pub(crate) mod tests` 化し、`SPS_320X240` を `pub(crate) const` 化 (`src/decoder/openh264.rs::tests` から参照するため)。
3. **§1 + §2 + 全 7 呼び出し側追従** (破壊的、原子コミット):
   - `h264_sample_entry_from_sps_pps_lists` を追加。
   - `h264_sample_entry_from_annexb` を新シグネチャ (`fn(data: &[u8]) -> Result<SampleEntry>`) の薄いラッパーに変更。
   - 全 7 呼び出し側 (SRT inbound / RTSP × 2 / openh264 / video_toolbox / nvcodec / decoder/openh264 テスト × 2) を新シグネチャに追従。
   - decoder/openh264 テストの SPS バイト列差し替え + AVCC 形式テストの NAL 長 prefix `[0, 0, 0, 8]` → `[0, 0, 0, 24]` 更新。
4. **dead code 削除**:
   - `H264_PROFILE_BASELINE` / `H264_LEVEL_3_1` 定数削除。
   - `src/encoder/video_toolbox.rs::h264_sample_entry` 関数削除。
   - `src/video/h264.rs::create_sequence_header_annexb` 関数削除 (本 issue 完了後は呼び出し側が消える)。
   - `src/encoder/video_toolbox.rs` の import 整理。

## 完了条件

設計方針の §1〜§3 と完了条件を 1:1 対応で整理する。

### §1 `h264_sample_entry_from_sps_pps_lists` 新設

- `src/video/h264.rs` に `pub fn h264_sample_entry_from_sps_pps_lists(sps_list: Vec<Vec<u8>>, pps_list: Vec<Vec<u8>>) -> Result<(SampleEntry, VideoFrameSize)>` が追加されている。
- 内部で `parse_sps(sps_list[0].as_slice())` を 1 回呼び、その後 `sps_list` / `pps_list` を `AvccBox.sps_list / pps_list` に move する。
- `sps_list.is_empty()` で `Err("missing H.264 SPS")` / `pps_list.is_empty()` で `Err("missing H.264 PPS")` を返す。
- 構築した `SampleEntry::Avc1` の `avcc_box` フィールドが SPS 由来の実値を持つ:
  - `avc_profile_indication` = SPS の `profile_idc`
  - `profile_compatibility` = SPS の `constraint_set0..5_flag + reserved_zero_2bits` (NAL ヘッダ除去後の RBSP byte[1]、0-indexed)
  - `avc_level_indication` = SPS の `level_idc`
  - High 系プロファイル時のみ `chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` が `Some(Uint::new(SPS 由来実値))`、それ以外は `None`
  - `length_size_minus_one` / `sps_ext_list` は現状値を維持

### §2 `h264_sample_entry_from_annexb` 薄いラッパー化 (破壊的シグネチャ変更)

- `h264_sample_entry_from_annexb` のシグネチャから `width` / `height` 引数が削除され、内部が `H264AnnexBNalUnits` を 1 回走査して SPS / PPS のみ `nalu.data.to_vec()` で抽出 → `h264_sample_entry_from_sps_pps_lists` 呼び出し → `SampleEntry` のみ返す薄いラッパーになっている。
- §2 経由で呼び出す側 (Annex-B バイト列のみ持ち SPS / PPS リストを別途構築しない経路) は計 3 箇所: `src/encoder/nvcodec.rs::new_h264` / `src/decoder/openh264.rs::tests::build_annexb_input_*` 2 箇所が新シグネチャに追従している。一方 SRT inbound / RTSP × 2 / openh264 / video_toolbox の 4 箇所は SPS / PPS リストを直接持つため §1 `h264_sample_entry_from_sps_pps_lists` を直接呼ぶ (改修対象表参照)。本 issue の全 7 呼び出し側 = §1 直接呼び 4 箇所 + §2 経由 3 箇所。

### §3 `extract_dimensions_from_sps` 薄いラッパー化 (pub シグネチャ維持)

- `extract_dimensions_from_sps` の `pub` API シグネチャは維持されたまま、内部実装が `parse_sps(sps).map(|p| (p.width as usize, p.height as usize))` の薄いラッパーになっている。

### parse_sps と SpsParams

- `src/video/h264.rs` に `fn parse_sps(sps: &[u8]) -> Result<SpsParams>` (非 pub) と `SpsParams` / `HighProfileSpsParams` (非 pub のモジュール内 struct) が追加されている。
- `parse_sps` の入力契約は NAL ヘッダ 1 バイト含む raw NAL バイト列で、内部で `rbsp_from_sps_nalu` (関数として残置) を呼んで RBSP を取り出す。NAL タイプ検査は `rbsp_from_sps_nalu` 内で実施 (既存実装の流用)。
- `read_chroma_array_type` は削除され、High プロファイル分岐と `HighProfileSpsParams` の構築が `parse_sps` 本体に統合されている。`skip_pic_order_cnt_type_extras` / `read_dimensions_with_cropping` / `rbsp_from_sps_nalu` は関数として残置されている。
- `parse_sps` 内で次の Err 化が実装されている (堅牢性):
  - `profile_idc` が `{66, 77, 88} ∪ H264_HIGH_PROFILES` のいずれにも含まれない値 → `Err`
  - High 系プロファイル時の `chroma_format_idc > 3` → `Err`
  - High 系プロファイル時の `bit_depth_luma_minus8 > 6` → `Err`
  - High 系プロファイル時の `bit_depth_chroma_minus8 > 6` → `Err`
- エラーメッセージは仕様参照のみで記述する (issue 0044 等の番号や `pic_order_cnt_type` のような他 issue 由来の比喩を持ち込まない)。

### 呼び出し側追従

- `src/srt/inbound_endpoint.rs::build_video_sample` で `pending.data` に対する NAL 走査が 1 回だけになり、IDR 判定 + SPS / PPS 収集 (`Vec<Vec<u8>>` で `nalu.data.to_vec()`) が 1 ループに統合されている。`keyframe` 判定は走査内側で従来通り設定し、走査後の `if keyframe { ... }` ブロックで新ヘルパー関数を呼ぶ。`extract_dimensions_from_sps` 呼び出しは削除され、新ヘルパー関数の戻り値タプルから `VideoFrameSize` を直接受け取る。
- `src/rtsp/subscriber.rs::apply_sample_entry` で判定ループが SPS / PPS NAL 本体収集に置き換わり、3 条件成立時に `h264_sample_entry_from_sps_pps_lists` を直接呼ぶ。`Avc1Box.visual.width / .height` が SPS 由来の実値に変わる。
- `src/rtsp/subscriber.rs::extract_sample_entry_from_sprop` も同様に NAL 走査 1 回化が完了している。
- `src/encoder/openh264.rs::encode` で `create_sequence_header_annexb` 呼び出しが削除され、`encoded.sps_list.clone()` / `encoded.pps_list.clone()` を `h264_sample_entry_from_sps_pps_lists` に直接渡している (encoder 注記の `VideoFrame.size` 既存挙動維持を反映)。
- `src/encoder/video_toolbox.rs::handle_encoded` の H.264 経路で独自 `h264_sample_entry` 関数呼び出しが消え、`h264_sample_entry_from_sps_pps_lists` 呼び出しに置き換わっている (encoder 注記の `VideoFrame.size` 既存挙動維持を反映)。独自 `h264_sample_entry` 関数は削除されている。
- `src/encoder/nvcodec.rs::new_h264` で `h264_sample_entry_from_annexb(&seq_params)` (新シグネチャ) を呼ぶ (encoder 注記の `SharedSampleEntry::new(...)` wrap 維持を反映)。
- `src/decoder/openh264.rs::tests::build_annexb_input_prepends_missing_sps_pps_from_sample_entry` および `build_annexb_input_keeps_existing_sps_pps` が `SPS_320X240` (24 バイト実 SPS、`pub(crate) const` 化済み) を埋め込み、`h264_sample_entry_from_annexb(&annexb)` (新シグネチャ) を呼ぶ形に追従。AVCC 形式テストの NAL 長 prefix `[0, 0, 0, 8]` が `[0, 0, 0, 24]` に同時更新されている。

### 定数 / 関数削除

- `H264_PROFILE_BASELINE` / `H264_LEVEL_3_1` 定数とそのコメントが削除されている。
- `src/encoder/video_toolbox.rs::h264_sample_entry` 独自関数が削除されている。
- `src/video/h264.rs::create_sequence_header_annexb` 関数が削除されている (本 issue 完了後の呼び出し側ゼロ)。
- `src/encoder/video_toolbox.rs` の import 行 (`H264_LEVEL_3_1` / `H264_PROFILE_BASELINE` の use と、`Avc1Box` / `AvccBox` / `Uint` の use のうち不要になったもの) が整理されている。

### テスト追加・既存テスト維持

- 新ヘルパー関数の単体テストが `src/video/h264.rs::tests` モジュール内に追加されている (`SpsBuilder` を流用するため)。`SpsBuilder` を `with_profile_idc(u32)` / `with_constraint_set_flags(u8)` / `with_chroma_format_idc(u32)` / `with_bit_depth_luma_minus8(u32)` / `with_bit_depth_chroma_minus8(u32)` 等で拡張する (`SpsBuilder::raw` のデフォルト値で `constraint_set_flags = 0` / `chroma_format_idc = 1` / `bit_depth_luma_minus8 = 0` / `bit_depth_chroma_minus8 = 0` を初期化し、`build` 内 `is_high` 分岐のハードコード値および constraint_set_flags のハードコード値 `w.write_u(8, 0)` をフィールド参照に置換):
  - Baseline (`profile_idc = 66`) で `avc_profile_indication = 66` / `chroma_format = None` / `bit_depth_* = None`
  - Main (`profile_idc = 77`) で `avc_profile_indication = 77` / `chroma_format = None`
  - High (`profile_idc = 100`) で `chroma_format = Some(Uint::new(1))` / `bit_depth_luma_minus8 = Some(Uint::new(0))`
  - High 10 (`profile_idc = 110`) で `bit_depth_luma_minus8 = Some(Uint::new(2))`
  - `profile_compatibility` が SPS の RBSP byte[1] (NAL ヘッダ除去後、0-indexed) と一致
  - `sps_list` 空 → `Err("missing H.264 SPS")`
  - `pps_list` 空 → `Err("missing H.264 PPS")` (PPS バイト列は既存 `src/video/h264.rs::tests` 内の `[0x68, 0xce, 0x06, 0xe2]` を再利用)
  - 戻り値タプルの `VideoFrameSize` が SPS の cropping 適用後の値と一致
  - `parse_sps` 内 Err 化テスト (4 ケース):
    - `profile_idc = 99` (`{66, 77, 88} ∪ H264_HIGH_PROFILES` のいずれにも含まれない値) で Err
    - High プロファイル (`profile_idc = 100`) + `chroma_format_idc = 4` で Err
    - High プロファイル + `bit_depth_luma_minus8 = 7` で Err
    - High プロファイル + `bit_depth_chroma_minus8 = 7` で Err
  - assertion 例 (Baseline ケース、`SpsBuilder` に `with_constraint_set_flags(u8)` を追加して RBSP byte[1] を任意値で指定し `profile_compatibility` の反映を検証する形式):
    ```rust
    let sps = SpsBuilder::raw(1920, 1088).with_constraint_set_flags(0xc0).build();
    let (entry, _) = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![pps_nal()]).expect("Baseline SPS");
    let SampleEntry::Avc1(b) = entry else { panic!("expected Avc1") };
    assert_eq!(b.avcc_box.avc_profile_indication, 66);
    assert_eq!(b.avcc_box.profile_compatibility, 0xc0);
    assert!(b.avcc_box.chroma_format.is_none());
    ```
- 既存テストが全て pass する:
  - `src/video/h264.rs::tests` (`extract_dimensions_from_sps` 系)。特に `extract_dimensions_from_sps_rejects_non_sps_nal` が `parse_sps` 内の NAL タイプ検査経由で同じ assertion を通すこと。
  - `src/srt/inbound_endpoint.rs::tests` (`srt_h264_sample_entry_and_size_reflect_sps_dimensions` 等で `Avc1Box.visual.width / .height` を assert する系)。
  - `src/decoder/openh264.rs::tests` (偽 SPS を実 SPS に差し替えたうえで `nalu_types` 系の assertion がそのまま通る)。
  - `pbt/tests/prop_h264_sps.rs` (`extract_dimensions_from_sps` のクラッシュフリー PBT。内部実装が `parse_sps` の薄いラッパーになっても入力範囲のクラッシュフリー性質は維持される)。

### CI / feature gate

- 以下が pass する。feature gate ごとに分けて確認する:
  - デフォルト build: `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test && cargo fmt --all -- --check`
  - `fdk-aac` feature: `cargo check --features fdk-aac && cargo clippy --features fdk-aac --all-targets -- --deny warnings`
  - `nvcodec` feature (CUDA SDK 利用可能環境): `cargo check --features nvcodec && cargo clippy --features nvcodec --all-targets -- --deny warnings`
  - macOS 限定 `shiguredo_video_toolbox` 経路 (cfg dependency): macOS 上で `cargo check && cargo clippy --all-targets -- --deny warnings`

## 関連

- issue 0037 (closed): SPS 解像度抽出パーサ `extract_dimensions_from_sps` を追加し、本 issue の前提となる SPS パース経路を提供。0037 では「`h264_sample_entry_from_annexb` のシグネチャと既存挙動は変更しない」「`(0, 0)` をセンチネルとして関数内で分岐させる案は採用しない」と意図的に保守的な範囲に限定された。本 issue は 0037 で整った SPS パース基盤を前提に、その制約を一段引き上げて関数内側に SPS 抽出を 1 回で完結させる方針に転換する。
- issue 0044 (open, polish 未完了): `extract_dimensions_from_sps` の `pic_order_cnt_type` 仕様外値 (0/1/2 以外) を Err 化。本 issue で `skip_pic_order_cnt_type_extras` 関数は関数として残置するため、0044 と本 issue は同じ関数経路に手を入れる。コンフリクト範囲は `skip_pic_order_cnt_type_extras` 1 関数のみで局所的なため、0044 の polish 確定を待たずに本 issue を着手しても影響は限定的。マージ順序の責務分担は: (a) 0044 → 0043 順なら、本 issue 着手時に 0044 で追加された `_ => Err(...)` を `skip_pic_order_cnt_type_extras` 内にそのまま反映 (本 issue では同関数を残置するため統合作業は最小)。(b) 0043 → 0044 順なら、0044 が `skip_pic_order_cnt_type_extras` 内の該当箇所にパッチを当てる形に追従する。
- issue 0039 (open, polish 未完了): writer 側 fallback 補完経路 (`src/sample_entry.rs::resolve_*_sample_entry`) の削除可能性を調査する。0039 は「frame 流入時点で sample_entry が常に存在する」前提のもと fallback 補完経路がデッドコードか判定する issue で、本 issue が `Avc1Box` の中身を SPS 由来実値化することとは独立 (writer fallback は `frame.sample_entry.is_none()` のみが対象で、sample_entry の中身は参照しない)。本 issue は 0039 の判定材料に直接影響しない。
- issue 0047 (open, polish 未完了): WebM リーダーの AV1 / H264AnnexB 映像経路に sample_entry 構築を追加。0047 は H264AnnexB 経路で avcC (`AVCDecoderConfigurationRecord`) から SPS / PPS を抽出して本 issue の新ヘルパー関数 `h264_sample_entry_from_sps_pps_lists` を直接呼ぶ設計を想定している。本 issue 完了後に 0047 を着手するのが推奨マージ順序。0047 polish 時に「`h264_sample_entry_from_avcc` を新設する案」と「avcC → SPS/PPS 抽出後に `h264_sample_entry_from_sps_pps_lists` を直接呼ぶ案」の二案を決着させる。

## 解決方法

設計方針の §1 / §2 / §3 と推奨パッチ順序に沿って実装し、レビュー指摘を踏まえて追加対応した。本ブランチで合計 9 コミット。

### §1 / §2 / §3: 新ヘルパー関数と薄いラッパー化

- `src/video/h264.rs` に `pub fn h264_sample_entry_from_sps_pps_lists(sps_list, pps_list) -> Result<(SampleEntry, VideoFrameSize)>` を新設した。入力契約は EBSP 形式 (ISO/IEC 14496-15 §5.3.3.1、NAL ヘッダ 1 バイト含む、start code なし)。先頭 SPS のみ `parse_sps` でパースし avcC フィールドに反映する。`pps_list[i]` の NAL タイプ検査も内部で実施する。
- `h264_sample_entry_from_annexb` のシグネチャから `width` / `height` 引数を削除し、内部で `H264AnnexBNalUnits` を 1 回走査して SPS / PPS を抽出する薄いラッパーに変更した (破壊的シグネチャ変更)。
- `extract_dimensions_from_sps` の `pub` API シグネチャは維持し、内部実装を `parse_sps(sps).map(|p| (p.width as usize, p.height as usize))` の薄いラッパーにした。`pbt/tests/prop_h264_sps.rs` のクラッシュフリー PBT 専用 API として残置する旨を docstring に明示した。

### parse_sps と SpsParams

- `fn parse_sps(sps: &[u8]) -> Result<SpsParams>` を内部関数として実装した。`SpsParams` / `HighProfileSpsParams` は非 pub のモジュール内 struct。
- ITU-T H.264 仕様 7.4.2.1.1 の値域外を Err 化: `profile_idc` が `{66, 77, 88} ∪ H264_HIGH_PROFILES` 以外、High 系で `chroma_format_idc > 3` / `bit_depth_luma_minus8 > 6` / `bit_depth_chroma_minus8 > 6`。
- High 系プロファイル分岐は `fn read_high_profile_sps_fields(reader) -> Result<(u32, HighProfileSpsParams)>` に切り出し、`parse_sps` 本体を約 60 行短縮した。`read_chroma_array_type` は削除して新関数に統合。`rbsp_from_sps_nalu` / `skip_pic_order_cnt_type_extras` / `read_dimensions_with_cropping` は関数として残置。

### 全 7 呼び出し側追従

- SRT inbound `build_video_sample`: 単一ループで IDR 判定 + SPS / PPS NAL 収集 (`Vec<Vec<u8>>` + `nalu.data.to_vec()`) に統合し、`h264_sample_entry_from_sps_pps_lists` を直接呼ぶ形に変更。`extract_dimensions_from_sps` 呼び出しを削除。
- RTSP `apply_sample_entry`: 同様に NAL 走査を 1 回化し、`Avc1Box.visual.width / .height` が SPS 由来の実値になる。
- RTSP `extract_sample_entry_from_sprop`: Base64 デコード結果を直接 SPS / PPS リストに振り分け、`h264_sample_entry_from_sps_pps_lists` を呼ぶ。`forbidden_zero_bit` の検査 (ITU-T H.264 7.4.1 の MUST) を追加して `apply_sample_entry` 経路と対称にした。Ok(None) / Err 境界を docstring で明示。
- openh264 encoder: `create_sequence_header_annexb` 呼び出しを削除し、`encoded.sps_list.clone()` / `encoded.pps_list.clone()` を `h264_sample_entry_from_sps_pps_lists` に直接渡す形に変更。`VideoFrame.size` は引き続き `frame.size()` (encoder 設定値) を使う既存挙動を維持。
- video_toolbox encoder: 独自 `h264_sample_entry` 関数を削除し、`h264_sample_entry_from_sps_pps_lists` に統一。H.264 経路に「空 sps_list / pps_list でサンプルエントリー構築をスキップ」のガードを追加 (openh264 経路と対称、非 keyframe フレームでのエンコーダ落ち回避)。H.265 経路はスコープ外のため挙動を変更せず。
- nvcodec encoder: `h264_sample_entry_from_annexb(&seq_params)` (新シグネチャ) を呼ぶ形に追従。`SharedSampleEntry::new(...)` で wrap する既存挙動を維持。
- decoder/openh264 テスト: 偽 SPS (8 バイト) を `crate::video::h264::tests::SPS_320X240` (24 バイト実 SPS) に差し替え。AVCC 形式テストの NAL 長 prefix `[0, 0, 0, 8]` を `[0, 0, 0, 24]` に同時更新。

### 定数 / 関数削除

- `H264_PROFILE_BASELINE` / `H264_LEVEL_3_1` 定数を削除。
- `src/encoder/video_toolbox.rs::h264_sample_entry` 独自関数を削除。
- `src/video/h264.rs::create_sequence_header_annexb` 関数を削除 (本 issue 完了後の呼び出し側ゼロ)。
- `src/encoder/video_toolbox.rs` の import 整理。

### テスト追加・既存テスト維持

- `src/video/h264.rs::tests` を `pub(crate) mod tests` 化し、`SPS_320X240` / `SPS_1920X1080` を `pub(crate) const` 化。Annex-B 形式は `LazyLock<Vec<u8>>` で遅延初期化 (`SPS_320X240_ANNEXB` / `SPS_1920X1080_ANNEXB`)。decoder/openh264.rs / RTSP / SRT inbound のテストフィクスチャ重複を解消。
- `SpsBuilder` を `with_profile_idc(u32)` / `with_constraint_set_flags(u32)` / `with_chroma_format_idc(u32)` / `with_bit_depth_luma_minus8(u32)` / `with_bit_depth_chroma_minus8(u32)` で拡張。`build` 内のハードコード値をフィールド参照に置換。引数型は他の `with_*` メソッドと u32 に揃える。
- parse_sps 単体テスト 8 件追加 (Baseline / Main / High / High10 のフィールド検証、4 件の Err 化テスト)。
- `h264_sample_entry_from_sps_pps_lists` の単体テスト 7 件追加 (空 sps/pps の Err、Baseline / High の AvccBox マッピング、複数 SPS/PPS の保持、戻り値 frame_size、pps_list の NAL タイプ検査)。
- RTSP に `select_video_track_returns_err_on_sprop_with_broken_nal` を追加 (forbidden_zero_bit テスト)。
- 既存テストはすべて pass。Default build + clippy --deny warnings + fmt --check が通る (675 件)。

### コメント整理 (レビュー指摘の反映)

- encoder 3 経路の「VideoFrameSize 捨てる」コメント重複を解消し、`h264_sample_entry_from_sps_pps_lists` の docstring に集約。
- `parse_sps` 内の「`// 仕様 7.4.2.1.1`」4 連発を削除し、関数 docstring に集約。
- 自明な再説明コメント (「`as u16` キャストは情報損失しない」「move で受け取る」「Some/None セットを誤ることはない」等) を削除。
- SRT inbound テストコメントの古い関数名 `h264_sample_entry_from_annexb` を新関数名に更新。
- `expect()` メッセージの日本語混在を英語化。

### CHANGES.md

記載しない (issue 0043 の方針通り)。本 issue が触る 3 経路 (HLS / RTSP / SRT inbound) はいずれも `## develop` セクション内で初めて追加された未リリース機能で、最終的な diff として外部観測可能な変化が現れないため。closed 0030 / 0031 / 0032 / 0033 / 0034 / 0037 / 0044 と同方針。

### 残懸念 (別 issue 起票候補)

- `extract_video_dimensions` (`src/video/h264.rs`) の本番呼び出しゼロ。本 issue では「触らない」と明示済み。
- `src/rtmp/frame.rs::avc_sequence_header_to_sample_entry` の `chroma_format` / `bit_depth_*` の `None` 固定。RTMP 経路は SRT / RTSP とは制御フローが異なるため別 issue。
- `src/codec_string.rs::from_codec_pair` の `"avc1.42e01f"` 固定リテラル。代表値経路で本 issue 範囲外。
- video_toolbox の H.265 経路 (`h265::h265_sample_entry`) との対称性回復。H.265 経路の改修は別 issue。
- `pbt/tests/prop_h264_sps.rs` がクラッシュフリー専用テスト規約と乖離 + 本番経路から外れている件。
- `src/srt/inbound_endpoint.rs` の `last_video_sample_entry` / `last_video_frame_size` の二重保持 (既存問題、本 issue で導入したものではない)。
- `src/video/h264.rs` のテストファイル肥大 (累積的な改善対象)。
