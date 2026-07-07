# AMD AMF によるハードウェアビデオエンコード・デコードに対応する

- Priority: Medium
- Created: 2026-07-01
- Completed:
- Model: Opus 4.7
- Branch: feature/add-amf-encoder-decoder
- Polished:

## 目的

既存の NVENC/NVDEC 対応 (`feature = "nvcodec"`, `shiguredo_nvcodec`) と同様に、AMD GPU 上で動作するハードウェアビデオエンコーダー・デコーダーとして [`shiguredo_amf`](https://github.com/shiguredo/amf-rs) (AMD AMF: Advanced Media Framework) を hisui に統合する。これにより、AMD GPU 搭載環境でも合成パイプラインを GPU 側にオフロードでき、CPU エンコーダー (libvpx / openh264 / svt-av1) に強制フォールバックしなくて済むようにする。

## 優先度根拠

- 既に nvcodec によるハードウェアパスが用意されており、AMD GPU 環境の利用者だけが恩恵を受けられていない状態を解消する価値は大きい。
- ただし CPU パスは動作しており、AMD GPU が主戦場になっているわけではないため、機能停止レベルの問題ではない。High ではなく Medium が妥当。
- nvcodec 統合の設計・実装 (`src/encoder/nvcodec.rs` / `src/decoder/nvcodec.rs` 等) がリファレンスとして揃っており、新規設計要素は限定的で費用対効果が高いタイミング。

## 現状

- hisui は `shiguredo_nvcodec = { version = "=2026.2.0", optional = true }` を `Cargo.toml:99` で optional 依存として持ち、`Cargo.toml:130` の `nvcodec = ["shiguredo_nvcodec"]` で有効化する。
- 統合箇所は概ね以下 (`shiguredo_nvcodec` → `shiguredo_amf` 相当の置き換え対象):
  - `src/types.rs`: `EngineName::Nvcodec` 追加 (`:141`)、`nvcodec_supported_codecs()` / `to_nvcodec_codec()` によるコーデック対応判定 (`:173-188`)、`supported_engines_for_decoder()` / `supported_engines_for_encoder()` への組み込み (`:240-263`, `:291-314`)、`as_str()` / `TryFrom` の分岐 (`:340`, `:354-361`, `:389-396`)。
  - `src/decoder.rs`: `mod nvcodec` 宣言 (`:7-8`)、`NvcodecDecoder` インポート (`:20-21`)、`DecodeConfig` に `nvcodec_h264`/`h265`/`av1`/`vp8`/`vp9` 追加 (`:271-283`)、`Default` 実装 (`:292-320`)、`VideoDecoderInner` 側での `NvcodecDecoder::new_*` 呼び出し。
  - `src/encoder.rs`: `mod nvcodec` 宣言 (`:6-7`)、`NvcodecEncoder` インポート (`:27-28`)、`EncodeConfig` に `nvcodec_h264`/`h265`/`av1` 追加 (`:346-351`)、`VideoEncoderInner` 側での `NvcodecEncoder::new_*` 呼び出し。
  - `src/encoder/nvcodec.rs` (423 行) / `src/decoder/nvcodec.rs` (377 行): 実体。NV12 ↔ I420 変換 (libyuv 経由)、非同期エンコード完了コールバック→共有キュー、AV1 の Sequence Header OBU ワークアラウンド、Annex B ↔ MP4 変換、Sample Entry 生成、H.264/H.265 のパラメータセットキャッシュ等を含む。
  - `src/sora/recording_encoder_nvcodec_params.rs` / `src/sora/recording_decoder_nvcodec_params.rs`: JSON パーサー。
  - `src/sora/recording_layout_encode_params.rs`: `nvcodec_h264_encode_params` / `nvcodec_h265_encode_params` / `nvcodec_av1_encode_params` キーの読み取り。
- AMD GPU 環境では、上記のハードウェアエンジンが選ばれる余地がなく、CPU エンコーダー (libvpx / openh264 / svt-av1) にフォールバックしている。

## 設計方針

### 基本方針: nvcodec 統合パターンを踏襲する

`shiguredo_amf` を `feature = "amf"` として、既存 nvcodec 統合と同じ責務分割で組み込む。ファイル名は `nvcodec` → `amf` に置換したものを 1:1 で追加する。

- `Cargo.toml`
  - `shiguredo_amf = { version = "=2026.3.0", optional = true }` を optional 依存に追加。
  - `[features]` に `amf = ["shiguredo_amf"]` を追加。
- `src/types.rs`
  - `EngineName::Amf` を追加、`as_str()` は `"amf"`、`TryFrom` の分岐に `"amf"` を追加、supported_engines への組み込み。
  - AMD GPU / AMF ランタイムの検出は `shiguredo_amf::supported_codecs()` の戻り値が空でないことをチェックする方針 (README「AMF ランタイムがロードできない環境では、全コーデックが非対応として返される」)。nvcodec の `is_cuda_library_available()` に相当するランタイム検出フックとして扱う。
- `src/decoder.rs` / `src/encoder.rs`
  - `DecodeConfig` / `EncodeConfig` に `amf_h264` / `amf_h265` / `amf_av1` を追加 (feature gated)。VP8 / VP9 は AMF 非対応なので追加しない。
  - `Default` 実装は既存パターンに合わせる。
- `src/encoder/amf.rs` / `src/decoder/amf.rs`
  - nvcodec と同じく非同期コールバック→共有キューのパターンで書く。
  - 入力は I420 → NV12 変換 (libyuv 経由) を行い AMF に渡す。
  - H.264/H.265 は Annex B ↔ MP4 変換、AV1 は Sequence Header OBU の付与ワークアラウンドを含む既存ロジックを流用可能な範囲で共通化する (共通化するか個別に持たせるかは実装時に判断)。
  - Sample Entry 生成は既存の `video::h264` / `video::h265` / `video::av1` を再利用。
- `src/sora/recording_encoder_amf_params.rs` / `src/sora/recording_decoder_amf_params.rs`
  - nvcodec の JSON パーサーを雛形にする。
- `src/sora/recording_layout_encode_params.rs`
  - `amf_h264_encode_params` / `amf_h265_encode_params` / `amf_av1_encode_params` キーを追加。

### nvcodec との差分

以下は nvcodec とは異なるので、`shiguredo_amf` の API に合わせて個別に対応する。

| 項目 | nvcodec | amf |
| --- | --- | --- |
| 対応コーデック (エンコード) | H.264 / H.265 / AV1 | H.264 / H.265 / AV1 |
| 対応コーデック (デコード) | H.264 / H.265 / AV1 / VP8 / VP9 | H.264 / H.265 / AV1 (VP8 / VP9 非対応) |
| ランタイム検出 | `is_cuda_library_available()` | `supported_codecs()` の戻り値の非空判定 (README 記載) |
| 入力フレームの受け渡し | 生バイト列を `encode()` に渡す | `encoder.alloc_surface()` で AMF 側の Surface を確保し、Y/UV プレーンにコピーしてから `encode(surface, ...)` |
| 動作要件 | NVIDIA GPU + CUDA ライブラリ | AMD GPU + `libamfrt64.so.1` (AMD GPU ドライバー同梱) + Vulkan ドライバー |
| ビルド時依存 | (別途) | ビルド時に GitHub から AMF ヘッダーを自動取得 (git が必要) |

Surface alloc モデルの違いにより、`src/encoder/amf.rs` の `encode()` は「Surface を alloc → NV12 をコピー → encode」の 3 ステップに分かれる。既存の `src/encoder/nvcodec.rs:202-257` の `encode()` のフローとは若干異なるため、そのまま流用はできない。

### VP8 / VP9 の扱い

AMF は VP8 / VP9 のハードウェアエンコード・デコードに対応しない。`EngineName::Amf` はコーデック対応判定 (`src/types.rs` の `supported_engines_for_decoder` / `supported_engines_for_encoder` に相当) で H.264 / H.265 / AV1 のみ true を返し、VP8 / VP9 に対しては候補から外す。

### Default 実装

`EncodeConfig` / `DecodeConfig` の `Default` 実装で AMF 用のデフォルト値を初期化する。既存の nvcodec 用 `default_nvcodec_decoder_config()` (`src/decoder.rs:309-320`) と同じ構造で `default_amf_decoder_config()` / `default_amf_encoder_config()` を追加する。

## 完了条件

- `Cargo.toml` に `shiguredo_amf` の optional 依存が追加され、`[features] amf = ["shiguredo_amf"]` が追加されていること。
- `src/encoder/amf.rs` / `src/decoder/amf.rs` が実装され、H.264 / H.265 / AV1 の各コーデックでエンコード・デコードが動作すること。
- `src/types.rs` の `EngineName::Amf` および関連マッピングが nvcodec と同水準で整備されていること (as_str / TryFrom / supported_engines_for_encoder / supported_engines_for_decoder)。
- `src/decoder.rs` / `src/encoder.rs` の `DecodeConfig` / `EncodeConfig` に AMF 用フィールドが追加され、`Default` 実装が更新されていること。
- `src/sora/recording_encoder_amf_params.rs` / `src/sora/recording_decoder_amf_params.rs` が追加され、`src/sora/recording_layout_encode_params.rs` に AMF 用 JSON キーが追加されていること。
- `layout-examples/default.jsonc` (実体パスは要確認) に AMF の既定値エントリーが追加されていること。
- `cargo build` (feature `amf` 有効時 / 無効時双方) が成功すること。
- `cargo build --features amf` が Linux x86_64 環境で成功すること。macOS / Windows でビルドが失敗しないこと (feature 無効時)。
- `cargo test` が成功すること (feature 有効時 / 無効時双方)。
- AMD GPU 搭載の Linux 環境で H.264 / H.265 / AV1 の各コーデックのエンコード・デコードが実際に動作すること (合成結果を目視で確認したログを残す)。
- `CHANGES.md` の `## develop` に以下を追記:
  - `[ADD] AMD AMF によるハードウェアビデオエンコード・デコードに対応する`
    - 対応コーデック: H.264 / H.265 / AV1 (エンコード・デコード双方)
    - `feature = "amf"` で有効化する
    - 対応環境: Linux (x86_64) + AMD GPU (AMD GPU ドライバー同梱の `libamfrt64.so.1` を利用)

## 解決方法

### 実装ステップ

1. **`shiguredo_amf` の最新版と API を確認する**
   - `/Users/voluntas/shiguredo/amf-rs/README.md` および `/Users/voluntas/shiguredo/amf-rs/src/*.rs` を参照し、`EncoderConfig` / `DecoderConfig` / `Encoder` / `Decoder` / `Surface` / `EncodeOptions` / `RateControlMode` / `CodecConfig` / `H264EncoderConfig` / `HevcEncoderConfig` / `Av1EncoderConfig` の最終形を把握する。
   - `supported_codecs()` の戻り値型と、AMF ランタイムが無い環境での挙動 (全 codec が非対応で返る) を確認する。
2. **`Cargo.toml` を更新する**
   - `[dependencies]` に `shiguredo_amf = { version = "=2026.3.0", optional = true }` を追加。
   - `[features]` に `amf = ["shiguredo_amf"]` を追加。
3. **`src/types.rs` に `EngineName::Amf` を追加する**
   - `EngineName` の variant 追加、`as_str()` に `EngineName::Amf => "amf"`、`TryFrom` の分岐に `"amf"` (feature gate)。
   - `amf_supported_codecs()` / `to_amf_codec()` を追加。ランタイム検出は `shiguredo_amf::supported_codecs()` を呼び、結果を静的キャッシュする。
   - `supported_engines_for_encoder` / `supported_engines_for_decoder` に AMF 用の分岐を追加。VP8 / VP9 は対象外。
4. **`src/encoder.rs` / `src/decoder.rs` の `EncodeConfig` / `DecodeConfig` を更新する**
   - `amf_h264` / `amf_h265` / `amf_av1` フィールドを追加 (feature gated)。
   - `Default` 実装で `default_amf_*_config()` を呼ぶ。
5. **`src/encoder/amf.rs` を実装する**
   - `src/encoder/nvcodec.rs` を雛形にする。
   - Surface alloc モデルへの対応: `encoder.alloc_surface()` で AMF Surface を確保、libyuv で I420 → NV12 変換した結果を Surface にコピー、`encode(surface, options, ())` で送る。
   - H.264 / H.265 の Annex B ↔ MP4 変換、AV1 の Sequence Header OBU 付与ロジックは nvcodec と共通化できる部分は共通化する。
6. **`src/decoder/amf.rs` を実装する**
   - `src/decoder/nvcodec.rs` を雛形にする。
   - デコード出力は NV12 (README 記載通り) なので、既存の NV12 → I420 変換ロジックを再利用。
   - H.264 / H.265 のパラメータセット (SPS/PPS/VPS) 抽出・Annex B 変換ロジックは nvcodec と同一。VP8 / VP9 対応は削除。
7. **`src/sora/recording_encoder_amf_params.rs` / `src/sora/recording_decoder_amf_params.rs` を追加する**
   - `src/sora/recording_encoder_nvcodec_params.rs` を雛形にする。
   - `parse_h264_encode_params` / `parse_h265_encode_params` / `parse_av1_encode_params` を提供。
8. **`src/sora/recording_layout_encode_params.rs` に AMF 用のキーを追加する**
   - `amf_h264_encode_params` / `amf_h265_encode_params` / `amf_av1_encode_params` を対応させる。
   - `EncodeConfig` フィールドへの反映を追加。
9. **`layout-examples/default.jsonc` (実体パスは要確認) に AMF の既定値エントリーを追加する**
10. **`CHANGES.md` の `## develop` を更新する**
11. **AMD GPU 環境で動作確認する**
    - Linux x86_64 + AMD GPU 環境で `cargo build --features amf` → `cargo run --features amf -- compose ...` を H.264 / H.265 / AV1 それぞれで実行し、出力ファイルが再生できることを確認する。
    - 検証ログ (実行コマンド + 出力ファイルのメタ情報 + 再生確認結果) を PR 本文に残す。

### リスク・留意点

- **AMD GPU 検証環境の確保**: 開発機で AMD GPU が使えない場合、動作確認は別環境で実施する必要がある。エミュレーションは不可 (AMF ランタイムが AMD GPU ドライバーに同梱される仕様)。
- **`libamfrt64.so.1` のバージョン差異**: AMD GPU ドライバーのバージョンによって AMF ランタイムの版が異なる。動作確認時に使用したドライバーバージョンを PR 本文に明記する。
- **Vulkan ドライバー依存**: AMF は Vulkan バックエンドを使うため、Vulkan ドライバーがインストールされていない環境では初期化失敗する。エラーメッセージから原因が分かるようにする (`shiguredo_amf::Error` をそのまま `crate::Error` に変換して伝達)。
- **Surface alloc モデルの違い**: nvcodec は生バイト列を `encode()` に渡すが、AMF は Surface を alloc してからコピーする必要がある。この違いを吸収するために `src/encoder/nvcodec.rs` の `encode()` フローをそのまま流用することはできず、AMF 用に書き直す。
- **AV1 Sequence Header OBU の扱い**: nvcodec 側で「NVENC SDK 13.0 の挙動として二番目以降のキーフレームに Sequence Header が付かない」というワークアラウンドが入っている (`src/encoder/nvcodec.rs:174-198`)。AMF でも同様の挙動があるかは動作確認時に検証する。
- **feature 無効時のコンパイル**: `feature = "amf"` を無効化した状態でも `cargo build` が通ることを CI で担保する。既存 nvcodec と同じく全ての AMF 参照箇所を `#[cfg(feature = "amf")]` でガードする。
- **依存 crate の更新頻度**: `shiguredo_amf` の新バージョンが出た場合は、依存を上げた PR 内で `EncoderConfig` のフィールド追加・削除に追従する運用にする (issue 0005 の方針と同じ)。

## 関連

- open/0067 (`feature/refactor-add-async-video-encoder`): VideoEncoder 系の Sender 化 (`AsyncVideoEncoder` 追加 + wrap 化 + inner Sender 化)。 本 issue で追加する AMF encoder は 0067 の Sender 化 API (`OutputSink` / `AsyncVideoEncoder`) を雛形として踏襲するため、 0067 完了後に着手する
- closed/0066 (`feature/refactor-add-async-video-decoder`) / closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`) / closed/0078 (`feature/refactor-remove-unused-next-decoded-frame`): VideoDecoder 系の Sender 化 (完了)。 本 issue で追加する AMF decoder は Sender 化された `VideoDecoder` API を雛形として踏襲する
