# HlsWriter の MpegTs 経路で sample_entry None 時の静かな劣化を Err 化する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-hls-mpegts-fail-on-missing-sample-entry
- Polished: 2026-06-26

## 目的

closed/0051 で writer 入口の sample_entry fallback 補完経路を全削除した結果、`HlsWriter` の MpegTs 経路だけが「不変条件違反フレーム流入時に Err にもならず、不正な ADTS / AnnexB を静かに出力する」唯一の経路として残った。本 issue では `src/hls/writer.rs` のヘルパ `convert_length_prefixed_to_annexb` / `extract_aac_config` のハードコードフォールバックを Err 化し、`DashWriter` / `HlsWriter` fMP4 経路と同じ「Err を上位に伝播し `run` で `tracing::warn!` ログ + 該当フレームスキップ」挙動に揃える。

本 issue は **観察可能なバグの修正ではなく fail-safety 補強の refactor** である。closed/0051 で確立した入力側不変条件のもとでは違反フレームは writer に届かないため、現状の本番経路では発火しない。

## 優先度根拠

Low。closed/0051 で確立した入力側不変条件のもとでは違反フレームは writer に届かないため、現状の実装でも発火しない経路。closed/0054（encoder 側の出力保留設計）で encoder 側の fail-safety 補強が完了した今、本 issue は writer 側に残る最後の例外（HLS MpegTs 経路のハードコードフォールバック）を消す位置付け。将来の入力経路追加で前提が崩れた場合に「静かな破壊」が起きるリスクを取り除く保険として価値が高い。

## 現状

closed/0051 で writer 入口の sample_entry fallback 補完経路を削除した結果、writer 側の違反流入時の挙動は以下のように分かれる:

- `Mp4Writer` / `HybridMp4Writer` 経路: muxer の `shiguredo_mp4::mux::MuxError::MissingSampleEntry` Err を `?` で `run` 上位まで伝播 → パイプライン fail-fast 停止
- `DashWriter` / `HlsWriter` の fMP4 経路: muxer の `shiguredo_mp4::mux_fmp4_segment::Fmp4SegmentMuxer` は以下のように振る舞う:
  - (a) `sample_entry: None` のサンプルが届いたとき、`current_sample_entry_index`（前段から引き継いだもの、または初期 `None`）も `None` であれば `MissingSampleEntry` Err
  - (b) 同セグメント内の前サンプルと **`PartialEq` で異なる** `sample_entry` が現れると `MixedSampleEntries` Err
  - (c) `sample_entry: None` の後続サンプルで `current_sample_entry_index` に既知 index が残っている場合は、その index を流用する（Err にしない）
  - Err はいずれも `run` の `tracing::warn!` で握り潰され配信は止まらない。(c) は本来異なる sample_entry に紐付くべきフレームが `sample_entry: None` で流入したときに既知 index と一致したものとして扱われるため、入力側不変条件違反時の意味的整合性は保証されない（本 issue のスコープ外。現状確認結果として残す）
- **`HlsWriter` の MpegTs 経路**: Err にもならず、ハードコードフォールバックで不正な ADTS / AnnexB を静かに出力。運用上はファイル再生時に初めて気づく経路になる。MpegTs 経路は `mpeg2ts` crate の `TsPacketWriter` を直接使い `Fmp4SegmentMuxer` を経由しないため、不変条件違反の第一線がヘルパ関数になる

該当ヘルパ（実装着手時に行番号は再特定する）:

- `src/hls/writer.rs::convert_length_prefixed_to_annexb`: `sample_entry` が `Avc1` でない場合（`None` 含む）は `length_size: 4` を使う。キーフレーム時 SPS/PPS 注入も `Avc1` でないときはスキップする
- `src/hls/writer.rs::extract_aac_config`: 以下 3 経路で AAC-LC (`audio_object_type = 2`) / 48kHz (`sampling_frequency_index = 3`) / stereo (`channel_configuration = 2`) を返す
  - `sample_entry` が `None` か `Mp4a` 以外
  - `Mp4a` だが `esds_box.es.dec_config_descr.dec_specific_info` が `None`
  - `dec_specific_info.payload.len() < 2`

`extract_aac_config` の唯一の呼び出し元は `src/hls/writer.rs::wrap_raw_aac_in_adts` であり、内部で `extract_aac_config(sample_entry)?` を `?` 伝播する module-private fn である。

## 設計方針

### 1. ヘルパ関数で sample_entry None / 期待外型 / 中身異常時に Err を返す

