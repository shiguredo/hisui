# H.264 SPS から解像度（width / height）を抽出してサンプルエントリーと VideoFrame.size に反映する

- Priority: Low
- Created: 2026-06-16
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/add-h264-sps-dimensions-parser
- Polished: 2026-06-16
- Reporter: @sile

## 目的

SRT MPEG-TS 入力の H.264 Annex-B 映像経路で、SPS から解像度（width / height）を抽出して `SharedSampleEntry` (`SampleEntry::Avc1`) と `VideoFrame.size` の両方に反映する。

現状は IDR 検出時に `src/srt/inbound_endpoint.rs:937` で `h264_sample_entry_from_annexb(0, 0, &pending.data)` と固定値 0 を渡しているため、MP4 出力時のサンプルエントリーで `width` / `height` が 0 になる。MPEG-TS では PMT / PAT に解像度情報がないため、Annex-B ES 内の SPS から ITU-T H.264 仕様 7.3.2.1.1 / 7.4.2.1.1 に従って Exp-Golomb で解像度を抽出する必要がある。

合わせて、本 issue で追加する SPS パース関数を `src/video/h264.rs` の共有ユーティリティとして置き、open 中の `0032-feature-add-rtsp-annexb-video-sample-entry.md`（RTSP 経路）でも再利用できる形にする（0032 でも同じく `0` を渡す前提で設計されている）。

## カテゴリ判定

ブランチ命名は `feature/add-h264-sps-dimensions-parser`（`add` カテゴリ）。主目的は H.264 SPS 解像度抽出機能（共有ユーティリティ）の新規追加。SRT inbound 経路への適用と既存テストフィクスチャ差し替えは、新機能を機能させるための不可分の整理として同 issue に含める。

注: `VideoFrame.size` を埋める変更は「H.264 で VideoToolbox スキップを回避する」というバグ修正ではない（`src/decoder.rs:579-587` の skip 条件は `matches!(codec, CodecName::Vp9 | CodecName::Av1) && frame.size.is_none()` で H.264 は対象外）。`VideoFrame.size` を埋める動機は「`sample_entry` 不変条件 docstring との整合」「将来 RTSP / RTMP 等で SPS パーサを再利用する際の API 一貫性」に集約され、SPS パース機能追加の自然な副産物として add カテゴリ内に収まる。

## 優先度根拠

Low。本 issue は予防的整備（broken window 解消）として位置付ける:

- (a) MP4 出力サンプルエントリーの `width` / `height` が 0 になる正確性問題は存在するが、SRT inbound 経路は直近で追加された（issue 0033 / PR #271）ばかりで本番運用への直接的なブロッカーではない。`src/srt/inbound_endpoint.rs:931-934` のコメントに「将来の改善余地」と明示されており、0033 close 時点で意図的に持ち越された残課題。
- (b) `src/video.rs:51-57` の `VideoFrame.sample_entry` の不変条件 docstring には「現時点で未適用の経路: WebM リーダー、rtsp の Annex-B 映像」とあり、SRT inbound は 0033 で既に Some 化済み。一方で `VideoFrame.size` 側（`src/video.rs:49` の `pub size: Option<VideoFrameSize>`）には明示的な不変条件記述がないものの、`src/encoder/video_toolbox.rs:172-175` 等で「圧縮 frame は size: Some」が事実上の慣習となっている。SPS から解像度を抽出することで「SRT inbound 経路の H.264 フレームは sample_entry と size の両方を Some で持つ」整合した状態を作れる。
- (c) `0032-feature-add-rtsp-annexb-video-sample-entry.md`（open）の設計方針が「width / height は RTMP 実装と同様に 0 で構築する。SPS 内 Exp-Golomb パースによる解像度抽出は別 issue 扱い」と本 issue を前提として参照している。0032 着手前に SPS パーサを整備しておく必要がある。

## 現状

行番号は HEAD（develop = 5cc5dab2）時点。実装着手時は grep で再特定する。

### 解像度を SPS パースしていない SRT inbound 呼び出し箇所

