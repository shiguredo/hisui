# WebM リーダーの AV1 / H264AnnexB 映像経路に sample_entry 構築を追加する

- Priority: Low
- Created: 2026-06-18
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/add-webm-reader-av1-h264-sample-entry
- Polished: 2026-06-22

## 目的

WebM リーダーに sample_entry 構築を追加した PR (closed 0031) で、AV1 / H264AnnexB は CodecPrivate のパーサ実装規模が大きいため、暫定的に `WebmVideoReader::new` で `sample_entry: None` のまま開く形に留めた。本 issue では AV1CodecConfigurationRecord と AVCDecoderConfigurationRecord (avcC) のパーサを新設し、Sora 録画の AV1 / H264AnnexB WebM 経路でも `sample_entry: Some(...)` を構築する。あわせて `src/video.rs::VideoFrame.sample_entry` の docstring から「現時点で未適用の経路: WebM リーダーの AV1 / H264AnnexB 映像。」の 1 行を削除する。

## 優先度根拠

Low。closed 0030 の不変条件起点と closed 0031 の broken window 解消の延長で、不変条件を「全 WebM 経路」と言い切れる状態にする位置づけ。compose / record 経路は encoder を必ず挟むため、reader 側 `sample_entry` の有無は writer 側挙動に直接影響しない (encoder が新規 `sample_entry` を構築する)。

## 現状

`src/webm/reader.rs::WebmVideoReader::new` (`L613-667`) の `match header.codec` で `VideoFormat::Av1 | VideoFormat::H264AnnexB => None` の暫定分岐 (`L641`) と、その直前の暫定説明コメント (`L626-631`) が残っている。`src/video.rs::VideoFrame.sample_entry` docstring (`L52-58`) に「現時点で未適用の経路: WebM リーダーの AV1 / H264AnnexB 映像。」 (`L57`) の 1 行が残る。

### 既存ヘルパ (closed 0031 / 0043 マージ後)

- `src/video/av1.rs::av1_sample_entry(width: EvenUsize, height: EvenUsize, config_obus: &[u8]) -> SampleEntry`
  - 内部の `Av1cBox` フィールドは Hisui 内エンコーダ前提の Main profile / 4:2:0 / 8-bit 固定値で埋める。本 issue ではシグネチャ・固定値とも変更しない (「本 issue で触らない経路」参照)。
- `src/video/h264.rs::h264_sample_entry_from_sps_pps_lists(sps_list: Vec<Vec<u8>>, pps_list: Vec<Vec<u8>>) -> crate::Result<(SampleEntry, VideoFrameSize)>`
  - SPS / PPS リスト (NAL ヘッダ 1 バイト含む raw NAL バイト列、start code なし) を受け取り、内部で `parse_sps(sps_list[0])` を呼んで avcC 各フィールドを SPS 由来の実値で埋める。SPS / PPS の NAL タイプ検査も内部で実施する。本 issue の H264AnnexB 経路は本関数を直接呼ぶ。

### `VideoTrackHeader` の現状

`VideoTrackHeader` (`L343-348`) は `{ codec, width, height }` の 3 フィールド。`VideoTrackHeader::read` (`L351-398`) は `read_master(ID_TRACKS)` を直接呼び、CODEC_ID を `skip_until(ID_CODEC_ID)` で見つけた後 `read_pixel_dimensions` で残り子要素を走査する 2 段構造。CodecPrivate は読み捨てられる。

### WebM CodecPrivate のフォーマット

`refs/` 配下に該当一次資料は無いため、実装着手時に polish-refs スキルで一次資料を取得して引用節番号を検証する。本文では以下を前提とする:

- `V_AV1`: AV1CodecConfigurationRecord (AOM Codecs ISO Media File Format Binding §2.3)。固定 4 バイトヘッダ + 可変長 `configOBUs` (Sequence Header OBU を含む OBU 列)。byte 0 の最上位 bit が marker (= 1)、下位 7 bit が version (= 1)。byte 1..=3 は seq_profile 等の各フィールドだが、本 issue では検証も抽出もせず一括スキップする。
- `V_MPEG4/ISO/AVC`: AVCDecoderConfigurationRecord (avcC、ISO/IEC 14496-15)。`configurationVersion` / `AVCProfileIndication` / `profile_compatibility` / `AVCLevelIndication` / reserved + `lengthSizeMinusOne` / reserved + `numOfSequenceParameterSets` / SPS リスト / `numOfPictureParameterSets` / PPS リスト / (High 系プロファイル時の末尾追加フィールド)。

