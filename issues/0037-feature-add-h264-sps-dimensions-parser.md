# H.264 SPS から解像度（width / height）を抽出してサンプルエントリーと VideoFrame.size に反映する

- Priority: Medium
- Created: 2026-06-16
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/add-h264-sps-dimensions-parser
- Polished: {YYYY-MM-DD}
- Reporter: @sile

## 目的

現在 `src/video/h264.rs` の `h264_sample_entry_from_annexb(width, height, data)` は呼び出し側から渡された `width` / `height` をそのまま `SampleEntry::Avc1` に埋め込んでおり、Annex-B ストリーム（SPS）から実際の解像度を抽出する機能を持たない。
このため `src/srt/inbound_endpoint.rs:937` の SRT inbound 経路では `h264_sample_entry_from_annexb(0, 0, &pending.data)` と固定値 0 を渡しており、MP4 出力時にサンプルエントリーの `width` / `height` が 0 になる。
加えて `src/srt/inbound_endpoint.rs:952` で `VideoFrame.size: None` を渡しており、`src/decoder.rs:580-584` のロジックで `frame.size.is_none()` のフレームは VideoToolbox デコーダから除外される（H.264 だけ size が要求される点に注意）。SRT inbound 経由の H.264 では VideoToolbox 経路が常にスキップされる挙動になっている。
SRT / MPEG-TS では PMT / PAT に解像度情報を持たないため、Annex-B ES 内の SPS から `pic_width_in_mbs_minus1` / `pic_height_in_map_units_minus1` / `frame_cropping_flag` 等を Exp-Golomb パースして解像度を取得する必要がある。
本 issue では H.264 SPS から解像度を抽出する関数を `src/video/h264.rs` に共有ユーティリティとして追加し、SRT inbound 経路で実際の解像度を `SampleEntry::Avc1` と `VideoFrame.size` の両方に反映できるようにする。

## 優先度根拠

- Medium。
- SRT inbound 経由で受信した H.264 映像の MP4 出力で解像度メタデータが 0 になるという正確性の問題があり、下流（プレイヤー / トランスコーダ）の挙動に影響する可能性がある。
- 同経路で `VideoFrame.size` が None のままなのでデコーダ選択（VideoToolbox）が常にスキップされ、利用可能な高速経路を活かせていない。
- 一方で、SRT inbound 自体が直近で追加された経路（issue 0033 / PR #271）であり、現時点で本番運用に直接ブロッカーになっているわけではない。
- `src/srt/inbound_endpoint.rs:931-934` のコメントに「将来の改善余地」と明示されており、0033 close 時点で意図的に持ち越された残課題である。

## 現状

### 解像度を SPS パースしていない呼び出し箇所

- `src/srt/inbound_endpoint.rs:937` （`sample_entry` 側）
  ```rust
  // width / height は 0 で渡す。
  // `h264_sample_entry_from_annexb` は引数をそのまま埋め込むだけで SPS パースはしないため、
  // Annex-B から実値を取り出すには呼び出し側で SPS Exp-Golomb パースを実装する必要がある（将来の改善余地）。
  // SPS / PPS 不在 IDR や破損 NAL は同関数が Err を返す。
  // 正常な H.264 ストリームは IDR に SPS / PPS を inline するため、
  // Err はエンコーダ側の異常とみなしてそのまま伝播し、接続を打ち切る。
  let entry = crate::video::h264::h264_sample_entry_from_annexb(0, 0, &pending.data)?;
  ```
- `src/srt/inbound_endpoint.rs:952` （`VideoFrame.size` 側）
  ```rust
  Ok(Some(TsSample::Video(crate::VideoFrame {
      data: pending.data,
      format: crate::video::VideoFormat::H264AnnexB,
      keyframe,
      size: None, // ← SPS パースで埋めたい
      timestamp,
      sample_entry: self.last_video_sample_entry.clone(),
  })))
  ```

### `VideoFrame.size: None` の副作用

- `src/decoder.rs:580-584`
  ```rust
  // width/height が必須なので、frame.size が無い入力では選択しない
  && frame.size.is_none()
  ```
  H.264 で `frame.size` が None だと VideoToolbox デコーダがスキップされる。SRT inbound 経由の H.264 はこの分岐に常時引っかかる。