- `src/srt/inbound_endpoint.rs:930-938`（`build_video_sample` の IDR 検出箇所）:

  ```rust
  if keyframe {
      // width / height は 0 で渡す。
      // `h264_sample_entry_from_annexb` は引数をそのまま埋め込むだけで SPS パースはしないため、
      // Annex-B から実値を取り出すには呼び出し側で SPS Exp-Golomb パースを実装する必要がある（将来の改善余地）。
      // SPS / PPS 不在 IDR や破損 NAL は同関数が Err を返す。
      // 正常な H.264 ストリームは IDR に SPS / PPS を inline するため、
      // Err はエンコーダ側の異常とみなしてそのまま伝播し、接続を打ち切る。
      let entry = crate::video::h264::h264_sample_entry_from_annexb(0, 0, &pending.data)?;
      self.last_video_sample_entry = Some(crate::sample_entry::SharedSampleEntry::new(entry));
  }
  ```

- `src/srt/inbound_endpoint.rs:948-955`（`VideoFrame` 構築箇所）:

  ```rust
  Ok(Some(TsSample::Video(crate::VideoFrame {
      data: pending.data,
      format: crate::video::VideoFormat::H264AnnexB,
      keyframe,
      size: None,
      timestamp,
      sample_entry: self.last_video_sample_entry.clone(),
  })))
  ```

### `h264_sample_entry_from_annexb` 関数シグネチャと内部処理

- `src/video/h264.rs:87-129`:

  ```rust
  pub fn h264_sample_entry_from_annexb(
      width: usize,
      height: usize,
      data: &[u8],
  ) -> crate::Result<SampleEntry> {
      // H.264 ストリームから SPS と PPS と取り出す
      let mut sps_list = Vec::new();
      let mut pps_list = Vec::new();
      for nalu in H264AnnexBNalUnits::new(data) {
          let nalu = nalu?;
          match nalu.ty {
              H264_NALU_TYPE_SPS => sps_list.push(nalu.data.to_vec()),
              H264_NALU_TYPE_PPS => pps_list.push(nalu.data.to_vec()),
              _ => {}
          }
      }
      // SPS / PPS の存在検査と SampleEntry::Avc1 構築（width / height はそのまま埋め込む）
  }
  ```

  `H264NalUnit.data`（`src/video/h264.rs:64-68`）は **NAL ヘッダ 1 バイト（forbidden_zero_bit + nal_ref_idc + nal_unit_type）を含む raw NAL バイト列** であり、後段の RBSP 抽出ではこの 1 バイトをスキップする必要がある。

### Exp-Golomb パーサの有無

- リポジトリ内に Exp-Golomb (ue(v) / se(v)) パーサは存在しない（`grep -rn "exp_golomb\|expgolomb\|Exp-Golomb" --include="*.rs"` でヒット 0 件）。
- `src/rtsp/subscriber.rs` 周辺の `BitReader` は au-headers 用で SPS パースには転用できない。

### 既存 SRT inbound テストフィクスチャ

- `src/srt/inbound_endpoint.rs:1326` 付近の `SPS_INITIAL` は 9 バイト全体（start code 4 バイト + payload 5 バイト `0x67 0x42 0xc0 0x1e 0xab`）。同じく `:1329` 付近の `SPS_UPDATED` も短い payload で構成されており、両者とも `pic_width_in_mbs_minus1` まで届かない（ビット位置がそこに到達する前に終端する）。本 issue の SPS 解像度抽出を有効化すると、現フィクスチャでは Err を返して既存テストが全壊するため、両フィクスチャの差し替えが必須。
- 0033 のテストフィクスチャは `mod tests` 内に **`const SPS_INITIAL: [u8; 9] = [0x00, 0x00, 0x00, 0x01, ...]` のように直接埋め込む** 方式で、外部ファイル (`tests/fixtures/`) は使っていない。本 issue でも同じ方式に揃える。

### 解像度を呼び出し側から渡せる箇所（本 issue で挙動を変えない箇所）

- `src/encoder/nvcodec.rs:50`: encoder の `options.width.get()` / `options.height.get()` を渡す。
- `src/encoder/openh264.rs:62`: 入力 `frame.size()` から渡す。
- `src/decoder/openh264.rs:167`, `:201`: decoder 側で取得した値を渡す。

これらは `width != 0` / `height != 0` を渡す。本 issue は `h264_sample_entry_from_annexb` のシグネチャと挙動を変えず、呼び出し側で SPS パースして実値を渡す方式を採るため、これらの経路に影響しない。

## 設計方針

### スコープ

