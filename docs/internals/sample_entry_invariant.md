# `sample_entry` 不変条件と入力経路の責務

## 概要

Hisui のメディアパイプラインでは、`AudioFrame` / `VideoFrame` の `sample_entry: Option<SharedSampleEntry>` フィールドが MP4 / fMP4 muxer に渡すサンプルエントリーを保持する。
このサンプルエントリーは muxer がトラックの最初のサンプルで必ず要求するため、writer 入口に届くフレームには「圧縮フォーマットなら必ず `Some` を持つ」という不変条件が要求される。

本ドキュメントでは、その不変条件の定義・適用範囲・各入力経路の責務・新規入力経路追加時のチェックリストを整理する。

## 不変条件

> **writer 入口に届く圧縮（エンコード済み）フレームの `sample_entry` は必ず `Some` でなければならない**

ここで「圧縮フレーム」とは `format.codec_name()` が `Some` を返すフレーム（コーデック名を持つ符号化済みデータ）を指す。
具体的には音声側で `Opus` / `Aac`、映像側で `H264` / `H265` / `Vp8` / `Vp9` / `Av1` 等が対象。

不変条件を満たさないフレーム（圧縮なのに `sample_entry: None`）は writer 側で対処手段を持たない（補完値の保険を入れないため、muxer が最初のサンプルでサンプルエントリーを得られず処理が破綻する）。

## 対象外

生フォーマット（`format.codec_name()` が `None` を返すフレーム）は不変条件の対象外であり、`sample_entry: None` が許容される。
具体的には音声側で `I16Be`、映像側で `I420` / `I420A` 等。

## 適用範囲（入力側全経路）

不変条件は writer の上流に位置する **すべての入力経路** で確立する必要がある。
責務分担は以下のとおり。

### リーダー（外部ソースから圧縮フレームを生成）

| ソース | ファイル | サンプルエントリー確立タイミング |
|---|---|---|
| MP4 ファイル | `src/mp4/reader.rs` | `stsd` ボックス読み出し時にトラックごとに確定し、各サンプルへ載せる |
| WebM ファイル | `src/webm/reader.rs` | `Tracks` 要素読み出し時にトラックごとに確定し、各サンプルへ載せる |
| RTSP 経路 | `src/rtsp/subscriber.rs` | SDP の `sprop-parameter-sets` または inline SPS / PPS / IDR 揃いで確定し、以降のフレームへ載せる |
| SRT 経路 | `src/srt/inbound_endpoint.rs` | AAC は `AudioSpecificConfig`、映像は AnnexB の SPS / PPS / IDR 揃いで確定し、以降のフレームへ載せる |
| Sora 録画 MP4 | `src/sora/recording_mp4_reader.rs` | `stsd` ボックス読み出し時にトラックごとに確定し、各サンプルへ載せる（MP4 リーダーと同じ経路） |

### エンコーダ（生フレームを符号化して圧縮フレームを生成）

| エンコーダ | ファイル | サンプルエントリー確立タイミング |
|---|---|---|
| openh264 | `src/encoder/openh264.rs` | 最初の出力フレームの SPS / PPS から確定し、以降の全フレームへ伝播 |
| svt_av1 | `src/encoder/svt_av1.rs` | コンストラクタで確定し、全フレームへ載せる |
| libvpx | `src/encoder/libvpx.rs` | コンストラクタで確定し、全フレームへ載せる |
| VideoToolbox | `src/encoder/video_toolbox.rs` | 最初の keyframe 出力時の SPS / PPS / VPS から確定し、以降の全フレームへ伝播 |
| NVENC | `src/encoder/nvcodec.rs` | コンストラクタで確定し、全フレームへ載せる |
| fdk-aac | `src/encoder/fdk_aac.rs` | コンストラクタで確定し、全フレームへ載せる |
| AudioToolbox | `src/encoder/audio_toolbox.rs` | コンストラクタで確定し、全フレームへ載せる |
| Opus | `src/encoder/opus.rs` | コンストラクタで確定し、全フレームへ載せる |

## 確立できない場合の扱い

リーダー側で SPS / PPS や `AudioSpecificConfig` 等のサンプルエントリー素材が揃わない場合は、**圧縮フレームを生成しない**ことで不変条件を保つ。
具体例:

- WebM リーダーで音声トラックが存在しない場合は圧縮 `AudioFrame` を生成しない（`track_number` 不一致でスキップ）
- RTSP 経路で SPS / PPS が未到来の場合はフレームをバッファリングして待機し、揃ってから圧縮 `VideoFrame` を生成する
- SRT 経路で AnnexB の SPS / PPS が未到来の場合も同様にバッファリング

エンコーダ側で「最初の keyframe より前に出力が出る」可能性を持つもの（openh264 / VideoToolbox 等）は、SPS / PPS 揃うまで出力を保留する設計とすることで不変条件を保つ。

## writer 側の前提

`src/mp4/writer.rs`（`Mp4Writer`）/ `src/mp4/hybrid_writer.rs`（`HybridMp4Writer`）/ `src/dash/writer.rs`（`DashWriter`）/ `src/hls/writer.rs`（`HlsWriter`）の writer 入口は補完値（fallback）や違反検知ロジックを持たず、入力側で不変条件が確立している前提で動作する。
万一不変条件が破られた場合は muxer が最初のサンプルで `MissingSampleEntry` Err を返してパイプラインを fail-fast 停止させる。
退行検知は各入力経路（リーダー / エンコーダ）の単体テストおよび e2e テストで担保する。

## 新規入力経路追加時のチェックリスト

新しいリーダー / エンコーダ / 入力経路を追加する際は、以下を必ず満たすこと。

1. **不変条件の宣言**: その経路が圧縮フレームを生成するか、生フレームを生成するかを明確にする。
2. **サンプルエントリー素材の入手経路**: 圧縮フレームを生成する場合、サンプルエントリーをいつどこから組み立てるかを設計に明示する（例: ファイルヘッダから、SDP から、エンコーダの最初の出力から、コンストラクタ引数から）。
3. **素材が揃わない場合の挙動**: 素材が揃わない時点では圧縮フレームを生成しない、もしくは生成を保留してバッファリングする方針を取る。`sample_entry: None` の圧縮フレームを下流に流すことは**してはならない**。
4. **テスト**: 単体テストまたは PBT で「圧縮フレームには必ず `sample_entry: Some` が載る」ことを検証する。
5. **`AudioFrame.sample_entry` / `VideoFrame.sample_entry` の docstring 更新**: 必要なら不変条件の範囲を更新する（例: 例外節がある場合）。

## 関連

- `src/audio.rs` `AudioFrame.sample_entry`: 音声側のフィールド定義と不変条件 docstring
- `src/video.rs` `VideoFrame.sample_entry`: 映像側のフィールド定義と不変条件 docstring
- `src/sample_entry.rs` `SharedSampleEntry`: サンプルエントリーを `Arc` で共有するための共通型