### 解像度を呼び出し側から渡せる箇所（本 issue の対象外）

- `src/encoder/nvcodec.rs:50`: encoder の `options.width.get()` / `options.height.get()` から取得済み。
- `src/encoder/openh264.rs:62`: 入力 `frame.size()` から取得済み。
- `src/decoder/openh264.rs:167`, `:201`: decoder 側で取得済み。

これらの呼び出し側は既に解像度を知っているため、SPS パースは不要で挙動を変えない。

### 関数シグネチャ

- `src/video/h264.rs:87-91`
  ```rust
  pub fn h264_sample_entry_from_annexb(
      width: usize,
      height: usize,
      data: &[u8],
  ) -> crate::Result<SampleEntry> {
  ```

### Exp-Golomb パーサの有無

- リポジトリ内に Exp-Golomb パーサは存在しない（`grep -rn "exp_golomb\|expgolomb\|Exp-Golomb" --include="*.rs"` でヒット 0 件）。
- H.265 SPS パースを伴う既存実装も存在しない。

## 設計方針

### スコープ

- 本 issue は **H.264 のみ** を対象とする。H.265 (`h265_sample_entry_from_annexb`) の SRT inbound 経路は現状存在しないため、対象外とする。
- 解像度 (`width` / `height`) の抽出のみを対象とする。プロファイル / レベル / chroma_format / bit_depth 等の他のパラメータは本 issue では扱わない（`h264_sample_entry_from_annexb` 内に既に `TODO: 実際の値に合わせる` コメントとして残っているが、別 issue として切り出す）。
- 反映先は **SRT inbound 経路の `SampleEntry::Avc1` と `VideoFrame.size` の両方**。SPS パース関数は `src/video/h264.rs` に共有ユーティリティとして置き、将来 RTSP / 他経路でも再利用できる形にする。
- **RTSP subscriber (`src/rtsp/subscriber.rs:637` の `size: None`) は対象外**。`0032-feature-add-rtsp-annexb-video-sample-entry.md` で別途扱う（本 issue で追加する SPS パーサを再利用する想定）。
- **mp4 reader は対象外**。mp4 reader (`src/sora/recording_mp4_reader.rs:160`) は avcC ボックスから解像度を取得済みで SPS パースを必要としない。

### モジュール構成

- `src/video/h264.rs` 内に Exp-Golomb パーサと SPS 解像度抽出関数を追加する。新規ファイルは作らない。
- パーサは H.264 SPS のみ対応で十分。汎用化（H.265 対応 / モジュール分離）は需要が出てから検討する（YAGNI）。

### 関数設計

- 追加する関数:
  ```rust
  /// Annex-B 形式の SPS NAL ユニットから width / height を抽出する。
  /// 入力は emulation prevention byte (0x000003) を含む生の SPS バイト列を想定する。
  pub fn extract_dimensions_from_sps(sps: &[u8]) -> crate::Result<(usize, usize)>;
  ```
- 内部処理:
  1. RBSP 抽出: emulation prevention byte (`0x00 0x00 0x03`) を除去して RBSP を得る。
  2. ビットリーダで以下を順に読み出す（ITU-T H.264 仕様 7.3.2.1.1 / 7.4.2.1.1）:
     - `profile_idc` (u8)
     - `constraint_set*_flag` 群 + `reserved_zero_2bits` (u8)
     - `level_idc` (u8)
     - `seq_parameter_set_id` (ue(v))
     - `profile_idc` が 100/110/122/244/44/83/86/118/128/138/139/134/135 の場合は `chroma_format_idc` (ue(v)) ほかの追加フィールド群（仕様準拠で読み飛ばす）
     - `log2_max_frame_num_minus4` (ue(v))
     - `pic_order_cnt_type` (ue(v)) と関連フィールド群
     - `max_num_ref_frames` (ue(v))
     - `gaps_in_frame_num_value_allowed_flag` (u1)
     - `pic_width_in_mbs_minus1` (ue(v))
     - `pic_height_in_map_units_minus1` (ue(v))
     - `frame_mbs_only_flag` (u1)
     - `frame_cropping_flag` (u1) と `frame_crop_*_offset` (ue(v) x 4)
  3. 算出:
     - `width_mb = pic_width_in_mbs_minus1 + 1`
     - `height_map_units = pic_height_in_map_units_minus1 + 1`
     - `raw_width = width_mb * 16`
     - `raw_height = height_map_units * 16 * (2 - frame_mbs_only_flag)`
     - `frame_cropping_flag == 1` の場合、`chroma_format_idc` から決まる SubWidthC / SubHeightC を考慮して
       `width = raw_width - SubWidthC * (frame_crop_left_offset + frame_crop_right_offset)`
       `height = raw_height - SubHeightC * (frame_crop_top_offset + frame_crop_bottom_offset)`
     - `frame_cropping_flag == 0` の場合は `(raw_width, raw_height)` をそのまま返す。
  4. `chroma_format_idc` が SPS に含まれない場合は仕様デフォルト (`chroma_format_idc == 1` 相当の SubWidthC=2 / SubHeightC=2) を用いる。