- 本 issue は **H.264 のみ** を対象とする。H.265 (`h265_sample_entry_from_annexb`) の SRT inbound 経路は現状存在しないため、対象外。
- 解像度（`width` / `height`）の抽出のみを対象とする。プロファイル / レベル / chroma_format / bit_depth 等の他のパラメータは本 issue では扱わない。
- 反映先は **SRT inbound 経路の `SampleEntry::Avc1` と `VideoFrame.size` の両方**。SPS パース関数は `src/video/h264.rs` に共有ユーティリティとして置き、0032 (RTSP) で再利用できる形にする。

### モジュール構成

- `src/video/h264.rs` 内に Exp-Golomb パーサと SPS 解像度抽出関数を追加する。新規ファイルは作らない。
- パーサは H.264 SPS のみ対応で十分。汎用化（H.265 対応 / モジュール分離）は需要が出てから検討する（YAGNI）。

### 関数設計

#### 公開関数と入力契約

```rust
/// H.264 SPS NAL ユニットから width / height を抽出する。
///
/// 入力 `sps` は `H264AnnexBNalUnits` が返す `H264NalUnit.data` をそのまま渡す形式で、
/// 先頭 1 バイトに NAL ヘッダ（forbidden_zero_bit + nal_ref_idc + nal_unit_type = 7）を含む。
/// 内部で先頭 1 バイトをスキップしたうえで RBSP 抽出（emulation prevention byte 除去）を行う。
pub fn extract_dimensions_from_sps(sps: &[u8]) -> crate::Result<(usize, usize)>;
```

- 入力に NAL ヘッダを含める理由: 呼び出し側（`H264AnnexBNalUnits::new(data)` の `H264NalUnit.data` をそのまま渡す）と統一でき、SRT / RTSP / RTMP 等の経路で同じ呼び出し方ができる。
- 内部で先頭バイトに対し `debug_assert_eq!(sps[0] & 0x1F, H264_NALU_TYPE_SPS)` を入れる。release ビルドでは検証を省くが、開発時に誤った NAL を渡すバグを早期検出できる。
- 抽出結果が `width == 0` または `height == 0` になった場合（cropping 適用で 0 や負になるパス）は `invalid H.264 SPS: zero or negative dimensions` 相当の Err を返す。`VideoFrameSize::new` (`src/video.rs:23-29`) に 0 を渡して内部 Err を出させるのではなく、SPS パーサ側で発生源情報を含む Err にして呼び出し側で扱いやすくする。

#### 内部処理（ITU-T H.264 仕様 7.3.2.1.1 / 7.4.2.1.1 準拠）

1. NAL ヘッダ 1 バイトをスキップする（先頭バイトの下位 5 ビットが SPS の NAL unit type (7) であることは呼び出し側で保証済み）。
2. RBSP 抽出: `0x00 0x00 0x03` を `0x00 0x00` に置換して emulation prevention byte を除去する。
3. ビットリーダで以下を順番に **全フィールド消費** する（途中で位置がずれると `pic_width_in_mbs_minus1` 以降が壊れる）:
   - `profile_idc` (u(8))
   - `constraint_set0_flag` 〜 `constraint_set5_flag` + `reserved_zero_2bits` (u(8))
   - `level_idc` (u(8))
   - `seq_parameter_set_id` (ue(v))
   - **`profile_idc ∈ {100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135}` のとき** (High 系プロファイル群。ITU-T H.264 仕様 7.3.2.1.1 (Sequence parameter set data syntax) の `if (profile_idc == ...)` 条件節をそのまま列挙したもの。本 issue では版を ITU-T H.264 (2017/06) 想定で固定し、版差により値が増減する可能性は本実装スコープ外):
     - `chroma_format_idc` (ue(v))
     - `chroma_format_idc == 3` のとき `separate_colour_plane_flag` (u(1))
     - `bit_depth_luma_minus8` (ue(v))
     - `bit_depth_chroma_minus8` (ue(v))
     - `qpprime_y_zero_transform_bypass_flag` (u(1))
     - `seq_scaling_matrix_present_flag` (u(1))
       - 1 のとき: `chroma_format_idc == 3` で 12 回、それ以外で 8 回、`seq_scaling_list_present_flag` (u(1)) を読み、立っているスロットで `scaling_list()` サブルーチン（size 16 または 64 の要素ごとに `delta_scale` (se(v)) を読む。仕様 7.3.2.1.1.1）を実行
   - `log2_max_frame_num_minus4` (ue(v))
   - `pic_order_cnt_type` (ue(v))
     - `== 0` のとき: `log2_max_pic_order_cnt_lsb_minus4` (ue(v))
     - `== 1` のとき: `delta_pic_order_always_zero_flag` (u(1))、`offset_for_non_ref_pic` (se(v))、`offset_for_top_to_bottom_field` (se(v))、`num_ref_frames_in_pic_order_cnt_cycle` (ue(v)) と要素数ぶんの `offset_for_ref_frame[i]` (se(v))
     - `== 2` のとき: 追加読み出しなし
   - `max_num_ref_frames` (ue(v))
   - `gaps_in_frame_num_value_allowed_flag` (u(1))
   - `pic_width_in_mbs_minus1` (ue(v))
   - `pic_height_in_map_units_minus1` (ue(v))
   - `frame_mbs_only_flag` (u(1))
   - `frame_cropping_flag` (u(1))
     - 1 のとき `frame_crop_left_offset` / `frame_crop_right_offset` / `frame_crop_top_offset` / `frame_crop_bottom_offset` を ue(v) で 4 個読む

   必要なプリミティブ:
   - `u(n)` (n ビット符号なし整数)
   - `ue(v)` (符号なし Exp-Golomb、仕様 9.1)
   - `se(v)` (符号付き Exp-Golomb、仕様 9.1.1。`offset_for_*` と `delta_scale` で必須)
   - `scaling_list()` 読み飛ばし（実値は使わないが位置を進める必要がある）