WebM SimpleBlock の payload 形式 (Sora 録画の H264 Annex-B / AV1 OBU 列) は本 issue では触らず、CodecPrivate のみ扱う。

## 設計方針

### 1. AV1 経路: `parse_av1_codec_private` 新設

`src/video/av1.rs` に以下を追加する (closed 0031 が `parse_opus_head_pre_skip` を `src/audio/opus.rs` に集約した先例と整合):

```rust
pub fn parse_av1_codec_private(data: &[u8]) -> crate::Result<&[u8]>
```

- 入力: WebM CodecPrivate の AV1CodecConfigurationRecord バイト列。
- 戻り値: 固定 4 バイトヘッダ (byte 0..=3) を読み飛ばし、byte 4 以降の `configOBUs` スライス参照を返す。空でも `Ok` (Sequence Header OBU 不在は後段デコーダで検出)。
- 検証:
  - `data.len() < 4` → `Err("invalid AV1 CodecPrivate: too short (expected >= 4 bytes, got {len})")`
  - byte 0 の最上位 bit (marker) が 0 → `Err("invalid AV1 CodecPrivate: marker bit is not set")`
  - byte 0 の下位 7 bit (version) が 1 以外 → `Err("invalid AV1 CodecPrivate: unsupported version {version}")`
- エラーメッセージは `"invalid AV1 CodecPrivate: ..."` プレフィックスで統一。

`WebmVideoReader::new` の match 分岐:

```rust
VideoFormat::Av1 => {
    let config_obus = parse_av1_codec_private(&codec_private)?;
    Some(SharedSampleEntry::new(av1_sample_entry(
        EvenUsize::truncating_new(width),
        EvenUsize::truncating_new(height),
        config_obus,
    )))
}
```

`EvenUsize::truncating_new(0)` は `EvenUsize::ZERO` を自動的に返すため、`width == 0 || height == 0` のフォールバック (VP8 / VP9 経路と同じ closed 0031 方針) も同じコードで動く。奇数解像度の異常系も同様 (`truncating_new` が `n - 1` を返す)。

### 2. H264AnnexB 経路: `parse_avcc_sps_pps_lists` 新設

`src/video/h264.rs` に以下を追加する。

```rust
pub fn parse_avcc_sps_pps_lists(data: &[u8]) -> crate::Result<(Vec<Vec<u8>>, Vec<Vec<u8>>)>
```

- 入力: WebM CodecPrivate の avcC バイト列。
- 戻り値: `(sps_list, pps_list)` のタプル。各要素は NAL ヘッダ 1 バイト含む raw NAL バイト列 (start code なし) で、`h264_sample_entry_from_sps_pps_lists` の入力契約と一致。avcC 内の出現順を保持する (`parse_sps(sps_list[0])` が先頭 SPS のパラメータを採用する設計のため順序保証が必須)。
- avcC の構造 (ISO/IEC 14496-15、節番号は polish-refs で検証):

  ```text
  byte 0:    configurationVersion (8 bit, must be 1)
  byte 1:    AVCProfileIndication (8 bit) -- 捨てる
  byte 2:    profile_compatibility (8 bit) -- 捨てる
  byte 3:    AVCLevelIndication (8 bit) -- 捨てる
  byte 4:    reserved (6 bit) | lengthSizeMinusOne (2 bit)
  byte 5:    reserved (3 bit) | numOfSequenceParameterSets (5 bit)
  byte 6+:   for each SPS: length (16 bit BE 固定) + SPS NAL bytes
             numOfPictureParameterSets (8 bit)
             for each PPS: length (16 bit BE 固定) + PPS NAL bytes
             [High 系プロファイル時のみ末尾追加フィールド -- 読み飛ばす]
  ```
  - SPS / PPS リストの長さフィールド (常に 16 bit BE 固定) は `lengthSizeMinusOne` (avcC 内 sample data 用の length prefix サイズ) とは別経路。
