# candle を用いた ML 推論機能 (文字起こし・物体検出) に正式対応する

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/add-candle-ml-inference
- Polished:

## 目的

PR #246 「[DO NOT MERGE] ML 機能をお試しする」(https://github.com/shiguredo/hisui/pull/246) で、Rust 製 ML 推論フレームワーク candle を用いた ML 機能の PoC を実装済み。このお試し成果のうち本番に値するものを選別し、正式な機能として hisui に取り込む。

具体的には次の 2 系統:

- 音声入力に対する Whisper 文字起こし (transcription)
- 映像フレームに対する YOLOv8 物体検出

特に文字起こしは、別 issue 0013「合成映像へのテキスト (字幕) 描画」と組み合わせて、合成映像への字幕オーバーレイ表示の基盤になる。これが本対応を進める主目的。

## 優先度根拠

- PoC は PR #246 で動作確認済みのため、ゼロからの研究ではなく「正式対応への引き上げ」であり実現性が高い。
- 一方で candle と関連クレート (candle-core / candle-nn / candle-transformers / candle-onnx, tokenizers) という比較的大きな依存追加を伴い、feature 設計・モデル配布・ビルド / CI への影響など設計判断が必要で、即マージできる小変更ではない。
- 文字起こしオーバーレイという明確なユースケースはあるが、現時点で業務を止めている課題ではない。
- 以上から High ではなく、Low でもなく Medium。

## 現状

- develop には ML 機能は無い (`src/ml/` は存在せず、CHANGES.md にも該当エントリ無し)。実装は PR #246 ブランチにのみ存在する。
- PR #246 で追加済みの構成 (正式対応の実装リファレンス):
  - 依存: `candle` feature 配下に candle-core / candle-nn / candle-transformers (=0.10.2)、Silero VAD 用に candle-onnx、tokenizers。
  - `src/ml/`: `device.rs` (CPU / Metal / CUDA デバイス選択)、`yolo.rs` (YOLOv8 物体検出、safetensors 重みロード、I420 フレームへの推論・描画)、`mod.rs`。
  - `src/ml/audio/`: `whisper.rs` / `silero_vad.rs` / `vad.rs` / `processor.rs` / `decode.rs` / `buffer.rs` / `config.rs` / `multilingual.rs` / `mod.rs` (Whisper 文字起こし + VAD)。
  - `src/subcommand_ml.rs`: `ml` / `ml audio` サブコマンド。`ml audio` はマイク入力 48 kHz を Whisper で文字起こしし、Silero VAD + RMS フォールバックで無音チャンクをスキップする。`--vad-trim` / `--language` / `--task` を持つ。`candle` feature のみでビルド可 (player 不要)。
  - `scripts/download_ml_models.sh`: Whisper tiny / Silero VAD / YOLOv8s の重み取得。
  - `docs/command_ml.md`: コマンド説明。
  - 映像系は MediaPipeline の VideoRealtimeMixer 上に ML 推論プロセッサとして統合 (モデルは `Arc` 共有で複数カメラへ適用)。ML 前処理の I420 → RGB 変換・リサイズは shiguredo_libyuv (SIMD) で実施。
- PoC のため [DO NOT MERGE] とされており、このまま develop へは入れられない。

## 設計方針

本 issue は「PoC をそのままマージする」のではなく、正式対応として以下を詰める。

1. feature 分割
   - `candle` feature を default off の optional とし、ML を使わない通常ビルドに影響を与えない。
   - 依存バージョンは hisui の方針 (Cargo.toml 冒頭 NOTE「依存は突然挙動が変わらないようバージョンは厳密一致で指定」) に従い `=0.10.2` の exact pin を維持する (AGENTS.md の「マイナーまで」より hisui の実態を優先する)。
   - 各依存に用途コメントを付す (AGENTS.md「依存ライブラリには用途をコメントで明記」)。
2. スコープの確定 (要判断)
   - PoC には Whisper 文字起こし・YOLO 物体検出・Silero VAD が含まれる。正式対応として全てを入れるか、まず文字起こし (+ VAD) に絞り YOLO は別 issue にするかを決める。文字起こしは 0013 と直結するため優先度が高い。
3. モデルの取得・配布
   - `scripts/download_ml_models.sh` でのダウンロード方式を維持するか別手段にするかを決める。モデルファイルはリポジトリに含めない。
4. デバイス選択
   - `src/ml/device.rs` の CPU / Metal / CUDA 切り替えを、hisui の対応プラットフォーム (macOS / Linux、nvcodec feature 等) と整合する形に整理する。
5. テスト方針
   - hisui は「モックやスタブを使わない」「PBT / fuzzing 優先」が規約。ML 推論は実モデル・実入力での検証になるため、CI でのモデル取得可否・実行コスト・推論結果の決定性 (ブレ) をどう扱うかを設計する。

## 完了条件

- candle を `candle` feature (default off) として正式に依存追加し、通常ビルド (`cargo build` / `cargo test`) に影響しないこと。
- 正式対応スコープに含めた機能 (最低限 Whisper 文字起こし) がサブコマンドとして動作すること。
- モデル取得手順がドキュメント化されていること。
- テスト方針が確定し、規約 (モック禁止) に反しない形で最低限の検証があること。
- CHANGES.md の `## develop` に該当エントリ ([ADD] candle …) を追記すること。

## 解決方法

- 実装は PR #246 をベースに、上記設計方針で取捨選択・整理して develop 向けに作り直す。
- スコープ (YOLO を含めるか) と CI でのモデル / 推論の扱いは、本 issue を `/polish-issue` で磨き上げる際に確定する。
- 推論結果を表に出す部分は本 issue では扱わない。映像への字幕重畳は 0013、外部への出力口は 0014 に委ねる。本 issue は推論結果をデータとして得るところまでを範囲とする。
