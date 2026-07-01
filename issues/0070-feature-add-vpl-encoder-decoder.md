# Intel VPL によるハードウェアビデオエンコード・デコードに対応する

- Priority: Medium
- Created: 2026-07-01
- Completed:
- Model: Opus 4.7
- Branch: feature/add-vpl-encoder-decoder
- Polished:

## 目的

既存の NVENC/NVDEC 対応 (`feature = "nvcodec"`, `shiguredo_nvcodec`) と同様に、Intel GPU 上で動作するハードウェアビデオエンコーダー・デコーダーとして [`shiguredo_vpl`](https://github.com/shiguredo/vpl-rs) (Intel VPL: Video Processing Library) を hisui に統合する。これにより、Intel GPU 搭載環境でも合成パイプラインを GPU 側にオフロードでき、CPU エンコーダー (libvpx / openh264 / svt-av1) に強制フォールバックしなくて済むようにする。

## 優先度根拠

- 既に nvcodec によるハードウェアパスが用意されており、Intel GPU 環境の利用者だけが恩恵を受けられていない状態を解消する価値は大きい。特に第 6 世代 Core 以降を搭載する一般的な Intel マシンでもハードウェア合成が使えるようになる。
- ただし CPU パスは動作しており、機能停止レベルの問題ではない。High ではなく Medium が妥当。
- nvcodec 統合の設計・実装 (`src/encoder/nvcodec.rs` / `src/decoder/nvcodec.rs` 等) がリファレンスとして揃っており、新規設計要素は限定的で費用対効果が高いタイミング。

## 現状

- hisui は `shiguredo_nvcodec = { version = "=2026.2.0", optional = true }` を `Cargo.toml:99` で optional 依存として持ち、`Cargo.toml:130` の `nvcodec = ["shiguredo_nvcodec"]` で有効化する。
- 統合箇所は概ね以下 (`shiguredo_nvcodec` → `shiguredo_vpl` 相当の置き換え対象):
  - `src/types.rs`: `EngineName::Nvcodec` 追加 (`:141`)、`nvcodec_supported_codecs()` / `to_nvcodec_codec()` によるコーデック対応判定 (`:173-188`)、`supported_engines_for_decoder()` / `supported_engines_for_encoder()` への組み込み (`:240-263`, `:291-314`)、`as_str()` / `TryFrom` の分岐 (`:340`, `:354-361`, `:389-396`)。
  - `src/decoder.rs`: `mod nvcodec` 宣言 (`:7-8`)、`NvcodecDecoder` インポート (`:20-21`)、`DecodeConfig` に `nvcodec_h264`/`h265`/`av1`/`vp8`/`vp9` 追加 (`:271-283`)、`Default` 実装 (`:292-320`)、`VideoDecoderInner` 側での `NvcodecDecoder::new_*` 呼び出し。
  - `src/encoder.rs`: `mod nvcodec` 宣言 (`:6-7`)、`NvcodecEncoder` インポート (`:27-28`)、`EncodeConfig` に `nvcodec_h264`/`h265`/`av1` 追加 (`:346-351`)、`VideoEncoderInner` 側での `NvcodecEncoder::new_*` 呼び出し。
  - `src/encoder/nvcodec.rs` (423 行) / `src/decoder/nvcodec.rs` (377 行): 実体。NV12 ↔ I420 変換 (libyuv 経由)、非同期エンコード完了コールバック→共有キュー、AV1 の Sequence Header OBU ワークアラウンド、Annex B ↔ MP4 変換、Sample Entry 生成、H.264/H.265 のパラメータセットキャッシュ等を含む。
  - `src/sora/recording_encoder_nvcodec_params.rs` / `src/sora/recording_decoder_nvcodec_params.rs`: JSON パーサー。
  - `src/sora/recording_layout_encode_params.rs`: `nvcodec_h264_encode_params` / `nvcodec_h265_encode_params` / `nvcodec_av1_encode_params` キーの読み取り。
- Intel GPU 環境では、上記のハードウェアエンジンが選ばれる余地がなく、CPU エンコーダー (libvpx / openh264 / svt-av1) にフォールバックしている。

## 設計方針

### 基本方針: nvcodec 統合パターンを踏襲する

`shiguredo_vpl` を `feature = "vpl"` として、既存 nvcodec 統合と同じ責務分割で組み込む。ファイル名は `nvcodec` → `vpl` に置換したものを 1:1 で追加する。

- `Cargo.toml`
  - `shiguredo_vpl = { version = "=2026.4.0-canary.0", optional = true }` を optional 依存に追加 (現時点の最新版に追従。安定版が出た時点で version を上げる)。
  - `[features]` に `vpl = ["shiguredo_vpl"]` を追加。
- `src/types.rs`
  - `EngineName::Vpl` を追加、`as_str()` は `"vpl"`、`TryFrom` の分岐に `"vpl"` を追加、supported_engines への組み込み。
  - Intel GPU / VPL ランタイムの検出は `shiguredo_vpl::list_adapters()` の戻り値が空でないことをチェックする方針。nvcodec の `is_cuda_library_available()` に相当するランタイム検出フックとして扱う。
- `src/decoder.rs` / `src/encoder.rs`
  - `DecodeConfig` / `EncodeConfig` に `vpl_h264` / `vpl_h265` / `vpl_vp9` / `vpl_av1` を追加 (feature gated)。VP8 は VPL 非対応なので追加しない。
  - `Default` 実装は既存パターンに合わせる。
- `src/encoder/vpl.rs` / `src/decoder/vpl.rs`
  - nvcodec と同じく非同期コールバック→共有キューのパターンで書く (`FnEncodeHandler` / `FnDecodeHandler` は nvcodec と同一設計)。
  - 入力は I420 → NV12 変換 (libyuv 経由) を行い VPL に渡す。
  - H.264/H.265 は Annex B ↔ MP4 変換、AV1 は Sequence Header OBU の付与ワークアラウンドを含む既存ロジックを流用可能な範囲で共通化する (共通化するか個別に持たせるかは実装時に判断)。
  - Sample Entry 生成は既存の `video::h264` / `video::h265` / `video::av1` を再利用。
- `src/sora/recording_encoder_vpl_params.rs` / `src/sora/recording_decoder_vpl_params.rs`
  - nvcodec の JSON パーサーを雛形にする。
- `src/sora/recording_layout_encode_params.rs`
  - `vpl_h264_encode_params` / `vpl_h265_encode_params` / `vpl_vp9_encode_params` / `vpl_av1_encode_params` キーを追加。

### nvcodec との差分

以下は nvcodec とは異なるので、`shiguredo_vpl` の API に合わせて個別に対応する。

| 項目 | nvcodec | vpl |
| --- | --- | --- |
| 対応コーデック (エンコード) | H.264 / H.265 / AV1 | H.264 / H.265 / VP9 / AV1 |
| 対応コーデック (デコード) | H.264 / H.265 / AV1 / VP8 / VP9 | H.264 / H.265 / VP9 / AV1 (VP8 非対応) |
| ランタイム検出 | `is_cuda_library_available()` | `list_adapters()` の戻り値の非空判定 |
| GPU 選択 | `device_id: 0` を直接 config に指定 | `AdapterSelector::DrmRenderNode(<n>)` を `EncoderConfig::new` / `DecoderConfig::new` に必ず渡す必要がある |
| 動作要件 | NVIDIA GPU + CUDA ライブラリ | Intel GPU (第 6 世代 Core 以降) + `/dev/dri/renderD<N>` (DRM render node) |
| ビルド時依存 | (別途) | ビルド時に GitHub から libvpl を取得し CMake で static link (clang / git が必要) |

### GPU アダプタ選択方針

Intel VPL は `AdapterSelector` を必須引数として取るため、既定選択ロジックを hisui 側で用意する必要がある。以下方針で組み込む:

- 既定は `shiguredo_vpl::list_adapters()` の先頭 Intel GPU アダプタを使う。
- 実運用では複数 GPU 環境も想定されるため、layout JSON および `EncodeConfig` / `DecodeConfig` から DRM render node 番号を明示指定できるようにする (キー名は `vpl_adapter_drm_render_node` などを候補として実装時に確定)。
- アダプタが 1 つも存在しない場合は `EngineName::Vpl` は候補から除外し (`supported_engines_for_encoder` / `supported_engines_for_decoder` の判定で除く)、他エンジンへフォールバックする。

### VP8 の扱い

VPL は VP8 のハードウェアエンコード・デコードに対応しない。`EngineName::Vpl` はコーデック対応判定 (`src/types.rs` の `supported_engines_for_decoder` / `supported_engines_for_encoder` に相当) で H.264 / H.265 / VP9 / AV1 のみ true を返し、VP8 に対しては候補から外す。

### VP9 対応

VPL は他ハードウェアエンジン (nvcodec / amf) と異なり VP9 エンコードにも対応する。既存の VP9 エンコード対応は libvpx のみだったので、`EngineName::Vpl` が VP9 エンコード候補として新たに登場することになる。`src/types.rs` の `supported_engines_for_encoder` の VP9 分岐でも `EngineName::Vpl` を候補に含める。

### Default 実装

`EncodeConfig` / `DecodeConfig` の `Default` 実装で VPL 用のデフォルト値を初期化する。既存の nvcodec 用 `default_nvcodec_decoder_config()` (`src/decoder.rs:309-320`) と同じ構造で `default_vpl_decoder_config()` / `default_vpl_encoder_config()` を追加する。ただし `AdapterSelector` の指定が必須なため、`Default` 実装内で `list_adapters()` を呼ぶか、あるいは実際にエンコーダー / デコーダーを構築する直前でアダプタを決定するか (config を `Option` で持って生成時に確定する等) は実装時に判断する。

## 完了条件

- `Cargo.toml` に `shiguredo_vpl` の optional 依存が追加され、`[features] vpl = ["shiguredo_vpl"]` が追加されていること。
- `src/encoder/vpl.rs` / `src/decoder/vpl.rs` が実装され、H.264 / H.265 / VP9 / AV1 の各コーデックでエンコード・デコードが動作すること。
- `src/types.rs` の `EngineName::Vpl` および関連マッピングが nvcodec と同水準で整備されていること (as_str / TryFrom / supported_engines_for_encoder / supported_engines_for_decoder)。VP9 エンコードの候補に `EngineName::Vpl` が正しく含まれること。
- `src/decoder.rs` / `src/encoder.rs` の `DecodeConfig` / `EncodeConfig` に VPL 用フィールドが追加され、`Default` 実装が更新されていること。
- `src/sora/recording_encoder_vpl_params.rs` / `src/sora/recording_decoder_vpl_params.rs` が追加され、`src/sora/recording_layout_encode_params.rs` に VPL 用 JSON キーが追加されていること。
- `layout-examples/default.jsonc` (実体パスは要確認) に VPL の既定値エントリーが追加されていること。
- DRM render node の指定機構 (layout JSON および CLI 経由) が実装され、既定は先頭 Intel GPU アダプタが選ばれること。
- `cargo build` (feature `vpl` 有効時 / 無効時双方) が成功すること。
- `cargo build --features vpl` が Linux x86_64 環境で成功すること。macOS / Windows でビルドが失敗しないこと (feature 無効時)。
- `cargo test` が成功すること (feature 有効時 / 無効時双方)。
- Intel GPU (第 6 世代 Core 以降) 搭載の Linux 環境で H.264 / H.265 / VP9 / AV1 の各コーデックのエンコード・デコードが実際に動作すること (合成結果を目視で確認したログを残す)。
- `CHANGES.md` の `## develop` に以下を追記:
  - `[ADD] Intel VPL によるハードウェアビデオエンコード・デコードに対応する`
    - 対応コーデック: H.264 / H.265 / VP9 / AV1 (エンコード・デコード双方)
    - `feature = "vpl"` で有効化する
    - 対応環境: Linux (x86_64) + Intel GPU (第 6 世代 Core 以降)

## 解決方法

### 実装ステップ

1. **`shiguredo_vpl` の最新版と API を確認する**
   - `/Users/voluntas/shiguredo/vpl-rs/README.md` および `/Users/voluntas/shiguredo/vpl-rs/src/*.rs` を参照し、`EncoderConfig` / `DecoderConfig` / `Encoder` / `Decoder` / `AdapterSelector` / `list_adapters()` / `EncodeOptions` / `RateControlMode` / `CodecConfig` / `H264EncoderConfig` / `HevcEncoderConfig` / `Vp9EncoderConfig` / `Av1EncoderConfig` の最終形を把握する。
   - `FnEncodeHandler` / `FnDecodeHandler` のインターフェースが nvcodec と同一かを確認する (同一なら共有キュー・エラー保持スロットのパターンをそのまま流用可能)。
2. **`Cargo.toml` を更新する**
   - `[dependencies]` に `shiguredo_vpl = { version = "=2026.4.0-canary.0", optional = true }` を追加。
   - `[features]` に `vpl = ["shiguredo_vpl"]` を追加。
3. **`src/types.rs` に `EngineName::Vpl` を追加する**
   - `EngineName` の variant 追加、`as_str()` に `EngineName::Vpl => "vpl"`、`TryFrom` の分岐に `"vpl"` (feature gate)。
   - `vpl_supported_codecs()` / `to_vpl_codec()` を追加。ランタイム検出は `shiguredo_vpl::list_adapters()` を呼び、結果を静的キャッシュする (アダプタ数 > 0 なら Vpl を候補に含める)。
   - `supported_engines_for_encoder` / `supported_engines_for_decoder` に VPL 用の分岐を追加。VP8 は対象外、VP9 は対象。
4. **`src/encoder.rs` / `src/decoder.rs` の `EncodeConfig` / `DecodeConfig` を更新する**
   - `vpl_h264` / `vpl_h265` / `vpl_vp9` / `vpl_av1` フィールドを追加 (feature gated)。
   - `Default` 実装で `default_vpl_*_config()` を呼ぶ。`AdapterSelector` 必須の問題に対する扱いは実装時に決める。
5. **`src/encoder/vpl.rs` を実装する**
   - `src/encoder/nvcodec.rs` を雛形にする。
   - `list_adapters()` から既定アダプタを選ぶロジックを追加。設定で上書き可能にする。
   - コールバック→共有キューのパターンは nvcodec と同じ。
   - H.264 / H.265 の Annex B ↔ MP4 変換、AV1 の Sequence Header OBU 付与ロジックは nvcodec と共通化できる部分は共通化する。
   - VP9 のペイロード形式は他エンコーダー実装 (libvpx) と揃える。
6. **`src/decoder/vpl.rs` を実装する**
   - `src/decoder/nvcodec.rs` を雛形にする。
   - デコード出力は NV12 (README 記載通り) なので、既存の NV12 → I420 変換ロジックを再利用。
   - H.264 / H.265 のパラメータセット (SPS/PPS/VPS) 抽出・Annex B 変換ロジックは nvcodec と同一。VP8 対応は削除、VP9 対応は追加。
7. **`src/sora/recording_encoder_vpl_params.rs` / `src/sora/recording_decoder_vpl_params.rs` を追加する**
   - `src/sora/recording_encoder_nvcodec_params.rs` を雛形にする。
   - `parse_h264_encode_params` / `parse_h265_encode_params` / `parse_vp9_encode_params` / `parse_av1_encode_params` を提供。
8. **`src/sora/recording_layout_encode_params.rs` に VPL 用のキーを追加する**
   - `vpl_h264_encode_params` / `vpl_h265_encode_params` / `vpl_vp9_encode_params` / `vpl_av1_encode_params` を対応させる。
   - `EncodeConfig` フィールドへの反映を追加。
   - DRM render node を指定するキー (例: `vpl_adapter_drm_render_node`) の JSON 経路を確保する。
9. **`layout-examples/default.jsonc` (実体パスは要確認) に VPL の既定値エントリーを追加する**
10. **`CHANGES.md` の `## develop` を更新する**
11. **Intel GPU 環境で動作確認する**
    - Linux x86_64 + Intel GPU (第 6 世代 Core 以降) 環境で `cargo build --features vpl` → `cargo run --features vpl -- compose ...` を H.264 / H.265 / VP9 / AV1 それぞれで実行し、出力ファイルが再生できることを確認する。
    - 検証ログ (実行コマンド + 出力ファイルのメタ情報 + 再生確認結果) を PR 本文に残す。

### リスク・留意点

- **Intel GPU 検証環境の確保**: 開発機で Intel GPU が使えない場合、動作確認は別環境で実施する必要がある。エミュレーションは不可 (`/dev/dri/renderD*` の DRM render node が必要)。
- **DRM render node の可視性**: コンテナ環境では `/dev/dri/` が明示的にマウントされていないと VPL が初期化失敗する。動作確認時に使用した環境 (bare metal / コンテナ + マウント指定 / VM + PCIe passthrough 等) を PR 本文に明記する。
- **`AdapterSelector` の必須化**: nvcodec の `device_id: 0` と異なり、VPL はアダプタ選択が config 生成時に必須。`EncodeConfig::default()` などで簡易に扱えない可能性があり、`Default` 実装の設計を再検討する必要がある (`Option` で持って生成時に確定する等)。
- **ビルド時依存の増加**: `libvpl` を CMake で static build するため、ビルド環境に `git` / `clang` (bindgen 依存) が必要になる。既存 CI やビルドドキュメントに追記する。
- **AV1 Sequence Header OBU の扱い**: nvcodec 側で「NVENC SDK 13.0 の挙動として二番目以降のキーフレームに Sequence Header が付かない」というワークアラウンドが入っている (`src/encoder/nvcodec.rs:174-198`)。VPL でも同様の挙動があるかは動作確認時に検証する。
- **feature 無効時のコンパイル**: `feature = "vpl"` を無効化した状態でも `cargo build` が通ることを CI で担保する。既存 nvcodec と同じく全ての VPL 参照箇所を `#[cfg(feature = "vpl")]` でガードする。
- **依存 crate の更新頻度**: `shiguredo_vpl` の新バージョンが出た場合は、依存を上げた PR 内で `EncoderConfig` のフィールド追加・削除に追従する運用にする (issue 0005 の方針と同じ)。canary 版に依存している間は特に追従頻度が高くなる可能性がある。
- **エンジン選択の優先順位**: 複数 GPU が搭載された環境 (例: Intel iGPU + NVIDIA dGPU) では、既定でどのエンジンを選ぶかの方針が必要。既存の `supported_engines_for_encoder` の順序に従うが、実運用で不都合が出た場合は明示指定できる経路 (layout JSON) を利用する。