- 検証 (逐次パース):
  - `data.len() < 6` → `Err("invalid H.264 avcC: too short (expected >= 6 bytes, got {len})")` (byte 0..=5 の固定ヘッダ最小サイズ)
  - byte 0 (configurationVersion) が 1 以外 → `Err("invalid H.264 avcC: unsupported configurationVersion {version}")`
  - byte 4 下位 2 bit (`lengthSizeMinusOne`) が 3 以外 → `Err("invalid H.264 avcC: unsupported lengthSizeMinusOne {value} (expected 3)")` (Sora 録画は 3 固定。`AvccBox.length_size_minus_one` も常に 3。下流 muxer 出力後にプレイヤーが NAL を切り出せない)
  - byte 4 / byte 5 の reserved bit はマスクで捨てる。
  - byte 5 下位 5 bit (`numOfSequenceParameterSets`) が 0 → `Err("invalid H.264 avcC: numOfSequenceParameterSets == 0")` (5 bit のため上限 31 は構造的に保証され、上限超過は発生しない)
  - 全 SPS 読了後の `numOfPictureParameterSets` が 0 → `Err("invalid H.264 avcC: numOfPictureParameterSets == 0")`
  - `numOfPictureParameterSets > 31` → `Err("invalid H.264 avcC: numOfPictureParameterSets exceeds 31")` (8 bit のため最大 255 だが、`shiguredo_mp4::AvccBox::encode` の制約で 31 個まで)
  - SPS / PPS の逐次パース中に残バイト不足 (length フィールド 2 バイトを読むのに不足、`numOfPictureParameterSets` の 1 バイトを読むのに不足、length フィールドが示す NAL バイト列が残バイトを超える) はすべて `Err("invalid H.264 avcC: SPS/PPS length exceeds remaining data")` の統一メッセージで返す。
- SPS / PPS の NAL タイプ検査は `h264_sample_entry_from_sps_pps_lists` 内で実施されるため本関数では検査しない。byte 1..=3 と High 系プロファイル末尾追加フィールドは `parse_sps` が SPS 由来実値を抽出するため捨てる。
- エラーメッセージは `"invalid H.264 avcC: ..."` プレフィックスで統一 (closed 0043 の `"invalid H.264 SPS: ..."` 形式と整合)。

`WebmVideoReader::new` の match 分岐:

```rust
VideoFormat::H264AnnexB => {
    let (sps_list, pps_list) = parse_avcc_sps_pps_lists(&codec_private)?;
    let (entry, _frame_size) = h264_sample_entry_from_sps_pps_lists(sps_list, pps_list)?;
    Some(SharedSampleEntry::new(entry))
}
```

戻り値タプルの `VideoFrameSize` は捨てる (`VideoFrame.size` は WebM SimpleBlock の payload を解析しない既存設計に揃える)。

### 3. `VideoTrackHeader::read` を `AudioTrackHeader::read` と同じ TrackEntry 単一 peek_id ループに揃える

現状の `VideoTrackHeader::read` は `skip_until(ID_CODEC_ID)` で CodecID 到達前の子要素を全消費する 2 段構造。Matroska 仕様で TrackEntry 内子要素順序は規定されていないため、CodecPrivate が CodecID より先に出る WebM では現状コードが CodecPrivate を取りこぼす。本 issue では `AudioTrackHeader::read` (`L289-340`) と同じく TrackEntry 直下を単一 peek_id ループで走査する構造に変更する (戻り値型の Option 化は本 issue では揃えず、既存の I420 センチネル方式を維持する)。

`VideoTrackHeader` 構造体は `codec: VideoFormat` のみを残す (`width / height` は `WebmVideoReader::new` のローカル変数で消費し struct に保持しない)。`WebmVideoReader.header: VideoTrackHeader` は `read_simple_block` 内の `format: self.header.codec` 参照のために `codec` のみ保持する。

戻り値型を `crate::Result<(Self, usize, usize, Vec<u8>)>` に変更 (タプル順序: `(VideoTrackHeader, width, height, codec_private)`)。

#### 走査ロジック

