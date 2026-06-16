# H.264 SPS から解像度（width / height）を抽出してサンプルエントリーと VideoFrame.size に反映する

- Priority: Low
- Created: 2026-06-16
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/add-h264-sps-dimensions-parser
- Polished: 2026-06-16

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
- (c) `0032-feature-add-rtsp-annexb-video-sample-entry.md`（open）が「SPS 内 Exp-Golomb 解像度抽出: `Avc1Box.visual.width/height` を 0 のまま埋める。実値抽出は RTMP / openh264 / SRT 横断で別 issue」（0032 スコープ外）と本 issue を「横断別 issue」として参照している。0032 着手前に SPS パーサを共有ユーティリティとして整備しておく必要がある。本 issue 自体は SRT inbound のみに適用するが、`extract_dimensions_from_sps` を `src/video/h264.rs` に置くことで、0032 が RTSP 経路で再利用、将来別 issue で RTMP / openh264 でも再利用可能な状態にする。

## 現状

行番号は HEAD（develop = 3e00764d）時点。実装着手時は grep で再特定する。

### 解像度を SPS パースしていない SRT inbound 呼び出し箇所

- `src/srt/inbound_endpoint.rs:930-939`（`build_video_sample` の IDR 検出箇所）。コメントは `:931-936` の 6 行で、(1) width / height は 0 で渡す、(2) SPS Exp-Golomb パース実装は将来の改善余地、(3) SPS / PPS 不在 IDR や破損 NAL は同関数が Err、(4) 正常な H.264 ストリームは IDR に SPS / PPS を inline するため Err はエンコーダ側の異常とみなしてそのまま伝播し接続を打ち切る (fail-fast)、の 4 点を述べる。引数 `(0, 0, &pending.data)` で `h264_sample_entry_from_annexb` を呼んでいる。

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
   - SPS の先頭付近（`profile_idc` / `level_idc` 周辺）では `0x000003` 出現パターンが実質発生しないため、先頭部分だけパースする実装ならスキップしても動く。しかし本実装は `pic_width_in_mbs_minus1` まで読み進める都合上、`seq_scaling_matrix_present_flag == 1` 経由で scaling_list を消費するとビット位置が深くなり、`0x000003` 出現が無視できなくなる。安全のため最初から RBSP 抽出する。
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

   整数型の取り扱いルール（一律）:
   - `read_u` / `read_ue` の戻り値は `u32`、`read_se` の戻り値は `i32`。
   - `usize` への as 変換は **算術演算の直前** に行い、変換後は `checked_add` / `checked_mul` / `checked_sub` で組む。これは `pic_width_in_mbs_minus1` / `pic_height_in_map_units_minus1` / `frame_crop_*_offset` / `offset_for_*` 等すべての算術で共通。
   - `usize` 加減算でアンダーフロー or オーバーフローが起きたら `invalid H.264 SPS: <理由>` Err を返す。

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
   - `raw_width = (pic_width_in_mbs_minus1 + 1) * 16`（一律ルールに従い `usize` 変換 + `checked_add(1)` / `checked_mul(16)` の順）
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

#### `build_video_sample` 内インライン処理 vs 純関数化

0032 (RTSP) は `apply_video_frame_sample_entry` を free function として切り出すパターンを採用するが、本 issue は 0033 close 時点のパターン（`build_video_sample` 内に直接実装）を踏襲する。理由（本 issue 側で判断する根拠）:

- 0032 の設計は `has_idr && has_sps && has_pps` の 3 条件判定を局所化する必要があるが、SRT は SPS / PPS inline 標準で `has_idr` 単独ゲートに帰着し、ロジックの複雑度が違う。
- `last_sample_entry` 相当の状態更新も SRT では IDR 検出時の 1 箇所だけで、関数として切り出すメリットが薄い。
- NAL 走査の単体テスト性は、本 issue で追加する `extract_dimensions_from_sps` 自体の単体テストで担保されるため、`build_video_sample` 内の NAL 走査自体を関数化する必要はない（NAL 走査と sample_entry 構築の組み合わせは SRT 統合テスト `SrtTsDemuxer` のテスト群で間接検証する）。
- 0033 で確定したインライン実装と整合を取ることで、レビュー時に diff が読みやすい。

将来 SRT inbound でも 3 条件判定相当の複雑性が増した場合に、改めて純関数化を検討する。