`src/hls/writer.rs` の以下を改修する。3 関数とも module-private な `fn` で戻り値型は既に `crate::Result<...>` のため、シグネチャ・公開境界に変化はなく外部 API 互換性に影響しない。

#### 1-1. `convert_length_prefixed_to_annexb`

関数冒頭で `let-else` パターンを使い `&Avc1Box` を取り出す。これにより以降の本体内で `avcc_box.sps_list` / `pps_list` のフィールドアクセスが直接書けるようになり、内部の `match` 構造と `Some(Avc1)` 分解の死コードが解消される。

整理する死コード:

- `length_size = match sample_entry { Some(Avc1) => ..., _ => 4 }` の `_ => 4` フォールバック分岐は除去する。`length_size = avc1.avcc_box.length_size_minus_one.get() as usize + 1` の式は維持する。NALU 切り出しループ内の `match length_size { 1 => ..., 2 => ..., 3 => ..., 4 => ..., _ => Err(...) }` は既存実装のまま変更しない（`length_size_minus_one` の値域は parse 経路の `Uint::from_bits` による 2 ビットマスクと入力経路の `Uint::new(NALU_HEADER_LENGTH as u8 - 1) = Uint::new(3)` 等の構築慣行で意味的に 0..=3 に担保されており、`_ => Err` は意味レベルで到達不能だが防御的検査として残す）
- `if keyframe && let Some(Avc1) = sample_entry { ... }` の `Some(Avc1)` 分解は不要になるため `if keyframe { ... }` に簡素化する

#### 1-2. `extract_aac_config`

以下 3 経路すべてで `Err` を返す。意味レベル不変条件違反として `docs/internals/sample_entry_invariant.md` 17 行の「型レベルと意味レベルの双方を要求する」と合流する:

- `sample_entry` が `None` か `Mp4a` 以外
- `Mp4a` だが `esds_box.es.dec_config_descr.dec_specific_info` が `None`
- `dec_specific_info.payload.len() < 2`

戻り値型は現状の `crate::Result<(u8, u8, u8)>` をそのまま維持する（順序は `(audio_object_type, sampling_frequency_index, channel_configuration)`）。

#### 1-3. `wrap_raw_aac_in_adts`

変更不要。内部の `extract_aac_config(sample_entry)?` が透過的に Err 伝播し `handle_audio_frame` まで届く。

#### 1-4. SampleEntry バリアント名取り出しヘルパ

Err メッセージで `Avc1` 等のバリアント名を文字列化するため、`src/hls/writer.rs` 内に module-private な小さなヘルパ fn を追加する。`Debug` 出力は内部フィールド全体を含むため使えない。

シグネチャは `fn sample_entry_variant_name(entry: Option<&shiguredo_mp4::boxes::SampleEntry>) -> &'static str` とし、`shiguredo_mp4::boxes::SampleEntry` の全バリアント (`Avc1` / `Hev1` / `Hvc1` / `Vp08` / `Vp09` / `Av01` / `Opus` / `Mp4a` / `Flac` / `Unknown`) を `"Avc1"` 等のバリアント名と同じ文字列にマッチさせ、`None` は `"None"` を返す。`convert_length_prefixed_to_annexb` と `extract_aac_config` 双方の Err メッセージで共有する。`shiguredo_mp4` 側で `SampleEntry` にバリアントが追加された場合は `match` の網羅性チェックでコンパイル時 Err になる。

#### 1-5. 上位への伝播

`HlsWriter::handle_video_frame` / `handle_audio_frame` の MpegTs 経路で違反フレームが流入した場合は `Err` で上位に伝播し、`run` メソッドの `tracing::warn!("HLS audio frame error: {}", e.display())` / `tracing::warn!("HLS video frame error: {}", e.display())` で握り潰されてログに残るが、不正な出力ファイルは生成されない（該当フレームだけスキップされる）。

### 2. Err メッセージ粒度

Err メッセージは以下 3 要素を含める。`tracing::warn!("HLS ... error: ...")` で運用ログに流れた際に原因の切り分けができる粒度を担保する:

- 経路を識別する接頭辞（`HLS MpegTs video` / `HLS MpegTs audio`）
- 期待する `SampleEntry` バリアント、または期待する内部状態
- 実際の状態（バリアント名、または欠落 / 長さ等の説明）

文体はリポジトリの既存 Err 文と揃え、`expected X, but got Y` のコンマ区切り形式を採用する（`src/yuv.rs:34` / `src/media.rs:34` / `src/video/h264.rs:794` / `src/webm/reader.rs:137` 等で確立済み）。`but got` の後は具体的な名詞句を置く慣行に従い、内部状態のケースも同形式に揃える:

- バリアント不一致: `expected Avc1 sample_entry, but got {variant_name}`
- `dec_specific_info` 欠落: `expected dec_specific_info to be Some, but got None`
- `audio_specific_config` 不足: `expected audio_specific_config to be at least 2 bytes, but got {len} bytes`

`{variant_name}` / `{len}` は `format!` の波括弧プレースホルダで、`sample_entry_variant_name(entry)` の戻り値や実値を埋め込む（実装文言にプレースホルダ表記はそのまま残さない）。既存の `convert_length_prefixed_to_annexb` の Err（`"unsupported NALU length size: {length_size}"` / `"NALU length {nalu_len} exceeds remaining data ..."`）と接頭辞で区別できる。

テスト assert は既存パターン（`src/video/h264.rs:1452-1455` の `display.contains(...)` 等）に倣い、接頭辞 + 期待状態のキーフレーズ 2 句（例: `"HLS MpegTs video"` と `"expected Avc1 sample_entry"`）を `display.contains(...)` で部分一致確認する。

### 3. テスト追加

`src/hls/writer.rs` には現状 `mod tests` が無いため新設する。対象関数は同期関数のため `#[test]` の同期テストで十分（tokio 不要）。リポジトリの `dev-dependencies` に `rstest` 等のパラメータ化テストクレートは無いため、複数ケースをまとめる場合は単一関数内の `for` ループまたは明示列挙で記述する（新規 dev-dependency は追加しない）。

#### 3-1. テストヘルパとバイト列

`crate::video::h264::tests` モジュールは `pub(crate)` で公開されており、`SPS_320X240` / `PPS_NAL` 等の集約バイト列を他モジュールの `mod tests` から `use` で取り込む先例が確立されている（`src/rtmp/frame.rs:369` / `src/decoder/openh264.rs:173` / `src/rtsp/subscriber.rs:2152` / `src/srt/inbound_endpoint.rs:1215`）。本 issue のテストも同じ参照パターンに揃える:

- `Avc1` 構築: `crate::video::h264::tests::SPS_320X240` と `crate::video::h264::tests::PPS_NAL` を `use` で取り込み、`crate::video::h264::h264_sample_entry_from_sps_pps_lists(vec![SPS_320X240.to_vec()], vec![PPS_NAL.to_vec()])` を呼ぶ。戻り値は `crate::Result<(SampleEntry, VideoFrameSize)>` のため `let (entry, _) = h264_sample_entry_from_sps_pps_lists(...).expect("Avc1 SampleEntry 構築成功");` で `SampleEntry` を取り出す
- `Mp4a` 構築（正常系）: `crate::audio::aac::create_mp4a_sample_entry(audio_specific_config, sample_rate, channels)` を呼ぶ。引数型は `&[u8]` / `crate::audio::SampleRate` / `crate::audio::Channels`。AAC-LC 44.1kHz mono は ASC `&[0x12, 0x08]` / `SampleRate::from_u32(44_100).expect("44.1kHz SampleRate 構築成功")` / `Channels::MONO` で構築。戻り値は `crate::Result<SampleEntry>` のため `.expect("Mp4a SampleEntry 構築成功")` で取り出す。正常系テストでは `extract_aac_config` が `(audio_object_type, sampling_frequency_index, channel_configuration) = (2, 4, 1)` を返すことを assert（フィールドごとに別の値を使うことで各 bit 位置の取り違えを検出する）
- `dec_specific_info: None` の `Mp4a` 構築: `create_mp4a_sample_entry` は常に `Some` を埋めるため、戻り値を以下の手順で書き換え再 wrap する:
  ```rust
  let SampleEntry::Mp4a(mut m) = create_mp4a_sample_entry(...).expect("Mp4a SampleEntry 構築成功") else {
      unreachable!("create_mp4a_sample_entry always returns SampleEntry::Mp4a")
  };
  m.esds_box.es.dec_config_descr.dec_specific_info = None;
  let entry = SampleEntry::Mp4a(m);
  ```