4. 算出（仕様 7.4.2.1.1。`CropUnitX` / `CropUnitY` は同節で正式に定義される）:
   - `chroma_array_type` の決定:
     - High 系プロファイル群以外で `chroma_format_idc` が SPS に含まれない: `chroma_format_idc = 1` とみなす（Baseline / Main / Extended プロファイルでは仕様デフォルトとして 4:2:0 固定）
     - `separate_colour_plane_flag == 0` のとき: `chroma_array_type = chroma_format_idc`
     - `separate_colour_plane_flag == 1` のとき: `chroma_array_type = 0`（仕様 7.4.2.1.1）
   - `(CropUnitX, CropUnitY)` の決定:
     - `chroma_array_type == 0` (monochrome または separate_colour_plane_flag == 1): `(1, 2 - frame_mbs_only_flag)`（仕様 6.2 で SubWidthC / SubHeightC は未定義、仕様 7.4.2.1.1 で CropUnitX / CropUnitY が直接定義される）
     - `chroma_array_type != 0`: `(SubWidthC, SubHeightC * (2 - frame_mbs_only_flag))` で、SubWidthC / SubHeightC は以下の表で決まる:
       - `chroma_array_type == 1` (4:2:0): (SubWidthC, SubHeightC) = (2, 2)
       - `chroma_array_type == 2` (4:2:2): (SubWidthC, SubHeightC) = (2, 1)
       - `chroma_array_type == 3` (4:4:4): (SubWidthC, SubHeightC) = (1, 1)
   - `raw_width = (pic_width_in_mbs_minus1 + 1) * 16`（`checked_mul` でオーバーフローを Err 化）
   - `raw_height = (pic_height_in_map_units_minus1 + 1) * 16 * (2 - frame_mbs_only_flag)`（同上）
   - `frame_cropping_flag == 1` のとき:
     - `width = raw_width - CropUnitX * (frame_crop_left_offset + frame_crop_right_offset)`
     - `height = raw_height - CropUnitY * (frame_crop_top_offset + frame_crop_bottom_offset)`
     - `checked_sub` で負（アンダーフロー）にならないことを確認し、なれば Err
   - `frame_cropping_flag == 0` のとき: `(raw_width, raw_height)` をそのまま返す

#### 二度パース回避と `h264_sample_entry_from_annexb` の扱い