1. `read_master(ID_TRACKS)` で TRACKS master を開く (既存通り、`skip_until(ID_TRACKS)` は不要)
2. `let mut found: Option<(VideoFormat, usize, usize, Vec<u8>)> = None;` を用意
3. `while !tracks_reader.is_eos()` で TRACK_ENTRY 走査。`TRACK_NUMBER` が `TRACK_NUMBER_VIDEO` でない、または `found.is_some()` なら `skip_all` で次へ (`AudioTrackHeader::read` と対称に、found 後も break せず全 TRACK_ENTRY を消費する)
4. 対象 TRACK_ENTRY 内で `let mut video_seen = false;` を用意し、`while !entry.is_eos()` の `peek_id` ループ:
   - `ID_CODEC_ID` → `read_bytes` で取り出し VideoFormat にマッピング (既存と同じく `b"V_VP8"` / `b"V_VP9"` / `b"V_AV1"` / `b"V_MPEG4/ISO/AVC"` 以外は `Err("unknown video codec ID: ...")`)
   - `ID_CODEC_PRIVATE` → `read_bytes_with_limit(ID_CODEC_PRIVATE, 65536)` (後述) で取り出し
   - `ID_VIDEO` → `video_seen = true;` した上で `read_master(ID_VIDEO)` で VIDEO master を開き、`read_pixel_dimensions` (改修後、後述) で PixelWidth / PixelHeight 取り出し
   - その他 → `read_id` + `skip_element_data` で読み捨て
5. peek_id ループを抜けた直後に、CodecID 未取得なら `Err("video TRACK_ENTRY missing CodecID element")` (`AudioTrackHeader::read` と対称)。`!video_seen` なら警告ログ (`"WebM video TRACK_ENTRY has no Video master element; falling back to width=0 height=0"`) + width / height = 0 でフォールバック (現状 `read_pixel_dimensions` 内の警告と同じ文面)
6. `found = Some((codec, width, height, codec_private))` を代入
7. TRACKS 走査完了後、`Ok((Self { codec }, width, height, codec_private))` を `found` から取り出して返す。`found.is_none()` (映像トラック不在) なら `Ok((Self { codec: VideoFormat::I420 }, 0, 0, Vec::new()))` でフォールバック (closed 0031 の既存挙動を維持)

VP8 / VP9 / I420 経路では `codec_private` を読むが使わない (空 Vec のまま流れる前提)。理論的に VP8 / VP9 の CodecPrivate が存在する WebM が来ても読み捨てとなり、後続の match 分岐で sample_entry 構築に使わない。

#### `read_pixel_dimensions` の責務縮小

現状の `read_pixel_dimensions` (`L410-455`) は TrackEntry 直下走査ループと VIDEO master 内ループの 2 段構造。本 issue では TrackEntry 直下走査は `VideoTrackHeader::read` 本体に移管するため、`read_pixel_dimensions` を VIDEO master 内ループだけに縮小する。シグネチャは現状の `ElementReader<std::io::Take<R>>` 受け取り型のままで、引数が「VIDEO master を開いた reader」になる。

警告ログは 2 種類維持する:

- (a) PixelWidth / PixelHeight が VIDEO master 内に不在 (片方または両方が 0) → `read_pixel_dimensions` 内で警告
- (b) VIDEO master 自体が TrackEntry 直下に不在 → `VideoTrackHeader::read` の peek_id ループ走査完了直後 (CodecID チェックと同じ場所) で警告

#### `ElementReader::read_bytes_with_limit` 新設

`ElementReader::read_bytes` (`L183-194`) は `size >= 1024` で Err になるガード値を持つ。AV1CodecConfigurationRecord と avcC は 1024 を超える可能性がある。`read_bytes` の上限を引き上げると `read_u64` / `expect_str` 経由経路も影響を受け、敵対的入力で最大 64 KB の中間アロケーションが起きる退行が発生する。これを避けるため、上限を引数で受け取る新メソッドを追加して CodecPrivate 経路のみ上限を緩める:

```rust
fn read_bytes_with_limit(&mut self, expected_id: u32, max_size: u64) -> crate::Result<Vec<u8>>
```

- 実装は `read_bytes` の `if size >= 1024` を `if size >= max_size` に置き換えた版。本 issue では CodecPrivate 取得時に `max_size = 65536` で呼び出す。
- 64 KB の根拠: Sora 録画の AV1 CodecPrivate と avcC は実測 1 KB 未満。安全マージン + 壊れた WebM の OOM ガードとして 65536 を採用。
- `read_bytes` 自体は 1024 上限のまま維持し、既存の `read_u64` / `expect_str` 経路の挙動を変えない。

### 4. 不変条件 docstring の例外節削除

`src/video.rs::VideoFrame.sample_entry` の docstring (`L52-58`) から以下の 1 行を削除する。