- `h264_sample_entry_from_annexb` の挙動変更:
  - 引数の `width` / `height` を残しつつ、両方が 0 の場合は SPS から抽出した値を使うようにする、ではなく、**引数を削除して常に SPS から抽出する** 方針を採る。
  - 既存呼び出し側（encoder / decoder）は既に手元で width / height を知っているが、SPS にも同じ情報が入っているはずなので、関数内で一元化する方が一貫性がある。
  - これにより `src/srt/inbound_endpoint.rs:937` の特殊扱い（0 を渡す）が不要になる。
  - **ただし呼び出し側の挙動変更を最小化したい場合は、引数を残して 0 が渡されたときのみ SPS パースする選択肢もある**。これは設計レビュー時に決定する。

  → 本 issue では **後者（引数を残し、両方 0 のとき SPS から抽出）** を採用する。理由:
    - encoder / decoder の既存挙動を一切変えない（回帰リスクを最小化）。
    - 0033 で固定した「SPS / PPS 不在 IDR は Err 伝播」の挙動を維持できる。
    - SPS パースが失敗した場合のフォールバックを「呼び出し側で値を持っているなら渡す」という形で残せる。

### エラー処理

- SPS が見つからない場合は既存の `missing H.264 SPS` Err をそのまま返す。
- SPS のパースに失敗した場合は `invalid H.264 SPS: <理由>` という Err を返す。
- 呼び出し側 (`src/srt/inbound_endpoint.rs`) は既存の通り Err を伝播して接続を打ち切る。

## 完了条件

- `src/video/h264.rs` に SPS 解像度抽出関数 (`extract_dimensions_from_sps` 等) と Exp-Golomb パーサが追加されている。
- `h264_sample_entry_from_annexb` が `width == 0 && height == 0` のとき SPS から実値を抽出する挙動になっている。
- `src/srt/inbound_endpoint.rs:937` の呼び出しが従来通り `(0, 0, ...)` のままで、結果として MP4 出力のサンプルエントリーに実解像度が反映される。
  - もしくは、呼び出し側を明示化する場合は `(0, 0, ...)` から実値抽出を意図する別形式に変更する（設計時に決定）。
- `src/srt/inbound_endpoint.rs:952` の `VideoFrame.size: None` が、同じ SPS パース結果から得た `Some(VideoFrameSize { width, height })` に置き換えられている。
  - SPS パースは IDR の build_video_sample で 1 度実施し、`sample_entry` と `VideoFrame.size` の両方で同じ値を共有する（二度パースしない）。
- 追加した SPS パーサに対して、典型解像度（320x240 / 640x480 / 1280x720 / 1920x1080）と frame_cropping_flag 有無の両ケースを含む単体テストが追加されている。
- 既存の SRT inbound テスト（`src/srt/inbound_endpoint.rs:1394` 等）が引き続きパスし、必要に応じて「sample_entry の `Avc1Box.visual.width` / `.height` と、`VideoFrame.size` の両方が SPS 由来の実値になる」ことを確認するアサーションが追加されている。
- `cargo test` / `cargo clippy` / `cargo fmt` がパスする。

## 解決方法

### 実装手順