- SPS パースは `src/srt/inbound_endpoint.rs` の `build_video_sample` で 1 回だけ実施する。抽出した `(width, height)` を `h264_sample_entry_from_annexb(width, height, &pending.data)` の引数として渡すことで、関数内で再度 SPS をパースする必要をなくす。
- `h264_sample_entry_from_annexb` のシグネチャと既存挙動（引数として渡された width / height をそのまま埋め込む）は **変更しない**。`(0, 0)` をセンチネルとして関数内で分岐させる案は採用しない（マジック値は API として脆く、`encoder/decoder` 等の既存呼び出し側に新しい意味論を持ち込まずに済むため）。
- 関数内 `:103-104` の `if sps_list.is_empty()` SPS 不在 Err は維持する。SRT inbound 経路では `build_video_sample` 側で先に SPS 抽出してから関数を呼ぶため、関数内の SPS 不在 Err は **到達不能（defensive な二重検査）** になるが、encoder / decoder 経路では引き続き Err 経路として有効。関数シグネチャを変えないことで両経路を統一する。
- 結果として encoder / decoder の既存呼び出し（`src/encoder/nvcodec.rs:50` 等）は引数として 0 以外を渡し続けるため挙動変更なし。SRT inbound では 0 を渡す処理が完全に消える。

### エラー処理

- SPS が IDR 内に存在しない場合: `build_video_sample` 側で SPS 検索後に判定し、`missing H.264 SPS` 相当の Err を返して接続を打ち切る（0033 で確定した fail-fast 方針を維持）。
- SPS のパースに失敗した場合: `invalid H.264 SPS: <理由>` Err を返す。`<理由>` には次のケースが含まれる:
  - ビット不足（バッファ末尾を超えた読み出し）
  - `pic_width_in_mbs_minus1` / `pic_height_in_map_units_minus1` / crop offset が `usize` 計算でオーバーフロー (`checked_mul` / `checked_add` 失敗)
  - cropping 適用で width / height が 0 以下になる (`checked_sub` 失敗)
  - cropping 後の width / height が 0 になる
- Err パターンの区別方針: 上記ケースは `crate::Error::new(format!("invalid H.264 SPS: ..."))` の **人間可読な文字列で区別** し、enum などで型分けはしない（ログでケース毎に切り分けるニーズが現状ないため。必要が出れば将来の別 issue で構造化する）。
- SPS パース成功後の `h264_sample_entry_from_annexb` Err（PPS 不在 等）は従来通り伝播。
- `build_video_sample` 失敗時の `self.last_video_sample_entry` の扱い: 失敗は呼び出し側（`SrtTsDemuxer` の上位）で接続を打ち切る前提のため、`last_video_sample_entry` を `None` に巻き戻すなどの後始末は不要（fail-fast 方針）。前回成功時の値が残っていても以降のフレームは流れない。

### SPS 走査方針（複数 SPS 時の採用ルール）

- IDR 内 PES に複数の SPS NAL ユニットが含まれる場合、本 issue では **最初に出現した SPS** を解像度抽出対象とする。これは `h264_sample_entry_from_annexb` 内部の `sps_list` の先頭要素と一致するため、関数間で抽出対象が一貫する。
- 複数 SPS が異なる解像度を持つケースは Hisui の入力前提（Sora / OBS 出力）では発生しない想定であり、本 issue では深追いしない（必要が出れば将来別 issue で扱う）。`h264_sample_entry_from_annexb` 側の `sps_list` には全件を保持し続けるため後方互換性は保たれる。

## 完了条件

- `src/video/h264.rs` に SPS 解像度抽出関数 `extract_dimensions_from_sps`、Exp-Golomb (`ue(v)` / `se(v)`) パーサ、scaling_list 読み飛ばし、RBSP 抽出ヘルパが追加されている。
- `src/srt/inbound_endpoint.rs` の `build_video_sample` が IDR 検出時に SPS を 1 回抽出し、その結果を `h264_sample_entry_from_annexb` の引数と `VideoFrame.size` の両方に流す構造になっている。
- `:931-936` のコメントが「SPS から実値を抽出して sample_entry と VideoFrame.size の両方に反映する」旨に書き換えられている。
- 既存テストフィクスチャ `SPS_INITIAL` / `SPS_UPDATED`（`src/srt/inbound_endpoint.rs:1326` / `:1329`）の双方が `pic_width_in_mbs_minus1` / `pic_height_in_map_units_minus1` / `frame_mbs_only_flag` / `frame_cropping_flag` まで到達する完全な SPS バイト列に差し替えられている。
  - `SPS_INITIAL` の想定解像度は **1920x1080**（固定）。
  - `SPS_UPDATED` の想定解像度は **1280x720**（固定）。SPS_INITIAL と異なる値であることがテストの肝。
  - 既存テスト（PPS 不在 Err / SPS 不在 Err / 後置 SPS/PPS / 通常 / mid-stream SPS 更新）が新フィクスチャでパスする。
  - 影響テスト一覧（`SPS_INITIAL` / `SPS_UPDATED` を参照する箇所）: 5 件（`:1347` / `:1400` / `:1433` / `:1458` / `:1470` 付近）。