### エラー処理

- SPS が IDR 内に存在しない場合: `build_video_sample` 側で SPS 検索後に判定し、`missing H.264 SPS` 相当の Err を返して接続を打ち切る（0033 で確定した fail-fast 方針を維持）。
- PPS 不在判定は本 issue では追加しない。SPS パース成功後に呼ぶ `h264_sample_entry_from_annexb` 内の既存 `pps_list.is_empty()` 検査が `Err("missing H.264 PPS")` を返す（SRT inbound では「SPS 不在 → 自前 Err」「PPS 不在 → 関数内 Err」の二段構え。0033 で確定済みの fail-fast 方針）。
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

### 設計判断ノート（簡略化案を採らない理由）

H.264 SPS から解像度を抽出する処理には、世の中の参考実装に幅広く簡略化版が存在する。本実装で **採用しない** 簡略化と、その採用しない理由を明示する:

- 簡略化案 (A): 解像度を `(pic_width_in_mbs_minus1 + 1) * 16` / `(pic_height_in_map_units_minus1 + 1) * 16` だけで算出し、`frame_mbs_only_flag` (interlaced) や `frame_cropping_flag` (crop) を無視する。
  - **採用しない理由**: 外部入力（SRT inbound）の解像度を MP4 メタデータに正確に反映する目的に反する。crop は ffmpeg testsrc 等のテスト用ストリームでも発生し、interlaced は地デジ系の MPEG-TS で発生する。これらで誤った値が MP4 sample entry に埋まると下流プレイヤーが正しいピクセル数を取得できない。
- 簡略化案 (B): `seq_scaling_matrix_present_flag == 1` のとき、scaling_list 全体を「`chroma_format_idc == 3` で 12 ビット、それ以外で 8 ビット」を flat に skip する。
  - **採用しない理由**: 仕様非準拠。実際は 12 個（または 8 個）の `seq_scaling_list_present_flag` を 1 ビットずつ読み、立っているフラグごとに scaling_list サブルーチン（要素数ぶんの `delta_scale` se(v)）を走らせる必要がある。flat skip は `seq_scaling_list_present_flag` が 1 個でも立つと以降のビット位置がずれ、`pic_width_in_mbs_minus1` の読み取りが壊れる。
これらを採らないことで実装は重くなるが、外部入力経路での正確性を優先する。

なお RBSP 抽出（emulation prevention byte 除去）を省略する案も世の中の参考実装に存在するが、本実装では内部処理 2 で述べたとおり最初から RBSP 抽出を行う方針を採る（理由は内部処理 2 参照）。

## 完了条件

- `src/video/h264.rs` に SPS 解像度抽出関数 `extract_dimensions_from_sps`、Exp-Golomb (`ue(v)` / `se(v)`) パーサ、scaling_list 読み飛ばし、RBSP 抽出ヘルパが追加されている。
- `src/srt/inbound_endpoint.rs` の `build_video_sample` が IDR 検出時に SPS を 1 回抽出し、その結果を `h264_sample_entry_from_annexb` の引数と `VideoFrame.size` の両方に流す構造になっている。
- `:931-936` のコメント全体（`build_video_sample` IDR 検出箇所のブロックコメント）が「SPS から実値を抽出して sample_entry と VideoFrame.size の両方に反映する」旨に書き換えられている。
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
  - High プロファイル（`profile_idc == 100`）で `seq_scaling_matrix_present_flag == 1` かつ **全 `seq_scaling_list_present_flag == 0`** の手書き SPS で、scaling_list 本体を読まずに正しくスキップして `pic_width_in_mbs_minus1` に到達するケース
  - monochrome (`chroma_format_idc == 0`、`chroma_array_type == 0`) の手書き SPS で crop 計算が `CropUnitX = 1` / `CropUnitY = 2 - frame_mbs_only_flag` で正しく行われるケース
  - cropping 適用で width / height が 0 以下になる SPS → Err

  なお `separate_colour_plane_flag == 1`（4:4:4 separate plane）の SPS は主要エンコーダで生成できないため本 issue ではテストカバレッジから外す。`chroma_array_type == 0` 分岐自体は monochrome テストで検証されるため、`separate_colour_plane_flag` 経路特有の分岐 (`chroma_format_idc == 3` で `separate_colour_plane_flag` を読む) のみが未テストになる。将来この経路を持つ入力が現れた際に別 issue で再整備する。
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
   - `:931-936` のコメント全体を「IDR 内 inline SPS から解像度を抽出し、sample_entry と VideoFrame.size の両方に反映する」旨に書き換える。