```text
    /// 現時点で未適用の経路: WebM リーダーの AV1 / H264AnnexB 映像。
```

削除後は「不変条件: 圧縮フォーマットの `VideoFrame` は常に `Some`」「生フォーマットと中間表現は `None` を許容」の 2 段構成のみが残る。

## 本 issue で触らない経路

- `av1_sample_entry` の Hisui 固定値解消: `Av1cBox` の Main profile / 4:2:0 / 8-bit 固定値は本 issue では変更せず別 issue 候補 (open 0048 の「将来別 issue」予告と整合)。副次的に、WebM ソースの AV1 録画では HLS codec_string が `av01.0.00M.08` 固定になり、クライアントの実エンコード設定 (10-bit / 4:4:4 等) と乖離する可能性がある。
- `WebmFileReader` (inspect 経路) の挙動変化: 本 issue 完了後は壊れた CodecPrivate の AV1 / H264AnnexB WebM を inspect しようとすると `WebmVideoReader::new` 段階で `Err` になり、`WebmFileReader::new` も Err 伝搬で構築不能になる (inspect サブコマンドが diagnosis を吐けず終了)。Sora 録画では発生しない異常系のため許容する。
- WebM SimpleBlock payload 形式: AV1 OBU 列 / H264 Annex-B / VP8 / VP9 の payload 解析は変更しない。本 issue は CodecPrivate のみ扱う。

## テスト

closed 0031 / 0043 と同じ密度で単体テストを追加する。reader 経路の合成 EBML フィクスチャテストは `WebmVideoReader::new` の API 制約 (`Path` のみ受け取り `Cursor` を流せない) で本 issue では見送り、parser 単体テスト 13 件でカバーする。AV1 / H264AnnexB WebM testdata の追加または `WebmVideoReader::new` の `Read` 受け取り版 API 追加は別 issue 候補。

### `src/video/av1.rs::#[cfg(test)] mod tests` (新設、4 件)

- `parse_av1_codec_private_extracts_config_obus`: 4 バイトヘッダ + ダミー OBU 列 → byte 4 以降を返す
- `parse_av1_codec_private_returns_err_on_too_short`
- `parse_av1_codec_private_returns_err_on_marker_bit_unset`
- `parse_av1_codec_private_returns_err_on_unsupported_version`

### `src/video/h264.rs::tests` (既存モジュールに追加、9 件)

closed 0043 で `pub(crate) const SPS_320X240` が `src/video/h264.rs::tests` 内に導入済み (参照パス `crate::video::h264::tests::SPS_320X240`)。既存 PPS バイト列 `[0x68, 0xce, 0x06, 0xe2]` も再利用する。avcC バイト列の構築はテストヘルパー関数 `build_avcc(sps_list, pps_list) -> Vec<u8>` をテストモジュール内に新設して使う (`lengthSizeMinusOne = 3` 固定、reserved bit はテスト側で適宜詰める)。実装の詳細は実装着手時に確定する。

- `parse_avcc_sps_pps_lists_extracts_single_sps_pps`: SPS 1 個 / PPS 1 個 → 両リスト取り出し
- `parse_avcc_sps_pps_lists_supports_multiple_sps_pps`: SPS 2 個 / PPS 2 個 → 出現順保持
- `parse_avcc_sps_pps_lists_returns_err_on_invalid_configuration_version`
- `parse_avcc_sps_pps_lists_returns_err_on_invalid_length_size`
- `parse_avcc_sps_pps_lists_returns_err_on_zero_sps_count`
- `parse_avcc_sps_pps_lists_returns_err_on_zero_pps_count`
- `parse_avcc_sps_pps_lists_returns_err_on_too_many_pps`: PPS 数 32 以上
- `parse_avcc_sps_pps_lists_returns_err_on_truncated_sps_length`: SPS 長フィールドが残りバイトを超える
- `parse_avcc_sps_pps_lists_returns_err_on_too_short`: バイト長 5 以下

### 既存テスト維持

`tests/reader_webm_tests.rs` の VP8 + Opus 2 テストと、`src/webm/reader.rs::tests` の `releases_new_arc_per_construction` 2 件は VP8 + Opus testdata のため変更不要。`VideoTrackHeader::read` の構造変更が VP8 経路に影響しないことを既存テストで確認する。

## 推奨パッチ順序