- `asc.len() < 2` の `Mp4a` 構築: 同様に `dec_specific_info.payload` を 1 バイト (`vec![0x12]`) に書き換えて再 wrap する
- `Hvc1` 構築（同ドメイン異コーデック）: `crate::video::h265::tests` には現状 SPS のみ集約定数（`HEVC_SPS_640X480`）が公開されており、VPS / PPS の集約定数は無い。本 issue では `src/video/h265.rs::tests` 内の `VPS_HEADER` / `PPS_HEADER`（現状 module-private）を `pub(crate) const HEVC_VPS_NAL: &[u8] = &[0x40, 0x01];` / `pub(crate) const HEVC_PPS_NAL: &[u8] = &[0x44, 0x01];` として新規追加し（h264 の `PPS_NAL` 公開と同じ流儀）、`crate::video::h265::tests::{HEVC_VPS_NAL, HEVC_SPS_640X480, HEVC_PPS_NAL}` を `use` で取り込んで `crate::video::h265::h265_sample_entry_from_vps_sps_pps_lists(vec![HEVC_VPS_NAL.to_vec()], vec![HEVC_SPS_640X480.to_vec()], vec![HEVC_PPS_NAL.to_vec()], FrameRate::FPS_30)` で構築する
- `Opus` 構築（同ドメイン異コーデック）: `crate::audio::opus::opus_sample_entry(0)` を呼ぶ。戻り値はそのまま `SampleEntry::Opus(...)` のため追加の wrap 不要

#### 3-2. テスト関数

異常系（Err を返す）と正常系（Ok を返す）を関数名 prefix で区別する:

- `convert_length_prefixed_to_annexb_returns_err_on_non_avc1_sample_entry`: `None` / `Mp4a`（クロスドメイン）/ `Hvc1`（同ドメイン異コーデック）の各ケースで Err、`keyframe` は `true` / `false` のいずれでも Err となることを assert
- `convert_length_prefixed_to_annexb_succeeds_on_avc1_keyframe`: `sample_entry: Avc1` + `keyframe = true` で Ok。出力先頭に SPS / PPS が start code (`[0x00, 0x00, 0x00, 0x01]`) 付きで注入されること、続いて元の NAL 本体が同じく start code 付きで続くこと、NAL 本体のバイト列が透過的に保持されることを assert
- `convert_length_prefixed_to_annexb_succeeds_on_avc1_non_keyframe`: `sample_entry: Avc1` + `keyframe = false` で Ok。SPS / PPS が注入されないこと（先頭は単一の start code + NAL 本体のみ）を assert
- `extract_aac_config_returns_err_on_non_mp4a_sample_entry`: `None` / `Avc1`（クロスドメイン）/ `Opus`（同ドメイン異コーデック）で Err
- `extract_aac_config_returns_err_on_missing_dec_specific_info`: `dec_specific_info = None` に書き換えた `Mp4a` で Err
- `extract_aac_config_returns_err_on_short_audio_specific_config`: `payload` を 1 バイトに書き換えた `Mp4a` で Err
- `extract_aac_config_succeeds_on_mp4a_aac_lc_44k_mono`: 上記正常 `Mp4a` で `(audio_object_type, sampling_frequency_index, channel_configuration) = (2, 4, 1)` を assert

`wrap_raw_aac_in_adts` の Err 伝播テストは追加しない。`extract_aac_config_returns_err_on_*` で Err 経路は担保され、`?` 伝播はコンパイル時保証のため、独立テストの追加価値が乏しい。`sample_entry_variant_name` ヘルパ単体テストも追加しない（網羅性は `match` のコンパイル時担保）。

### 4. docs/internals/sample_entry_invariant.md の整合性整理

現行「writer 側の前提」節（67-73 行、`## writer 側の前提`）は 4 writer を一律に「muxer が `MissingSampleEntry` Err を返してパイプライン fail-fast 停止」と書いているが、これは `Mp4Writer` / `HybridMp4Writer` の挙動にしか合致しない。本 issue 完了にあわせ、節タイトルは `## writer 側の前提` のまま維持し、内部のフォーマットを表で 4 writer / 3 グループの挙動差を明示する形に書き換える。

書き換え案:

```markdown
## writer 側の前提

各 writer は基本的に補完値（fallback）や違反検知ロジックを持たず、入力側で不変条件が確立している前提で動作する。
万一不変条件が破られた場合の Err 発生箇所と上位 `run` での扱いは経路ごとに異なる:

| writer | Err 発生箇所 | 上位 `run` での扱い |
|---|---|---|
| `Mp4Writer` / `HybridMp4Writer` | muxer (`MissingSampleEntry`) | `?` 伝播してパイプライン fail-fast 停止 |
| `DashWriter` / `HlsWriter` (fMP4 経路) | muxer (`MissingSampleEntry` / `MixedSampleEntries`) | `tracing::warn!` で握り潰し、該当フレームをスキップ |
| `HlsWriter` (MpegTs 経路) | ヘルパ (`convert_length_prefixed_to_annexb` / `extract_aac_config`) | `tracing::warn!` で握り潰し、該当フレームをスキップ |

`HlsWriter` (MpegTs 経路) のみ Err 発生箇所が muxer ではなくヘルパなのは、MpegTs 経路が `Fmp4SegmentMuxer` を経由せず `mpeg2ts::ts::TsPacketWriter` を直接使うため、不変条件違反の第一線がヘルパ関数になることによる。
退行検知は各入力経路（リーダー / エンコーダ）の単体テストおよび e2e テストで担保する。

なお `input_*_track_id == None`（track 無効化中）に受信した違反フレームを観測する手段も writer 側には持たない。
以前は警告ログとカウンタで観測連続性を保っていたが、責任の所在を入力側に集約する方針として意図的に放棄した。
track 無効化中も含めて違反は入力側で発生しない前提で運用する。
```