3. 既存テストフィクスチャの差し替え:
   - `SPS_INITIAL` / `SPS_UPDATED`（`src/srt/inbound_endpoint.rs:1326` 付近）を `pic_width_in_mbs_minus1` 等まで完全に符号化された SPS バイト列に差し替える。
   - 0033 と同じ方式（`mod tests` 内に `const SPS_*: [u8; N] = [...]` で直接埋め込み）に揃える。`tests/fixtures/` 配下への外部ファイル化はしない。
   - 入手手順は 2 段階で実施する:
     1. ffmpeg で H.264 ファイルを生成: 例 `ffmpeg -f lavfi -i testsrc=size=1920x1080:rate=30 -c:v libx264 -profile:v baseline -frames:v 1 -f h264 out.h264`
     2. 生成したファイルから SPS NAL バイト列を確認: `ffmpeg -i out.h264 -c:v copy -bsf:v trace_headers -f null -` のログ、または `hexdump -C out.h264 | head` で `00 00 00 01 67 ...` から始まる SPS を直接読み出して `[u8; N]` リテラルとして書き起こす。
   - 解像度別の入手コマンド例:
     - Baseline プロファイル + 1920x1080: `ffmpeg -f lavfi -i testsrc=size=1920x1080:rate=30 -pix_fmt yuv420p -c:v libx264 -profile:v baseline -frames:v 1 -f h264 sps_initial.h264`（`SPS_INITIAL` 用。`testsrc` は既定 4:4:4 のため `-pix_fmt yuv420p` を明示しないと Baseline 非互換）
     - Baseline プロファイル + 1280x720: 上記の `size=1280x720` 版（`SPS_UPDATED` 用）
   - High プロファイル + `seq_scaling_matrix_present_flag == 1` の SPS は libx264 標準オプションでは安定的に出力できない（`cqm=jvt` は AVC Intra 用途で SPS 側フラグを必ずしも立てない）。本 issue では実機 SPS 調達は **scope 外** とし、scaling_list 読み飛ばし処理の検証は **「`seq_scaling_matrix_present_flag == 1` かつ全 `seq_scaling_list_present_flag == 0`」の手書き SPS バイト列**（仕様 7.3.2.1.1.1 に基づき構築。8 個または 12 個の present_flag を全て 0 にすることで scaling_list 本体を読まない経路を踏める）で行う。全 0 とする理由は、本 issue で実装する `skip_scaling_list` 関数の主目的が「`seq_scaling_list_present_flag` を正しく 1 bit ずつ読んでスキップ判定する」ことであり、`delta_scale` 本体の読み飛ばしは仕様準拠の実装で自然に成立するため、テストで `present_flag == 0` のみ検証すれば十分というため。
   - monochrome (`chroma_format_idc == 0`、4:0:0) の SPS も libx264 標準ビルドでは出力できない（4:0:0 サポートは非標準パッチ扱い）。実機 SPS 調達は同じく **scope 外** とし、`chroma_array_type == 0` 分岐の単体テストは仕様 7.3.2.1.1 に基づく手書き SPS バイト列で行う。将来 monochrome 入力が現実的に発生する場合は別 issue で再整備する。
   - 既存テスト（PPS 不在 Err / SPS 不在 Err / 後置 SPS/PPS / 通常 / mid-stream SPS 更新）の想定挙動を、新フィクスチャの解像度でアサートし直す。

4. テスト追加:
   - `src/video/h264.rs` 内の `#[cfg(test)] mod tests` に、設計方針 → 関数設計の単体テスト + PBT を追加する。
   - SRT inbound テストに `sample_entry` / `VideoFrame.size` のアサートを追加する。

### 影響範囲

- `src/video/h264.rs`: 関数群追加（既存関数 `h264_sample_entry_from_annexb` / `extract_video_dimensions` / `create_sequence_header_annexb` / `convert_annexb_to_nalu` のシグネチャ・挙動は変更なし）。
- `src/srt/inbound_endpoint.rs`: `build_video_sample` の IDR 検出パスを SPS 抽出も含めた形に再構成。コメント書き換え。既存テストフィクスチャ `SPS_INITIAL` / `SPS_UPDATED` の差し替え。