各ステップ完了時点で `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が pass する原子コミットを作る。

1. **パーサ追加**:
   - `parse_av1_codec_private` を `src/video/av1.rs` に追加 + 単体テスト 4 件
   - `parse_avcc_sps_pps_lists` を `src/video/h264.rs` に追加 + 単体テスト 9 件 (`build_avcc` ヘルパーもテストモジュール内に追加)
2. **`ElementReader::read_bytes_with_limit` 新設**: `src/webm/reader.rs` に追加。既存 `read_bytes` は変更しない (1024 上限維持)
3. **`VideoTrackHeader::read` 構造変更 + match 切替 + docstring 削除** (原子コミット): §3 / §1 / §2 / §4 で定義した変更をまとめて 1 コミットにする。`VideoTrackHeader::read` の戻り値型変更とその呼び出し側追従、AV1 / H264AnnexB の Some 分岐化、暫定説明コメント (`L626-631`) 削除、`src/video.rs::VideoFrame.sample_entry` docstring の例外節 1 行削除を 1 コミットで行う (不変条件の成立とコメント更新は原子)。

## 影響範囲確認

着手前と完了時に grep して確認する:

```text
rg -n 'VideoFormat::Av1 \| VideoFormat::H264AnnexB' src/webm/reader.rs
rg -n 'パーサ実装が必要で本 PR スコープ外' src/webm/reader.rs
rg -n '現時点で未適用の経路' src/
rg -n 'parse_av1_codec_private|parse_avcc_sps_pps_lists' src/
```

完了時、最初の 3 つは 0 件。

## 完了条件

- `WebmVideoReader::new` が `VideoFormat::Av1` / `VideoFormat::H264AnnexB` で `sample_entry: Some(SharedSampleEntry)` を構築すること。CodecPrivate が壊れている / 欠落している場合は `Err` を返す。
- `src/video.rs::VideoFrame.sample_entry` docstring から該当 1 行が削除されていること。
- 上記テストセクションで定義した全テスト (13 件) が追加されて pass すること。
- `VideoTrackHeader::read` の構造変更が VP8 / VP9 / I420 経路の既存挙動を保つこと (既存テストが pass)。
- compose サブコマンドでの Sora 録画にリグレッションが無いこと (既存 e2e の WebM ソース利用テスト全通過)。
- 公開 API 変化なし: 新規 `pub fn parse_av1_codec_private` / `parse_avcc_sps_pps_lists` の追加のみ。`WebmVideoReader::new` / `WebmAudioReader::new` / `av1_sample_entry` / `h264_sample_entry_from_sps_pps_lists` のシグネチャは不変。
- 副次的影響: inspect 経路 (`src/subcommand_inspect.rs` 経由) で AV1 / H264AnnexB WebM の `VideoFrame.sample_entry` が `Some` で観測できるようになる。compose / record の writer 経路は encoder で sample_entry を再構築するため、reader 由来 sample_entry の有無に依存せず writer 側挙動は不変 (decoder 側の `openh264.rs::build_annexb_input` は `VideoFormat::H264` (AVCC) でのみ呼ばれ、`VideoFormat::H264AnnexB` 経路では呼ばれないため、本 issue による decode 成功率変化は無い)
- feature gate ごとに以下が pass する:
  - デフォルト build: `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test && cargo fmt --all -- --check`
  - `fdk-aac` feature
  - `nvcodec` feature (CUDA SDK 利用可能環境)
  - macOS 限定 `shiguredo_video_toolbox` 経路 (macOS 上)

### CHANGES.md

記載しない (closed 0017 / 0027 / 0030 / 0031 / 0037 / 0043 と同方針)。

## 関連

- issue 0030 (closed): 不変条件起点。圧縮フレームは常に sample_entry を持つ
- issue 0031 (closed): 本 issue の直接前提。WebM リーダー VP8 / VP9 / Opus 経路の sample_entry 構築を追加
- issue 0034 (closed): writer 側 sample_entry 欠落検知の `resolve_*_sample_entry` 導入
- issue 0043 (closed): `h264_sample_entry_from_sps_pps_lists` 新設。本 issue の H264AnnexB 経路で直接利用する
- issue 0048 (open): `h265_sample_entry` を VPS / SPS / PPS リスト受け取り版にリファクタ。AV1 経路の固定値解消は同 issue で「将来別 issue」として予告
