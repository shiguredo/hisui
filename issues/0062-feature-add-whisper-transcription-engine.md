# Whisper 文字起こしエンジンと TranscriptionService/Processor を実装する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-whisper-transcription-engine
- Polished: 2026-07-06

## 目的

Whisper モデルによる音声文字起こしのライブラリ層と、複数音声トラックを並列処理できる `TranscriptionService` (ワーカープール) + `TranscriptionProcessor` (MediaPipeline 上の processor) を実装する。親 issue 0012 系列の推論基盤層。

## 優先度根拠

本系列の最終層 0063 (利用者向けサブコマンド) の前提となる中核 issue。Medium。

## 現状

- hisui には文字起こし機能がない。`src/` に Whisper / Transcription 系のシンボルは存在しない
- 0059 (closed) で candle feature (candle-core / candle-nn / candle-transformers / candle-onnx / tokenizers は Cargo.toml 導入済み)、`src/ml.rs` + `src/ml/device.rs` (`select_device`)、`scripts/download_ml_models.py` (`whisper-tiny` / `silero-vad` ターゲット、SHA256 検証付き)、test-candle CI job が実装済み
- 0060 (closed) で `MediaFrame::Text(Arc<TextFrame>)` と `TextFrame` (`src/text.rs`: start / end / text / language / no_speech_prob / avg_logprob) が実装済み
- 0061 (closed) で `src/ml/audio/` に `buffer.rs` (`AudioChunkBuffer`) / `config.rs` (`VadConfig`) / `silero_vad.rs` (`SileroVadModel` / `SileroVad`) / `vad.rs` (`VadGate` / `SpeechSegment`)、`src/audio/resample.rs` に `resample_to_mono` (f32 入力、`SUPPORTED_HZ` 限定、バッチ設計) が実装済み
- 移植元は PoC ブランチ `origin/feature/try-candle` (PR #246): `src/ml/audio/whisper.rs` (`WhisperPipeline`) / `decode.rs` (`WhisperModel` / `Decoder`) / `multilingual.rs` (言語検出 + LANGUAGES テーブル) / `melfilters.bytes` (80-bin mel filter bank バイナリ、`include_bytes!` で参照) / `config.rs` 内の `load_whisper_config` (nojson による HF config.json パーサ)。PoC の processor (`AudioMlProcessor`) は結果をログ出力するのみで publish しない
- 既存音声デコーダ (Opus / AAC / AudioToolbox / fdk-aac) の出力 `AudioFrame` は `AudioFormat::I16Be` (i16 BE、mono / stereo interleave)。I16Be → f32 正規化のヘルパーは未実装 (既存の i16 iterator は stereo 限定のため流用できない)
- `testdata/` に発話実音声は存在しない (beep・カラーバー系のみ)
- CI の test-candle job は silero-vad モデルのみダウンロードし、job env `HISUI_CI: "1"` によりモデル不在の integration テストは skip ではなく panic する

## 設計方針

### スコープ境界 (0063 との分担)

- 本 issue: Whisper ライブラリ層、`TranscriptionService` / `TranscriptionProcessor`、testdata 実音声追加、integration テスト、test-candle CI への whisper-tiny 追加
- 0063: `-x transcribe` サブコマンド、WAV / MP4 入力、e2e テスト、ドキュメント、CHANGES.md の [ADD] エントリ。Silero VAD モデルパスを渡す CLI の口 (`--model-dir` は Whisper 専用のため別途必要) も 0063 側で確定する

### モジュール構成

`src/ml/audio/` 配下に以下を新規追加する。親 `pub mod ml;` が `src/lib.rs` で `#[cfg(feature = "candle")]` 済みのため追加 cfg は不要。`src/ml/audio.rs` に `pub mod` を追記し、`pub use` 再エクスポートはしない (モジュールパス参照の規約):

- `whisper.rs`: `WhisperPipeline` (モデルディレクトリから config.json / tokenizer.json / model.safetensors をロードし、PCM → mel → encode → decode を実行)。HF config.json パーサ (PoC `config.rs` の `load_whisper_config` 相当。ファイルとしての `config.rs` は移植せず、この関数だけを取り込む) も本ファイルに置く (既存 `config.rs` は VadConfig 専用のため)。mel フィルタは PoC の `melfilters.bytes` (80-bin、candle-examples 同梱由来) をコピーして `include_bytes!` する。128-bin (large-v3 系) は PoC 同様 Err にする
- `decode.rs`: `WhisperModel` / `Decoder` (KV cache 管理、decode ループ)。リクエスト間で KV cache を reset し、状態を持ち越さない
- `multilingual.rs`: 多言語モデル判定 (`is_multilingual_config`) と、指定 ISO 639-1 コード → Whisper 言語トークン変換 (`language_token_from_code`)。言語は必須指定とし、モデルによる自動検出は行わない (PoC の `detect_language` と LANGUAGES テーブルは移植しない)
- `transcription_service.rs`: `TranscriptionService` (ワーカープール) と `TranscriptRequest` / `TranscriptResult`
- `transcription_processor.rs`: `TranscriptionProcessor` (MediaPipeline processor)。I16Be → f32 正規化・resample 蓄積・SpeechSegment 30 秒分割の補助関数もここに置く

加えて `src/ml/audio/vad.rs` (0061 成果物) に小拡張を 1 つ入れる: 保持が必要な最小サンプル番号を返す `min_required_sample` (`Idle` なら `sample_count`、`InSpeech` / `Trailing` なら `speech.start_sample`)。Processor の PCM 破棄判定に使う (後述)。

PoC からの移植対象は `whisper.rs` / `decode.rs` / `multilingual.rs` / `melfilters.bytes` と processor の制御フローの参考のみ。PoC の `buffer.rs` / `vad.rs` / `mod.rs` は現行 develop 実装と別物なので移植しない (PoC 側 API に引き摺られないこと)。

### Whisper 入力制約 (30 秒 / mel パディング)

- candle の `pcm_to_mel` はパディングにより mel フレーム数が 1500 の倍数 + 1500 になるため、PoC の `transcribe_pcm16k` のように mel 全体を encoder に渡すと、15 秒を超える PCM で `max_source_positions = 1500` を超過して必ず Err になる
- 対策は 2 段構え:
  - `whisper.rs`: mel を `min(3000, mel フレーム数)` に narrow してから encode する (mel の Vec は [bins][frames] レイアウトなので Tensor 化後に dim 2 を narrow する。PoC `detect_language` に同型コードがある)
  - Processor 側: `SpeechSegment` の PCM を最大 30 秒 (480,000 サンプル @16 kHz) ごとに分割してから submit する。分割された各チャンクは独立の TextFrame になる。160 サンプル (10 ms) 未満の端数チャンクは捨てる (mel は最低 1500 フレーム生成されるため Err にはならないが、ほぼ全パディングの窓に encode / decode を 1 回費やす無駄になる上、10 ms 未満の音声に文字起こし価値がない)。分割は純関数として実装する
- 0061 申し送りの「`AudioChunkBuffer::new(30 * 16000)` 流用」は採らない (可変長 SpeechSegment の slice 分割には固定長 pull 型 buffer が合わないため)

### TranscriptionService

```
TranscriptionService
  ├ new(model_dir, device) で 1 個の WhisperPipeline をロード
  ├ 推論キュー: tokio::sync::mpsc の bounded channel (容量 2)
  └ 1 個の worker (tokio::task::spawn_blocking で常駐、キューから取って推論)
```

- 単一 worker とする理由: candle CPU 推論は既定でホスト物理コア数まで並列化するため、 hisui 側で worker を複数持つと per-decode の並列度がコア競合で相殺される。実効スループットは「1 worker + `RAYON_NUM_THREADS` を絞らない」で頭打ちになるので pool 化はしない。将来 GPU 複数枚 / 極小 decode で並列化する余地が出たら復活を検討する
- 投入 API (確定):

```rust
pub struct TranscriptRequest {
    /// 16 kHz mono f32 PCM (最大 30 秒 = 480_000 サンプル)
    pub pcm: Vec<f32>,
    /// ISO 639-1 言語コード (多言語モデルで必須)
    pub language: String,
}

pub struct TranscriptResult {
    pub text: String,
    /// 指定された言語 (ISO 639-1)。多言語 config では Some
    pub language: Option<String>,
    /// TextFrame の f32 幅に揃える (Whisper は常に値を返すので Option にしない)
    pub no_speech_prob: f32,
    pub avg_logprob: f32,
}

impl TranscriptionService {
    /// キューが満杯なら空くまで待つ (backpressure。オフライン入力での積み上がり防止)。
    /// worker 全滅などで send が失敗した場合は Err を詰めた oneshot を返す
    pub async fn submit(&self, request: TranscriptRequest)
        -> tokio::sync::oneshot::Receiver<crate::Result<TranscriptResult>>;
}
```

- 言語はリクエストで必須指定する。モデルによる言語自動検出は行わない (whisper-tiny の検出精度が低く、誤検出がトラック全体の文字起こしを劣化させるため)。必要になれば別 issue でライブラリ層に追加する
- 非多言語 config のモデルに `language` 指定が来た場合は Err を返す (構成ミスとして Processor がエラー終了する)
- 0063 の `--language` は必須オプションとし、Processor 経由で `TranscriptRequest.language` に載せる (0063 が依存する境界面)
- shutdown: 全 `Arc<TranscriptionService>` の drop でキュー sender が閉じ、worker はキュー内・処理中のリクエストを drain して完了させてから終了する (tokio mpsc は sender 全 drop 後も buffered メッセージを recv できる)。oneshot が drop されて受信側が RecvError になるのは worker panic 時のみ
- `new` は tokio runtime 内で呼ぶこと (spawn_blocking を使うため)。runtime shutdown より先に全 `Arc` を drop する必要がある (0063 で subcommand が生成する際の注意)
- 生成の所有者: 本 issue では in-process pipeline test が生成する。0063 では subcommand が生成して `Arc` で各 Processor に配る

### TranscriptionProcessor (MediaPipeline processor)

1 processor = 1 入力 audio track = 1 出力 text track。`ProcessorMetadata::new("transcription")` (processor_type) とする。

コンストラクタ境界 (0063 も同じ口を使う):

- `new(service: Arc<TranscriptionService>, silero: Arc<SileroVadModel>, language: String, 入力 TrackId, 出力 TrackId)` (引数の集合はこのとおり。型の細部・順序は実装時に確定してよい)
- Silero VAD モデルは呼び出し側 (テスト / 0063 の subcommand) が `SileroVadModel::load` でロードして `Arc` で配り、Processor が `new_instance()` で track 専用の `SileroVad` を派生させて `VadGate::new(instance, VadConfig::default())` する

処理の流れ:

```
subscribe_track(audio_track_id)
  → AudioFrame (I16Be) を i16 → f32 正規化
  → 1 秒分 (src_hz フレーム。ステレオ interleave なら 2 倍の値数) 蓄積して
    resample_to_mono で 16 kHz mono 化
  → VadGate::feed で発話区間抽出
  → SpeechSegment の PCM を slice し、最大 30 秒に分割して TranscriptionService::submit
  → oneshot::Receiver を FIFO で待ち、TextFrame を publish_track
```

- run ループの骨格は `src/decoder.rs` の `AudioDecoder::run` (subscribe → publish → notify_ready → wait_subscribers_ready → recv ループ、`Message::Syn` は無視) に従う。「入力受信」と「先頭 oneshot の完了待ち」を select で並行し、完了した結果から順に publish する
- resample を 1 秒単位にする理由: `resample_to_mono` はバッチ設計 (フィルタ状態を持ち回さない) で出力長が ceil 丸めされるため、フレーム単位で呼ぶと丸めが累積してサンプル通し番号がずれる。1 秒分なら `SUPPORTED_HZ` のどのレートでも 16,000 サンプルちょうどに変換され、丸めが発生しない。ブロック境界のフィルタ不連続は VAD / ASR 用途では許容する
- 言語は Processor 生成時に固定し、全リクエストで同じ言語を使う (自動検出・言語キャッシュ・プローブは行わない)
- 16 kHz PCM の保持: 0061 の契約どおり Processor が feed 済み 16 kHz PCM を保持し、`start_sample..end_sample` で slice する (`VadGate` は PCM を保持しない)。破棄判定には `VadGate::min_required_sample` を使い、submit 済みかつ VadGate が必要としない範囲を毎 feed 後に破棄する。16 kHz f32 は約 230 MB/時/track なので、無発話 track で保持が伸び続ける実装にはしないこと
- VadConfig のカスタマイズの口は本 issue では設けない (0063 で必要になったら追加)

### タイムスタンプ (TextFrame.start / end)

- `TextFrame.start / end` は Whisper 出力ではなく VAD 由来で埋める (PoC の Decoder は NO_TIMESTAMPS_TOKEN でデコードし、タイムスタンプを出力しない)
- 最初に受信した AudioFrame の `timestamp` (Duration) を `base_offset` として記録し、`start = base_offset + SpeechSegment::start_time()` とする (track 基準時刻への写像)。30 秒分割時は分割位置のサンプル数を加算する
- 入力フレーム間のギャップは詰めて連続 PCM とみなす (ギャップ補正はしない。乖離が問題になったら将来 issue で扱う)

### 順序保証・VAD・終端・エラーの方針

- 順序保証: 複数の入力 track (複数 Processor) 間での結果の順序保証はしない。単一 Processor 内は上記 FIFO publish のとおり start 順が保たれる
- VAD: Silero VAD あり前提とする。VAD なし (固定長分割等) のパスは本 issue では実装しない (0063 の `--vad off` の扱いは 0063 側で確定する)
- 終端: 入力 track の Eos 受信で、蓄積中の端数 PCM を resample → `VadGate::flush` で末尾区間を確定 → submit → pending の推論結果をすべて publish し切ってから出力 text track を終了する
- エラー: 次のいずれも該当 Processor をエラー終了させる (`VadGate` の作り直しはしない):
  - 入力 AudioFrame が I16Be 以外 (Opus / AAC 圧縮フレームが直接流れてきた構成ミス)
  - 蓄積途中で入力の sample_rate / channels が変化した
  - `resample_to_mono` の Err (`SUPPORTED_HZ` 外のサンプルレート)
  - `VadGate::feed` / `flush` の Err
  - 推論失敗 (Err の TranscriptResult、oneshot の RecvError)
- 区間スキップや backoff retry などの細かいケアは、将来必要になった場合に processor 共通の仕組みとして検討する (本 issue のスコープ外)

### MediaFrame::Text 出力フォーマット

- `TextFrame` (`src/text.rs`) の start / end は上記タイムスタンプ節の規則、text / language / no_speech_prob / avg_logprob は `TranscriptResult` から埋める
- no_speech ガード (PoC の閾値: `no_speech_prob > 0.6` かつ `avg_logprob < -1.0`) で text が空になった結果は publish しない (skip)

### テスト

- 単体テスト (モデル不要): 30 秒分割の純関数、i16 → f32 正規化、config.json パース (パース本体を `&str` 受けに分離して inline fixture で試験する)、タイムスタンプ写像、`VadGate::min_required_sample`。いずれも各ファイル内の `#[cfg(test)] mod tests` に置く (`min_required_sample` は private な `State` を直接構築する vad.rs 内テストの前例に倣う)
- integration テスト (`tests/test_ml_audio_whisper.rs`、実モデル): whisper-tiny + testdata 実音声で `WhisperPipeline` を実行し、text 非空 + 期待表記種別 (英語 fixture は英字、日本語 fixture は日本語文字) を含む + `no_speech_prob < 0.5` + `avg_logprob > -1.0` (実測して閾値調整可) + 指定言語一致 (ja / en) を assert。skip 動線は 0061 で確立した慣習に従う: `HISUI_ML_MODELS_DIR` (未設定時は `ml-models`) からモデルパスを解決し、ファイル不在なら skip、`HISUI_CI=1` なら panic。skip ヘルパーは `tests/test_ml_audio_silero_vad.rs` のものを複製する (テストバイナリ間で共有できない)。モデルディレクトリは `<HISUI_ML_MODELS_DIR>/whisper-tiny/` を渡す
- VAD 発話陽性 integration テスト (0061 からの委譲): 実音声を `VadGate` に流して非空の `SpeechSegment` が返ることを assert する。追加先は `tests/test_ml_audio_silero_vad.rs` (実推論テストの集約先。`tests/test_ml_audio_vad.rs` は実推論を経由しないテスト専用)
- in-process pipeline test (`tests/test_ml_audio_whisper.rs` 内): テスト用 source processor → `TranscriptionProcessor` → subscribe で `MediaFrame::Text` を受信できることを確認する。whisper-tiny / silero-vad の両モデルに上記 skip 動線を適用する。入力は testdata の s16le raw PCM をバイトスワップして `AudioFrame` (I16Be、16 kHz、mono) に組む。pipeline 組み立ての前例は `tests/decoder_tests.rs` を参照
- エラーパステスト: モデルディレクトリ不在・ファイル欠落で `TranscriptionService::new` (または `WhisperPipeline` ロード) が Err を返す
- PBT は追加しない (推論結果はモデル依存で PBT に不向き。純関数部分は単体テストで担保する)
- 実モデル使用 (モック・スタブは使わない)

### testdata 実音声

- 日本語 / 英語の短い発話 (2〜5 秒程度、書き起こし既知、CC0) を 16 kHz mono s16le raw PCM に変換して追加する:
  - `testdata/speech-ja-16k-mono-s16le.pcm`
  - `testdata/speech-en-16k-mono-s16le.pcm`
- 入手経路 (どちらでも可):
  - Mozilla Common Voice (クリップは CC0。要メールアドレス登録、小容量の Delta segment リリースを使う)
  - 自前録音した短文発話を CC0 宣言で追加する (登録不要で最も確実)
- raw PCM にする理由: hisui に WAV reader がまだ無い (0063 で追加予定) ため、テストは `std::fs::read` + i16 le → f32 変換で読む
- 出所 (クリップ ID または自前録音の旨)・ライセンス・変換手順 (ffmpeg コマンド) はテストファイル冒頭のコメントに記録する
- 文字起こし内容の厳密一致はモデル・浮動小数点の環境差で脆いため検証しない。表記種別 (英字 / 日本語文字) と最低分量のみを緩く検証する

### CI

- test-candle job に whisper-tiny のキャッシュ + ダウンロード step を追加する (silero-vad step と同型)。cache key は `whisper-tiny-7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395` (model.safetensors の SHA256 で代表。`scripts/download_ml_models.py` の expected_sha256 と手動同期する。config.json / tokenizer.json のみが更新された場合は key にサフィックスを足して無効化する)
- model.safetensors は約 150 MB。ダウンロードと実推論を含めて `timeout-minutes: 20` に収まるか実測し、不足なら調整する
- test-apple-toolbox / test-nvidia-video-codec には実モデルを積まない (両ジョブは `HISUI_ML_MODELS_DIR` / `HISUI_CI` 未設定のため integration テストは skip される。GPU 実推論はニーズが出たら別 issue で扱う)

### CHANGES.md

エントリは追加しない (内部実装のため)。

## 完了条件

- `src/ml/audio/` に `whisper.rs` / `decode.rs` / `multilingual.rs` / `melfilters.bytes` / `transcription_service.rs` / `transcription_processor.rs` が追加され、`src/ml/audio.rs` にモジュール宣言が追記されている
- `src/ml/audio/vad.rs` に `min_required_sample` が追加されている
- `TranscriptionProcessor` が `MediaFrame::Text` を publish できる (in-process pipeline test で確認)
- testdata の実音声 (ja / en、CC0) が追加され、whisper-tiny の integration テスト (`tests/test_ml_audio_whisper.rs`) と VAD 発話陽性 integration テスト (`tests/test_ml_audio_silero_vad.rs`) が green
- test-candle CI job (whisper-tiny ダウンロード step 追加済み) で `cargo clippy --features candle --all-targets -p hisui -- --deny warnings` と `cargo test --features candle -p hisui` が green
- 既定 feature ビルドに影響がない: `cargo check -p hisui` と `cargo test --workspace` が green
- `cargo fmt --all --check` が green

## 解決方法

PoC (PR #246、`origin/feature/try-candle`) の `whisper.rs` / `decode.rs` / `multilingual.rs` / `melfilters.bytes` を設計方針に合わせて移植・改修し、`TranscriptionService` / `TranscriptionProcessor` を新規実装する。