- `srt_h264_updates_sample_entry_on_mid_stream_sps_change` (`src/srt/inbound_endpoint.rs:1454-1486`) の本体アサートは現状の `entry2.changed_since(Some(&entry1))` を維持する（全体差分検証）。「`width` / `height` が初期値と別の値になる」直接アサートは下記の新規アサート要件で別途担保する:
  - 「`build_video_sample` が返す `sample_entry`（`Avc1Box.visual.width` / `.height`）と `VideoFrame.size` の両方が SPS 由来の実値（`SPS_INITIAL` 由来なら 1920x1080、`SPS_UPDATED` 由来なら 1280x720）になる」ことを `assert_eq!` で確認する新規テスト or 既存テスト追記を行う。`SampleEntry::Avc1` からの値取り出しは `Avc1Box.visual.width` / `.visual.height` を直接参照する（`extract_video_dimensions` 経由でも同値なので実装者裁量）。
- SPS パーサの単体テストが追加されている:
  - 解像度 320x240 / 640x480 / 1280x720 / 1920x1080（frame_cropping_flag 無し / 有り）の典型ケース
  - SPS 末尾でビット切れ → Err
  - SPS 内 emulation prevention byte 連続（`0x00 0x00 0x03 0x03`）が正しく除去されるケース
  - High プロファイル（`profile_idc == 100`）で `seq_scaling_matrix_present_flag == 1` のケース（scaling_list 読み飛ばしの検証）
  - monochrome (`chroma_format_idc == 0`、`chroma_array_type == 0`) の SPS で crop 計算が `CropUnitX = 1` / `CropUnitY = 2 - frame_mbs_only_flag` で正しく行われるケース
  - cropping 適用で width / height が 0 以下になる SPS → Err

  なお `separate_colour_plane_flag == 1`（4:4:4 separate plane）の SPS は libx264 等の主要エンコーダで生成できないため、本 issue ではテストカバレッジから外す（コード上の `chroma_array_type == 0` 分岐自体は上記の monochrome テストで検証されるため、`separate_colour_plane_flag` 経路特有の分岐 (`chroma_format_idc == 3` で `separate_colour_plane_flag` を読む) のみが未テストになる）。将来この経路を持つ入力が現れた際に別 issue でテストフィクスチャの調達手段（仕様準拠のハンドビルダー等）を含めて再整備する。
- `proptest` で任意バイト列を `extract_dimensions_from_sps` に投入し、パニックしないこと（Err 復帰のみ）を保証する PBT が追加されている。具体構成:
  - 入力長を `0..=4096` バイトに制限
  - `ProptestConfig { cases: 1024, .. }` 程度を目安
  - 無限ループ防止は `H264BitReader` の構造的保証（解決方法 1 参照）に委ね、PBT 自体に timeout は設けない
- SRT inbound テスト（`src/srt/inbound_endpoint.rs` の `#[cfg(test)] mod tests`）に「`build_video_sample` が返す `sample_entry`（`Avc1Box.visual.width` / `.height`）と `VideoFrame.size` の両方が SPS 由来の実値（テストフィクスチャの想定解像度）になる」アサートが追加されている。
- 既存の encoder / decoder 経由テスト（`tests/decoder_tests.rs` 等）が引き続きパスする（width != 0 の引数を渡し続けるため、本 issue の変更は呼び出し側挙動に影響しない）。
- `cargo test` / `cargo clippy` / `cargo fmt` がパスする。

## 解決方法

### 実装手順