1. `src/video/h264.rs` に以下を追加する:
   - `struct H264BitReader<'a>`（または equivalent）: バイト列から 1 ビット単位で読み出すリーダ。
   - `fn read_ue(&mut self) -> crate::Result<u32>`: Exp-Golomb unsigned 復号。
   - `fn rbsp_from_sps(sps: &[u8]) -> Vec<u8>`: emulation prevention byte 除去（0x000003 → 0x0000）。
   - `pub fn extract_dimensions_from_sps(sps: &[u8]) -> crate::Result<(usize, usize)>`: 上記設計方針に従って width / height を返す。
2. `h264_sample_entry_from_annexb` を以下のように修正する:
   ```rust
   pub fn h264_sample_entry_from_annexb(
       width: usize,
       height: usize,
       data: &[u8],
   ) -> crate::Result<SampleEntry> {
       // ... SPS / PPS 抽出 ...
       let (width, height) = if width == 0 && height == 0 {
           extract_dimensions_from_sps(&sps_list[0])?
       } else {
           (width, height)
       };
       // ...
   }
   ```
3. `src/srt/inbound_endpoint.rs` の `build_video_sample`（IDR 検出箇所）を以下の構造に変更する:
   - IDR を検出したら、`H264AnnexBNalUnits` を一度走査して SPS NAL ユニットを取り出す。
   - SPS から `extract_dimensions_from_sps` で `(width, height)` を取得し、それを `sample_entry` 構築と `VideoFrame.size` の両方に流す。
   - 同一の SPS パース結果を共有することで、二度パースを避ける。
   - SPS パース or `h264_sample_entry_from_annexb` が Err を返した場合は、従来通りそのまま伝播して接続を打ち切る（0033 で確定した fail-fast 方針を維持）。
   - `:931-934` のコメントを「SPS から実値を抽出し sample_entry と VideoFrame.size の両方に反映する」旨に書き換える。
4. テストを追加する:
   - `src/video/h264.rs` 内の `#[cfg(test)] mod tests`:
     - 既知の SPS バイト列に対して `extract_dimensions_from_sps` が正しい解像度を返すこと（320x240 / 640x480 / 1280x720 / 1920x1080 / frame_cropping_flag 有り 1920x1080）。
     - 不正な SPS（短すぎる / 中断する）に対して Err を返すこと。
   - SRT inbound テスト:
     - 既存の `src/srt/inbound_endpoint.rs:1394` 周辺のテストに対し、`build_video_sample` が返す `sample_entry` の `Avc1Box.visual.width` / `.height` と、`VideoFrame.size` の両方が SPS から復元された値（テストフィクスチャの想定解像度）になることをアサートする。

### 影響範囲

- `src/video/h264.rs`: 関数追加 + `h264_sample_entry_from_annexb` の挙動分岐追加。
- `src/srt/inbound_endpoint.rs`: `build_video_sample` の IDR 検出パスで SPS パース結果を `sample_entry` と `VideoFrame.size` の両方に反映する。コメントも書き換える。
- encoder / decoder 側 (`src/encoder/openh264.rs`, `src/encoder/nvcodec.rs`, `src/decoder/openh264.rs`): 引数に 0 以外を渡しているため挙動変更なし。

### 非対象

- H.265 SPS の解像度パース。SRT / 他経路で H.265 Annex-B 入力が増えたら別 issue で対応する。
- profile_idc / level_idc の SPS 反映（`h264_sample_entry_from_annexb` 内の `TODO: 実際の値に合わせる`）。
- chroma_format / bit_depth_luma / bit_depth_chroma の SPS 反映。
- RTSP subscriber (`src/rtsp/subscriber.rs:637` の `size: None`)。`0032-feature-add-rtsp-annexb-video-sample-entry.md` で別途扱う（本 issue の SPS パーサを再利用する想定）。
- mp4 reader 経路。`src/sora/recording_mp4_reader.rs:160` は avcC ボックスから解像度を取得済みで SPS パースを必要としない。

### テスト戦略

- SPS のバイト列は実機（Sora や OBS 等）から取得した実サンプルを `tests/fixtures/` 等に追加し、それを使って検証する。モック / ハンドコーディングした SPS は仕様の細部を取り違える可能性があるため避ける（CLAUDE.md「モックやスタブは絶対に利用しないこと」に準拠）。
- 既存の SRT 経路の PBT / フィクスチャ（`SrtTsDemuxer` のテストヘルパ）に解像度アサートを追加する形が望ましい。