`Mp4Writer` と `HybridMp4Writer`、および `DashWriter` と `HlsWriter` (fMP4 経路) はそれぞれ Err 発生箇所と上位扱いが完全に一致するため 1 行に集約する（5 経路を 3 行で表現）。これにより writer 入口に「基本的に補完値や違反検知ロジックを持たず」と例外を含意した表現に書き換えつつ、表で 5 経路の Err 経路差を区別する。

### CHANGES.md

`## develop` への記載は行わない。本 issue で改修する `convert_length_prefixed_to_annexb` / `extract_aac_config` は HLS MpegTs 経路として `## develop` の `[ADD] obsws の Output に HLS ライブ出力` で未リリース機能として導入されたもので、`shiguredo-changelog` の「派生元ブランチとの最終的な差分のみを記載すること」「開発ブランチ内の中間状態の修正は記載しないこと」に従う（最終 diff として現れない）。closed/0051 / closed/0054 の判定と同じ理屈。

## スコープ

含むもの:

- `src/hls/writer.rs::convert_length_prefixed_to_annexb` の Err 化と死コード分岐の整理（`length_size: 4` フォールバック、`Some(Avc1)` 分解の除去）、単体テスト追加
- `src/hls/writer.rs::extract_aac_config` の 3 経路 Err 化、単体テスト追加
- `src/hls/writer.rs` への `sample_entry_variant_name` ヘルパと `mod tests` の新設
- `src/video/h265.rs::tests` 内の `VPS_HEADER` / `PPS_HEADER` バイト列を `pub(crate) const HEVC_VPS_NAL` / `HEVC_PPS_NAL` として公開（h264 の `PPS_NAL` 公開化と同じ流儀。本 issue のテストで `Hvc1` 構築に利用するため）
- `docs/internals/sample_entry_invariant.md` の「writer 側の前提」節を §4 の書き換え案で書き換え

含まないもの:

- `Mp4Writer` / `HybridMp4Writer` 経路（既に muxer Err で fail-fast 停止する）
- `HlsWriter` の fMP4 経路と `DashWriter` の意味的整合性補強（現状の (c) 流用挙動は本 issue のスコープ外）
- `HlsWriter` インスタンス全体の単体テストカバレッジ拡充（closed/0051 の解決方法で明示された既存負債。本 issue では対象 3 関数に閉じたテストのみ追加する）

## 完了条件

- `convert_length_prefixed_to_annexb` / `extract_aac_config` で sample_entry None / 期待外型 / 中身異常時に Err を返すことが単体テストで保証されること
- 設計方針 1-1 の死コード分岐（`length_size: 4` フォールバック、`Some(Avc1)` 分解）が除去 / 簡素化されていること
- Err メッセージが設計方針 2 の粒度を満たすこと（経路接頭辞 + 期待状態 + 実際の状態を含み、`expected X, but got Y` 形式）
- `docs/internals/sample_entry_invariant.md` の「writer 側の前提」節が設計方針 §4 の書き換え案に置き換わっていること
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が default feature で通ること。本 issue の改修対象は `fdk-aac` / `nvcodec` feature と独立しているため、両 feature 付きビルドの検証は CI 側で担保される（本 issue では必須としない）
- 既存 e2e テスト（`e2e-tests/obsws/test_output.py::test_obsws_hls_start_stop_output`（MpegTs 経路）/ `test_obsws_hls_fmp4_start_stop_output`（fMP4 経路））が引き続き通ること

## 関連

- closed/0051（writer 入口 fallback 削除。本 issue の前提）
- closed/0054（openh264 / VideoToolbox エンコーダ側の出力保留設計。本 issue と性質が一対）
- `docs/internals/sample_entry_invariant.md`

## 解決方法