1. `src/video/h264.rs` に以下を追加:
   - `struct H264BitReader<'a>`: バイト列から 1 ビット単位で読み出すリーダ。**全 read メソッド（`read_u` / `read_ue` / `read_se`）がバッファ末尾を超えたら `Err` を返す**（パニックも無限ループもしない）。これが proptest の「無限ループ防止」の構造的保証になる。
   - `fn read_u(&mut self, n: usize) -> crate::Result<u32>`: n ビット符号なし整数。
   - `fn read_ue(&mut self) -> crate::Result<u32>`: 符号なし Exp-Golomb 復号（仕様 9.1）。
   - `fn read_se(&mut self) -> crate::Result<i32>`: 符号付き Exp-Golomb 復号（仕様 9.1.1）。
   - `fn skip_scaling_list(&mut self, size: usize) -> crate::Result<()>`: scaling_list() サブルーチンの読み飛ばし（要素ごとに se(v)）。
   - `fn rbsp_from_sps_nalu(nalu: &[u8]) -> crate::Result<Vec<u8>>`: 先頭 NAL ヘッダ 1 バイトをスキップし、`0x00 0x00 0x03` → `0x00 0x00` の置換で RBSP を得る。
   - `pub fn extract_dimensions_from_sps(sps: &[u8]) -> crate::Result<(usize, usize)>`: 設計方針 → 関数設計に従って `(width, height)` を返す。

2. `src/srt/inbound_endpoint.rs` の `build_video_sample` を修正:
   - 既存の IDR 判定ループ（`:921-928`）を「IDR 判定 + SPS NAL 収集」を **同じループ内で同時実施** する構造に拡張する。具体的には IDR を検出しても `break` せず最後まで走査し、`H264_NALU_TYPE_SPS` の NAL を 1 つでも見つけたらその先頭のもの（設計方針 → SPS 走査方針）を保持する。これで PES 走査を二度行わずに済む。
   - IDR 検出時の処理:
     - SPS NAL が **見つからなかった場合**: `missing H.264 SPS` 相当の Err を返して接続を打ち切る（設計方針 → エラー処理に従う）。
     - SPS NAL が **見つかった場合**: `extract_dimensions_from_sps` で `(width, height)` を取得し、`h264_sample_entry_from_annexb(width, height, &pending.data)` の引数として渡す。同じ `(width, height)` を戻り値 `TsSample::Video` 内の `VideoFrame.size = Some(VideoFrameSize::new(width, height)?)` にも反映する（`extract_dimensions_from_sps` 側で 0 を Err にしているため `VideoFrameSize::new` は基本的に成功するが、`?` で念のため伝播）。
   - SPS パース失敗時 or `h264_sample_entry_from_annexb` Err は従来通り伝播し接続を打ち切る（後始末不要、設計方針 → エラー処理参照）。
   - `:931-936` のコメントを「IDR 内 inline SPS から解像度を抽出し、sample_entry と VideoFrame.size の両方に反映する」旨に書き換える。

3. 既存テストフィクスチャの差し替え:
   - `SPS_INITIAL` / `SPS_UPDATED`（`src/srt/inbound_endpoint.rs:1326` 付近）を `pic_width_in_mbs_minus1` 等まで完全に符号化された SPS バイト列に差し替える。
   - 0033 と同じ方式（`mod tests` 内に `const SPS_*: [u8; N] = [...]` で直接埋め込み）に揃える。`tests/fixtures/` 配下への外部ファイル化はしない。
   - 入手手段:
     - Baseline プロファイル + 1920x1080: `ffmpeg -f lavfi -i testsrc=size=1920x1080:rate=30 -c:v libx264 -profile:v baseline -frames:v 1 -f h264 -` で出力した Annex-B から、`ffmpeg -bsf:v trace_headers` で SPS NAL を確認しバイト列を `[u8; N]` で書き起こす。
     - High プロファイル + scaling_matrix（パーサの最も複雑な経路の検証用）: `ffmpeg -f lavfi -i testsrc=size=1920x1080:rate=30 -c:v libx264 -profile:v high -x264-params 'cqm=jvt' -frames:v 1 -f h264 -` で出力した SPS。これは `extract_dimensions_from_sps` の単体テスト用に `src/video/h264.rs` の `mod tests` に置く（`mod tests` 内定数で SRT inbound テストと統一）。
   - 既存テスト（PPS 不在 Err / SPS 不在 Err / 後置 SPS/PPS / 通常 / mid-stream SPS 更新）の想定挙動を、新フィクスチャの解像度（例: 1920x1080）でアサートし直す。

4. テスト追加:
   - `src/video/h264.rs` 内の `#[cfg(test)] mod tests` に、設計方針 → 関数設計の単体テスト + PBT を追加する。
   - SRT inbound テストに `sample_entry` / `VideoFrame.size` のアサートを追加する。