### 非対象

- H.265 SPS の解像度パース。SRT / 他経路で H.265 Annex-B 入力が増えたら別 issue で対応する。
- 解像度以外の SPS パラメータ（`profile_idc` / `level_idc` / `chroma_format` / `bit_depth_*` 等）の sample_entry 反映。スコープ節の通り本 issue では扱わない。
- SPS の VUI パラメータ（`sample_aspect_ratio` / SAR / PASP、`overscan_info`、`timing_info` 等）。Pixel Aspect Ratio (PASP box) や表示用の縦横比情報は本 issue で扱わない。
- SEI 由来の解像度情報（pic_timing / display_orientation 等）。本 issue では SPS のみ参照する。
- RTSP subscriber (`src/rtsp/subscriber.rs:637` の `size: None`)。`0032-feature-add-rtsp-annexb-video-sample-entry.md` で別途扱う（本 issue の SPS パーサを再利用する想定）。
- RTMP frame (`src/rtmp/frame.rs:303` の `size: None`)。RTMP inbound は経路として現状利用されていないため本 issue では触らない。`size` を埋める必要が出た時点で別 issue として扱う（その際に本 issue の `extract_dimensions_from_sps` を再利用可能）。
- mp4 reader 経路。`src/sora/recording_mp4_reader.rs:160` は avcC ボックスから解像度を取得済みで SPS パースを必要としない。

### CHANGES.md 方針

**記載しない**。0027 / 0030 / 0033 と同方針（`## develop` 内未リリース機能の追補は独立エントリ化しない / `## develop` に SRT inbound 関連エントリ自体が無いため追補対象が存在しない）。0017 は `[FIX]` カテゴリで論拠の質が異なるため引用しない。

### テスト戦略

- SPS バイト列は **原則として実機 SPS** を採用する（libx264 で生成可能な Baseline / Main / High 経路）。実機 SPS は `ffmpeg -bsf:v trace_headers` 等で確認したものを 0033 と同じ方式（`mod tests` 内の `const SPS_*: [u8; N] = [...]`）で直接埋め込み、`tests/fixtures/` 外部ファイル化はしない。
- libx264 で安定生成できない経路（`seq_scaling_matrix_present_flag == 1` / `chroma_format_idc == 0`）の SPS は **例外的に** 仕様 7.3.2.1.1 に基づく手書きバイト列を使用する。これは「特定の仕様分岐を踏むための入力フィクスチャを仕様に基づいて生成する」行為であり、CLAUDE.md「モックやスタブは絶対に利用しないこと」のモック / スタブ（実装の振る舞いを偽装する代用品）には該当しない、と本 issue では整理する。手書き SPS は CLAUDE.md「テストはコメントを重視すること」に従い、各バイトに対応する仕様節番号と意味（例: `// 7.3.2.1.1 profile_idc = 100 (High)`）を Rust コメントで明記する。
- 既存の SRT 経路のテストヘルパ（`SrtTsDemuxer` のテスト群）に解像度アサートを追加する。
- PBT (`proptest`) は `extract_dimensions_from_sps` がパニック / 無限ループせず Err を返すことだけを保証する（実値の正当性は単体テストで担保）。

## 関連

- 0030（closed: `feature-refactor-encoded-frame-sample-entry-invariant`。本 issue は同 issue で確立した sample_entry 不変条件の整合性を SRT inbound の `VideoFrame.size` 側にも拡張する位置付け）
- 0032（open: `feature-add-rtsp-annexb-video-sample-entry`。RTSP 経路。本 issue で追加する SPS パーサを再利用する想定）
- 0033（closed: `feature-add-srt-annexb-video-sample-entry`。SRT inbound の sample_entry 構築。`:931-936` のコメント「将来の改善余地」が本 issue を指す）
- `src/video/h264.rs:132` の既存関数 `extract_video_dimensions` は **AVC1 サンプルエントリー（既に構築済みの `SampleEntry::Avc1`）から width / height を取り出す関数**であり、本 issue で追加する `extract_dimensions_from_sps`（SPS NAL バイト列からの抽出）とは別物。命名衝突しないよう注意する。