### 影響範囲

- `src/video/h264.rs`: 関数群追加（既存関数 `h264_sample_entry_from_annexb` / `extract_video_dimensions` / `create_sequence_header_annexb` / `convert_annexb_to_nalu` のシグネチャ・挙動は変更なし）。
- `src/srt/inbound_endpoint.rs`: `build_video_sample` の IDR 検出パスを SPS 抽出も含めた形に再構成。コメント書き換え。既存テストフィクスチャ `SPS_INITIAL` / `SPS_UPDATED` の差し替え。

### 非対象

- H.265 SPS の解像度パース。SRT / 他経路で H.265 Annex-B 入力が増えたら別 issue で対応する。
- `profile_idc` / `level_idc` の SPS 反映（`h264_sample_entry_from_annexb` 内の `TODO: 実際の値に合わせる`）。
- `chroma_format` / `bit_depth_luma` / `bit_depth_chroma` の SPS 反映。
- RTSP subscriber (`src/rtsp/subscriber.rs:637` の `size: None`)。`0032-feature-add-rtsp-annexb-video-sample-entry.md` で別途扱う（本 issue の SPS パーサを再利用する想定）。
- RTMP frame (`src/rtmp/frame.rs:303` の `size: None`)。RTMP inbound は経路として現状利用されていないため本 issue では触らない。`size` を埋める必要が出た時点で別 issue として扱う（その際に本 issue の `extract_dimensions_from_sps` を再利用可能）。
- mp4 reader 経路。`src/sora/recording_mp4_reader.rs:160` は avcC ボックスから解像度を取得済みで SPS パースを必要としない。

### CHANGES.md 方針

**記載しない**。理由:

- 0033 close 時点で SRT inbound 経路の追加自体が CHANGES.md `## develop` に記載されない方針で進められており、関連エントリが存在しない（`grep -i 'srt\|inbound' CHANGES.md` でヒット 0 件）。本 issue で「0033 のエントリの追補」は物理的に不可能。
- メモリ規約「未リリース機能の修正は独立 `[FIX]` にしない」（`changelog-unreleased-fix.md`）により、`## develop` 内の中間状態に対する追補は独立エントリにしない。`[UPDATE]` でも同原則が適用される。
- 結果として「0033 と同じく無記載」で整合させるのが素直。SRT inbound 経路全体がリリースされる際に、リリースノートで一括して説明される想定。

### テスト戦略

- SPS バイト列はモック / ハンドコーディングを避け、実機（`ffmpeg -bsf:v trace_headers` 等で確認した）SPS を採用する。配置は 0033 と同じく `mod tests` 内の `const SPS_*: [u8; N] = [...]` で直接埋め込み、`tests/fixtures/` 外部ファイル化はしない（CLAUDE.md「モックやスタブは絶対に利用しないこと」に準拠。本 issue では「実機 SPS の生バイト列を定数として埋め込む」のはハンドコーディング SPS には該当しないと整理する）。
- 既存の SRT 経路のテストヘルパ（`SrtTsDemuxer` のテスト群）に解像度アサートを追加する。
- PBT (`proptest`) は `extract_dimensions_from_sps` がパニック / 無限ループせず Err を返すことだけを保証する（実値の正当性は単体テストで担保）。

## 関連

- 0030（closed: `feature-refactor-encoded-frame-sample-entry-invariant`。本 issue は同 issue で確立した sample_entry 不変条件の整合性を SRT inbound の `VideoFrame.size` 側にも拡張する位置付け）
- 0032（open: `feature-add-rtsp-annexb-video-sample-entry`。RTSP 経路。本 issue で追加する SPS パーサを再利用する想定）
- 0033（closed: `feature-add-srt-annexb-video-sample-entry`。SRT inbound の sample_entry 構築。`:931-934` のコメント「将来の改善余地」が本 issue を指す）
- `src/video/h264.rs:132` の既存関数 `extract_video_dimensions` は **AVC1 サンプルエントリー（既に構築済みの `SampleEntry::Avc1`）から width / height を取り出す関数**であり、本 issue で追加する `extract_dimensions_from_sps`（SPS NAL バイト列からの抽出）とは別物。命名衝突しないよう注意する。
