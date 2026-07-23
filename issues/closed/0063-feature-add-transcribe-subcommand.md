# hisui -x transcribe 実験的サブコマンドを追加する

- Priority: Medium
- Created: 2026-06-24
- Completed: 2026-07-23
- Model: Opus 4.7
- Branch: feature/add-transcribe-subcommand
- Polished: 2026-07-22

## 目的

利用者が MP4 (音声のみの m4a を含む) の音声を文字起こしできる実験的サブコマンド `hisui -x transcribe <INPUT_FILE>` を提供する。 これは親 issue 0012 系列の利用者向け統合層であり、本系列のリリース対象。

## 優先度根拠

本系列 (0059 / 0060 / 0061 / 0062) の最終層で、ここまでの基盤を利用者から見える機能として完成させる。 一方で hisui 全体の中では **実験的機能** の位置づけ (`--experimental` フラグ必須) なので Medium。

## 現状

- hisui には文字起こし系のサブコマンドがない
- 0062 で MediaPipeline 上での文字起こし基盤 (`TranscriptionService` / `TranscriptionProcessor` / `MediaFrame::Text`) は develop に取り込み済みだが、外向きの CLI 露出はない
- `pipeline` サブコマンドおよび `--experimental(-x)` グローバルフラグは develop 中に一時的に追加された後にすべて削除済み (`src/main.rs` に痕跡なし)。 本 issue で `--experimental(-x)` を再導入し、`transcribe` サブコマンドを乗せる (参照コミットは「## 解決方法」節に記載)
- 現サブコマンド: inspect / list-codecs / compose / vmaf / tune / server

## 設計方針

### `#[cfg(feature = "candle")]` gate

- `src/subcommand_transcribe.rs` と `src/lib.rs` のモジュール宣言、および `src/main.rs` の dispatch チェイン内の transcribe 呼び出しは `#[cfg(feature = "candle")]` で gate する
- **`--experimental(-x)` グローバルフラグ本体は gate しない** (candle 無効ビルドでも `--help` の表示が変わらないほうが利用者が混乱しない)
- candle feature 無効ビルド時は `-x transcribe` を渡しても transcribe cmd 自体が subcommand 一覧に存在しないため silent に無視される (help にも出ない、`transcribe` cmd 名で unknown 扱い)。 warn ログは出さない (build 時 feature を意識するのは開発者側の責務、実行時に混乱を招くログは避ける)

### グローバルフラグ `--experimental(-x)` の再導入

- `src/main.rs` の `--emit-exit-metrics` (`src/main.rs:32-39`) の直後に `let experimental: bool = noargs::flag("experimental").short('x').take(&mut args).is_present();` を追加する
- main.rs で取得した `experimental: bool` を **subcommand_transcribe::try_run に第 3 引数として渡す** (noargs のフラグは `take` で消費されるため subcommand 側で再取得できない)
- `subcommand_transcribe::try_run` は `cmd("transcribe")` が present の場合のみ `experimental` フラグを検査する。 未指定なら **標準エラーに日本語で「実験的機能です。 `--experimental` (`-x`) フラグを付けて起動してください」を書き出して `noargs::Error::other(args, ...)` で Err を返す** (main の `?` で伝播、非ゼロ exit code。 user-facing メッセージは既存 CLI の日本語慣習に沿う)
- 他 subcommand の try_run signature (`experimental` 不要) は変更しない

### サブコマンド `hisui -x transcribe <INPUT_FILE>`

`src/subcommand_transcribe.rs` を新規追加 (`#[cfg(feature = "candle")]`)。 CLI 定義:

- 位置引数: `INPUT_FILE` (MP4、`.mp4` / `.m4a`)。 `-` (stdin) は非対応 (MP4 の seek 前提のため)
- `--model-dir <path>` (必須): Whisper モデルディレクトリ (例: `./ml-models/whisper-tiny/`)。 `config.json` / `tokenizer.json` / `model.safetensors` を含む
  - 将来別 ML モデル系列 (kotoba-whisper / YOLO 系 [0064]) との共存で命名が曖昧化する可能性があるが、初版は既存 `subcommand_inspect` 慣習に沿った `--model-dir` を採用。 リネーム (`--whisper-model-dir` 等) は共存が発生する時点で別 issue で扱う
- `--silero-vad-model <path>` (必須): Silero VAD ONNX モデル **ファイル** (例: `./ml-models/silero-vad/onnx/model.onnx`)。 環境変数 `HISUI_SILERO_VAD_MODEL_PATH` でも指定可
  - **ファイル指定 (ディレクトリではない)**: `SileroVadModel::load` は単一 `.onnx` ファイルを取り、`is_file()` で検証する (`src/ml/audio/silero_vad.rs:59-73`)。 Whisper の `--model-dir` (ディレクトリ = 3 ファイル収める場所) とは意味論が異なる
- `--language <code>` (必須): Whisper 言語指定 (`ja` / `en` 等)
  - CLI 層では文字列で受け、subcommand 内で `LanguageCode::new(code)` で `LanguageCode` 型にラップして `TranscriptionProcessor::new` に渡す (`src/text.rs:11-14`)
  - 妥当性検証は Whisper 側の初回推論に委ねる (tokenizer 未収録なら Processor が Err 終了)。 本 issue では CLI 層で追加検証しない
- `--transcribe-threads <N>` (任意): 1 推論あたりの candle rayon スレッド数
  - noargs 上は `Option<NonZeroUsize>` として受ける (`.default(...)` は静的文字列しか渡せないので使わない)。 環境変数 `HISUI_TRANSCRIBE_THREADS` も同じ扱いで受ける (`.env(...)`)
  - 未指定なら `RAYON_NUM_THREADS` env の値を尊重、それも無ければ candle / rayon の既定 (論理コア数)
  - CLI or env で値が指定された場合は、`subcommand_transcribe::try_run` の **先頭** (tokio runtime 構築より前、= candle が初回呼ばれる前) で `unsafe { std::env::set_var("RAYON_NUM_THREADS", n.to_string()) }` を叩いて `RAYON_NUM_THREADS` を上書きする (Rust 2024 edition 以降 `env::set_var` は unsafe fn)
    - logger は既に main.rs で init 済み (`src/main.rs:22-30`) なので「logger 初期化より前」の順序は達成不能。 意味を持つのは「rayon global pool 初回構築より前」
    - hisui の unsafe ブロックの多くには SAFETY コメントが付いているので (`src/tune/storage.rs`, `src/subcommand_server.rs`, `src/ml/audio/whisper/decode.rs` 等)、それに倣う。 SAFETY 本文は「try_run 先頭 = rayon global pool 初期化前、他スレッドは env を触らない (main.rs で logger init 済みだが env アクセスなし)」で足りる
- `--fdk-aac <path>` (任意、`feature = "fdk-aac"` 有効時のみ): libfdk-aac 共有ライブラリのパス。 `subcommand_inspect` と同じ pattern (環境変数 `HISUI_FDK_AAC_PATH` にも対応、`src/subcommand_inspect.rs:49-55` 参照)。 AAC in MP4 入力を Linux で扱うのに必須。 cfg attribute の位置も subcommand_inspect と揃える
- `--emit-exit-metrics` (共通フラグ、transcribe 分岐時は **main.rs 側で silent 抑止**):
  - `--emit-exit-metrics` は `src/main.rs:32-39` で main が消費する。 subcommand 側からは値を参照できない
  - main.rs 側で以下のように書き換える (dispatch チェインから transcribe の返り値を outer scope の `transcribe_matched` 変数に受ける):

    ```rust
    let mut transcribe_matched = false;
    let matched = hisui::subcommand_inspect::try_run(&mut args, stats.clone())?
        || hisui::subcommand_list_codecs::try_run(&mut args)?
        || {
            #[cfg(feature = "candle")]
            {
                transcribe_matched = hisui::subcommand_transcribe::try_run(&mut args, stats.clone(), experimental)?;
                transcribe_matched
            }
            #[cfg(not(feature = "candle"))]
            { false }
        }
        || ...
    ```

    そのうえで `emit_exit_metrics_to_stdout` の呼び出し部を以下のように書き換える:

    ```rust
    if emit_exit_metrics && matched && !args.metadata().help_mode {
        if transcribe_matched {
            tracing::warn!("--emit-exit-metrics is ignored for transcribe because JSON LINE output shares stdout");
        } else {
            hisui::metrics::emit_exit_metrics_to_stdout(&stats);
        }
    }
    ```

  - subcommand_transcribe 側は `--emit-exit-metrics` に一切触れない

VAD は Silero 固定。 `--vad <kind>` オプションおよび `--vad off` (固定長分割) は本 issue では実装しない (将来 issue で扱う)。

### 入力ファイル形式

- **MP4 のみ** (`.mp4` / `.m4a`)。 hisui 既存の `crate::mp4::sample_reader::Mp4SampleReader` (fMP4 対応) + `crate::decoder::AudioDecoder` (Opus / AAC → I16Be) を流用する
- **WAV / WebM / Opus 単体 / MP3 等は本 issue のスコープ外** (将来別 issue で追加する)
- 音声コーデック:
  - **Opus in MP4**: hisui の Opus decoder は features 非依存で常に有効
  - **AAC in MP4**: macOS では AudioToolbox で decode 可能。 Linux では `--features fdk-aac` build + `--fdk-aac <path>` 指定が必要 (`src/decoder.rs:207-224` の実装制約)
- 非対応コーデック (Vorbis / MP3 in MP4 等): `Mp4SampleReader` が Err で終了 (「No supported audio track found in the file」)。 発生タイミングは `Mp4SampleReader::run()` 内 (spawn 済み processor 内、pipeline 起動後) で、user-facing には「startup 後に processor failed で終了 (非ゼロ exit)」の形になる

### MP4 に音声トラックが複数ある場合の扱い

- **最初に見つかった対応コーデックの音声トラック 1 本のみを transcribe** する (`Mp4SampleReader::select_supported_tracks` の既存挙動を踏襲、`src/mp4/sample_reader.rs`)
- **複数存在しても warn は出さない (silent)**: 既存 `select_supported_tracks` は非対応コーデックのみ warn を出し、対応コーデックの 2 本目以降を skip する経路には warn 出力が無い。 追加で warn を出すには `select_supported_tracks` を拡張することになり、他 subcommand (inspect) にも波及する。 本 issue のスコープからは外し、silent 採用 (既存挙動そのまま) とする
- 複数トラックの並列 transcribe や `--audio-track <index>` 選択、複数検出時の warn 追加は将来 issue で扱う (0088 の MP4 字幕出力とも同期して決める)

### 内部実装の流れ

```
subcommand_transcribe::try_run(args, stats, experimental) -> noargs::Result<bool>
  ├ cmd("transcribe") が not present なら Ok(false)
  ├ !experimental なら Err (main で ? 伝播、非ゼロ exit)
  ├ (--transcribe-threads / HISUI_TRANSCRIBE_THREADS 指定時) unsafe { std::env::set_var("RAYON_NUM_THREADS", n) }
  │    (SAFETY: try_run 先頭 = rayon global pool 初期化前、他スレッドは env を触らない)
  ├ CLI 引数のパース (--model-dir / --silero-vad-model / --language / --fdk-aac 等)
  ├ tokio runtime を組む (subcommand_inspect と同じく new_multi_thread、worker_threads=1)
  └ runtime 内で:
        ├ device = hisui::ml::device::select_device()  (1 回だけ呼ぶ)
        ├ let service = Arc::new(TranscriptionService::new(model_dir, device.clone())?);  (Err は ? で bubble up)
        ├ let silero = SileroVadModel::load(silero_vad_model_path, device)?;  (戻り値は既に Arc<Self>)
        ├ MediaPipeline を組み立て
        ├ Mp4SampleReader (encoded audio publish → track_id = AUDIO_ENCODED_TRACK_ID)
        ├ AudioDecoder (audio_encoded subscribe → publish audio_decoded, I16Be)
        │    (AudioDecoder::new に fresh な crate::stats::Stats::new() を渡す = subcommand_inspect 踏襲)
        ├ TranscriptionProcessor::new(service, silero, LanguageCode::new(code), ...) (0062 実装済み)
        ├ text sink processor (text subscribe → 標準出力に JSON LINE)
        ├ pipeline_handle.trigger_start().await
        ├ let processor_failed = pipeline.run().await
        └ if processor_failed { Err } else { Ok(true) }
```

- **`try_run` の signature**: `pub fn try_run(args: &mut noargs::RawArgs, stats: crate::stats::Stats, experimental: bool) -> noargs::Result<bool>`
  - 既存 subcommand の signature (`src/subcommand_inspect.rs:26`, `src/subcommand_server.rs:5` 等) と揃えるため `noargs::Result<bool>` を返す (main.rs:46-51 の `||` chain と組み合わせるため)
  - `Ok(true)` = transcribe cmd を実行した、`Ok(false)` = transcribe cmd に該当しなかった、`Err(...)` = 実行時エラー
- **TranscriptionService の Arc ラップ**: `TranscriptionService::new` は `crate::Result<TranscriptionService>` を返す (`src/ml/audio/transcription_service.rs:36-42`) が、`TranscriptionProcessor::new` は `Arc<TranscriptionService>` を要求する (`src/ml/audio/transcription_processor.rs:55-56`)。 `Arc::new(TranscriptionService::new(...)?)` で明示的にラップする。 `SileroVadModel::load` は `crate::Result<Arc<Self>>` を返す (`src/ml/audio/silero_vad.rs:59`) のでそのまま渡す
- **TrackId 定数**: subcommand_transcribe.rs 内 local const で `AUDIO_ENCODED_TRACK_ID` / `AUDIO_DECODED_TRACK_ID` / `TEXT_TRACK_ID` を定義 (`src/subcommand_inspect.rs:21-24` と同じスタイル、crate 全体で共通化はしない)
- **ProcessorId / ProcessorMetadata 命名**: `mp4_file_reader` / `audio_decoder` / `transcription` / `text_stdout_sink` (`transcription` は 0062 の integration test で採用済み)
- **登録順**: `mp4_file_reader` → `audio_decoder` → `transcription` → `text_stdout_sink`。 全 processor 登録後に `MediaPipelineHandle::trigger_start` を呼ぶ
- **`TranscriptionService::new` は tokio runtime 内で呼ぶ** (`spawn_blocking` を使うため。 0062 の設計方針)
- **device の選択**: `hisui::ml::device::select_device()` を **1 回だけ呼び**、`device.clone()` を `TranscriptionService::new` と `SileroVadModel::load` に渡す (2 回呼ぶと GPU 初期化を 2 回試行して遅い / info ログが二重に出る。 GPU 初期化失敗時は warn ログも二重)
- **`pipeline.run()` の返り値**: `bool` (processor_failed)。 true の場合は `crate::Error::new("transcribe failed: one or more processors terminated abnormally")` で Err にして非ゼロ exit にする (`src/subcommand_inspect.rs:125-133` の pattern)

### 出力フォーマット: JSON LINE

`stdout` に 1 行 1 セグメントの JSON を書き出す (`\n` 区切り)。

```jsonl
{"start": 0.5, "end": 2.3, "text": "こんにちは", "language": "ja", "no_speech_prob": 0.02, "avg_logprob": -0.15}
```

- **シリアライザは `nojson` を使う** (hisui の JSON 出力は全面的に `nojson::DisplayJson`、依存に `serde_json` は無い)
- **`impl nojson::DisplayJson for TextFrame`** と **`impl nojson::DisplayJson for LanguageCode`** を `src/text.rs` に追加する
  - `TextFrame` は常に top-level (JSON LINE) で 1 レコード = 1 行として render するため、indent 制御は impl 内では行わず caller 側に任せる (既存の `AudioSampleInfo::fmt` の「impl 内で `set_indent_size(0)` → 末尾で 2 に復元」pattern は nested JSON 用途で、本 issue の top-level 用途とは違うので採らない)
  - `LanguageCode` の impl は `f.string(self.get())` を呼ぶだけ
  - `TextFrame` の impl 内で `Option` フィールドは **`if let Some(v) = ...` で分岐して `f.member(...)` を呼ぶ** (キーごと省略のため)。 `nojson::Option<T>: DisplayJson` の既定挙動は `None` を `"null"` として書き出す (`nojson-0.3.12/src/display_json.rs:128-136`) ので、そのまま `f.member("language", &self.language)` に渡すと `null` が出てしまう。 既存 `AudioSampleInfo::fmt` (`src/subcommand_inspect.rs:269-271, 305-313`) と同じ pattern:

    ```rust
    impl nojson::DisplayJson for TextFrame {
        fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
            f.object(|f| {
                f.member("start", self.start.as_secs_f64())?;
                f.member("end", self.end.as_secs_f64())?;
                f.member("text", &self.text)?;
                if let Some(v) = &self.language { f.member("language", v)?; }
                if let Some(v) = self.no_speech_prob { f.member("no_speech_prob", v)?; }
                if let Some(v) = self.avg_logprob { f.member("avg_logprob", v)?; }
                Ok(())
            })
        }
    }
    ```

- **indent 制御は subcommand_transcribe の text sink 側で行う** (`nojson::JsonFormatter` の `indent_size` の既定値は 0 だが意図を示すため明示):

  ```rust
  let json = nojson::json(|f| f.value(&text_frame));
  writeln!(stdout_lock, "{json}")?;
  stdout_lock.flush()?;
  ```

- **フィールドスキーマ**:
  - `start` / `end`: `Duration` を秒 (float、`Duration::as_secs_f64()` を使う)
  - `text`: `TextFrame.text` そのまま (nojson は内部で string escape する)
  - `language`: `Option<LanguageCode>` — `Some` なら文字列、`None` ならキーごと省略
  - `no_speech_prob`: `Option<f32>` — `Some` なら数値、`None` ならキーごと省略 (Whisper 経路は常に Some)
  - `avg_logprob`: `Option<f32>` — 同上
- **バッファリング**: `writeln!` + `flush()` ごとに `std::io::stdout().lock()` を **取得 → drop** する (StdoutLock は `!Send` なので tokio task の await 越しに保持できない。 subscribe の `input_rx.recv().await` を跨がないよう、1 行書き出しごとに lock/drop する)
- **BrokenPipe (stdout の pipe reader が閉じた) 時の扱い**: text sink processor は publish しない subscriber。 `writeln!` / `flush()` が返す `std::io::Error` の `kind() == BrokenPipe` を検知したら、内部で warn ログを 1 度出して text sink processor 自身は Err で早期終了する
  - text sink が Err で終了すると `MediaPipeline` は processor_failed を検知するため、`pipeline.run() -> true` を返し subcommand が非ゼロ exit する。 これで pipeline 全体 (Mp4SampleReader / AudioDecoder / TranscriptionProcessor) も巻き添えで停止する
  - 補足: `TrackPublisher::send` は subscribers が 0 個になっても pipeline が生きている限り true を返す (`src/media_pipeline.rs`) ので、text sink が Ok(()) で早期終了する (subscribe drop 待ち) 方式は使えない。 必ず Err で早期終了させる
- MP4 字幕トラック / SRT / WebVTT ファイル / データチャネル等の別出力経路は本 issue の対象外:
  - MP4 字幕トラック (WVTT): **0088** で扱う
  - SRT / WebVTT ファイル / データチャネル: **0014** 系列で別 issue

### エラー動線

user-facing の Err メッセージは既存 CLI の日本語慣習に沿う。 `tracing::warn!` / `tracing::error!` などの内部ログは英語 (CLAUDE.md 規約「ログメッセージは全て英語」)。

- `--experimental` 無しで `transcribe` を呼ぶ: 標準エラーに日本語メッセージ → `noargs::Error::other(args, ...)` で Err → 非ゼロ exit code
- `--model-dir` 不在 / 3 ファイル欠落: `WhisperPipeline::load` が Err (「missing config.json in model directory ...」等、0062 実装済み) → subcommand が伝播 (CLI 引数パース直後の TranscriptionService::new で発火 = pipeline 起動前)
- `--language` に tokenizer 未収録コード: Whisper 初回推論で Processor が Err → pipeline 終了 (`pipeline.run() -> true` → Err、pipeline 起動後)
- `--silero-vad-model` に不在パス: `SileroVadModel::load` が Err (「silero VAD model file not found: ...」、0062 実装済み) → pipeline 起動前
- 入力 MP4 に対応音声トラックなし / video-only MP4: `Mp4SampleReader::run()` 内 (spawn 済み processor 内) で Err → pipeline 起動後
- 入力 MP4 に音声トラックはあるが AAC で `--fdk-aac` 未指定 + 非 macOS: `AudioDecoder::new` が Err → pipeline 起動前
- 大サイズ MP4 (数時間): 動くが CI では短尺 fixture でのみ検証。 メモリ / 実行時間の最適化は本 issue のスコープ外

### ドキュメント: `docs/command_transcribe.md`

章立て:

1. 概要
2. 前提 (`candle` feature 有効ビルド、AAC 入力は `--fdk-aac` or macOS)。 build 手順は本節に完結して書く (`docs/build.md` の変更は不要)
3. モデル取得手順 (`uv run scripts/download_ml_models.py --dest ml-models whisper-tiny silero-vad`)
4. 使い方 (最短の実行例)
5. CLI オプション一覧
6. 出力フォーマット (JSON LINE の例と各フィールド解説)
7. 制約 (対応入力 MP4 のみ、実験的機能、音声トラック複数時の挙動)
8. `--verbose` を併用すると選択された ML device (cuda / metal / cpu) がログに出る旨
9. 関連ドキュメント (`docs/internals/transcription.md` の設計背景)

### e2e テスト

- `e2e-tests/transcribe/test_*.py` を新設し、`e2e-tests/pyproject.toml` の `testpaths` に `"transcribe"` を追加する
- pytest fixture (`binary_path` / `hisui_server.build_hisui_command`) を obsws と共有 (実運用では `cargo run --features candle,fdk-aac` を叩く `HISUI_E2E_CARGO_RUN_ARGS` 経由の build 実行、`binary_path` 引数は現行 `build_hisui_command` で消費されていない = `_ = binary_path`)。 subprocess で `hisui -x transcribe --model-dir ... --language ... <fixture.mp4>` を起動し (silero VAD path は env `HISUI_SILERO_VAD_MODEL_PATH` の transparent fallback を利用)、stdout の JSON LINE を 1 行ごとに `json.loads` する
- **`--model-dir` の組み立てルール**: pytest fixture は環境変数 `HISUI_ML_MODELS_DIR` を必須として読み、`--model-dir ${HISUI_ML_MODELS_DIR}/whisper-tiny` を組み立てる (CI と同じ規約)
- **per-test timeout**: `e2e-tests/pyproject.toml` の既定 `timeout = 30` では whisper-tiny CPU 推論に不足する見込みなので、transcribe テストは `@pytest.mark.timeout(120)` を各テスト関数に付ける (実測値は CI 実行後に調整する)
- 検証項目:
  - 各行に必須キー (`start` / `end` / `text` / `language`、および `no_speech_prob` / `avg_logprob`) が存在
  - `start <= end` かつ時刻が単調増加
  - `language` が指定値と一致
  - `text` 非空 + keyword substring (0062 の integration test と同じ緩い部分一致)
  - `no_speech_prob` / `avg_logprob` の範囲 (0062 と同じ緩い閾値: `no_speech_prob < 0.5`, `avg_logprob > -1.5`)
- integration test (`tests/test_subcommand_transcribe.rs` 等) は追加しない (subcommand は薄いラッパで、pipeline レベルは 0062 の `tests/test_ml_audio_whisper.rs` の in-process 検証で担保。 subcommand の e2e は本節の pytest に集約)
- **ローカル実行時の注意** (docs 側にも書く): `HISUI_E2E_CARGO_RUN_ARGS` 未設定のローカル環境では transcribe 系テストが全失敗する。 開発者は環境変数を設定するか docs/command_transcribe.md 記載の実行例に従う

### fixture

- `testdata/e2e/transcribe/speech-en.mp4` / `speech-ja.mp4` を配置する (`testdata/e2e/` は既存)
- 元素材は 0062 と同じ Common Voice クリップ (`common_voice_en_100540` / `common_voice_ja_19486650`)。 0062 の integration test と keyword を揃えられる
- 変換方針: **Opus in MP4 を採用** (Linux CI で `--fdk-aac` の追加なしに decode 可能)
- ライセンス表記 (Common Voice のクリップ ライセンスは CC0-1.0) と ffmpeg 変換手順は `testdata/README.md` の既存 Common Voice セクションに **派生形式** サブブロックとして追記する (issue 本文には要点だけ、詳細は README.md に集約)
- AAC in MP4 版は macOS ローカル用途で必要に応じて追加可能。 本 issue の CI では Opus 版のみ

### CI

- `test-candle` job への whisper-tiny / silero-vad モデル追加と integration テストの実推論組込は 0062 で完了済み
- 本 issue で `.github/workflows/e2e-test.yml` に以下を追加する:
  - `HISUI_E2E_CARGO_RUN_ARGS: --release --features candle,fdk-aac` (candle feature 追加、既存の fdk-aac は AAC fixture を将来追加する場合に備えて残す)
  - `apt-get install` に **`protobuf-compiler` を追加する** (candle feature は `candle-onnx` を有効化し、その build.rs は `protoc` を要求する。 `test-candle` job および CHANGES.md `## develop` の「candle-onnx のビルドに protoc が必要」記述と整合)
  - `test-candle` と同型の Silero / Whisper モデルキャッシュ + ダウンロード step
  - `HISUI_ML_MODELS_DIR: ${{ github.workspace }}/ml-models` を env に設定
  - `HISUI_SILERO_VAD_MODEL_PATH: ${{ github.workspace }}/ml-models/silero-vad/onnx/model.onnx` を env に設定 (subprocess.Popen が env を継承し hisui に伝わる)
  - `HISUI_CI: "1"` (integration test 側の silent skip を無効化する慣習)
- e2e job のタイムアウトは **30 分** (candle 系依存の cold build 時間 + whisper-tiny CPU 推論 + Opus decode + pytest overhead を見込む。 現行 e2e-test.yml の 20 分は fdk-aac 単独 build 前提)

### CHANGES.md

`## develop` の既存エントリを以下のように再編する (memory `changelog-unreleased-removal` に基づき、既存関連エントリは表現調整または削除する):

- **削除**: `[CHANGE] 実験的な pipeline サブコマンドを削除する` (pipeline サブコマンド追加自体は release セクションのどこにも `[ADD]` エントリが無く undocumented だったため、削除エントリも不要)
- **削除**: `[CHANGE] コマンドライン引数に --experimental(-x) フラグを追加して pipeline サブコマンドはこのフラグ指定時にのみ有効になるようにする` (対象の pipeline サブコマンドが消えており本文が矛盾。 `--experimental(-x)` フラグ追加自体は develop 期間内で完結)
- **追加**:

  ```
  - [ADD] `hisui -x transcribe <input.mp4>` 実験的サブコマンドを追加する
    - 実験的機能フラグ `--experimental` (`-x`) を新設し、`transcribe` はこのフラグ指定時のみ有効
    - Whisper モデル (`--model-dir` 必須) と Silero VAD (`--silero-vad-model` 必須) と言語指定 (`--language` 必須) で MP4 (`.mp4` / `.m4a`) の音声を文字起こしし、標準出力に JSON LINE で出力する
    - 対応入力は MP4 のみ (WAV / WebM / Opus 単体等は非対応)。 AAC in MP4 は Linux では `--features fdk-aac` build + `--fdk-aac` 指定が必要
    - モデル取得は `scripts/download_ml_models.py --dest ml-models whisper-tiny silero-vad`
    - @sile
  ```

## 完了条件

- `hisui -x transcribe <input.mp4>` (Opus in MP4 で動作、AAC in MP4 は macOS または `--fdk-aac` 併用で動作) が動作する
- `--experimental` (`-x`) フラグ無しで `transcribe` を呼ぶと標準エラーに実験機能である旨のメッセージを書き、非ゼロ exit code で終了する
- 結果が JSON LINE で標準出力に出力される (フィールドスキーマは上記の通り)
- `docs/command_transcribe.md` が整備されている
- `e2e-tests/transcribe/` の test が green (e2e-test job で実推論、Opus in MP4 fixture、`@pytest.mark.timeout(120)`)
- `cargo test --features candle -p hisui` が test-candle CI job で green
- CHANGES.md の既存 `pipeline` / `--experimental` 関連 2 エントリを削除し、新規 `[ADD] hisui -x transcribe` エントリを追記する
- `cargo fmt --all -- --check` / `cargo clippy --features candle --all-targets -p hisui -- --deny warnings` が green
- `#[cfg(feature = "candle")]` を外した既定 feature ビルド (`cargo check -p hisui` / `cargo test --workspace`) に影響がない

## 解決方法

0062 で実装済みの `TranscriptionService` / `TranscriptionProcessor` / `MediaFrame::Text` の基盤を組み合わせて `subcommand_transcribe` を実装する。 骨格は `src/subcommand_inspect.rs` (`try_run` から `setup_pipeline` まで、途中 `run` / `run_internal` を挟む構成) を参考にする (同じく `Mp4SampleReader` + `AudioDecoder` を組み合わせる)。

参照実装:

- 既存 subcommand の書き方: `src/subcommand_inspect.rs`
- MP4 デマルチプレクサ: `src/mp4/sample_reader.rs` の `select_supported_tracks` (音声トラック単数選別の既存挙動)
- 音声デコーダ: `src/decoder.rs` の `AudioDecoder` (AAC 分岐は `feature = "fdk-aac"` / macOS)
- 0062 の TranscriptionProcessor pipeline パターン: `tests/test_ml_audio_whisper.rs` の `transcription_processor_publishes_text_frames` (in-process pipeline test で `run_audio_source` / `collect_text_frames`)
- 内部設計ドキュメント: `docs/internals/transcription.md` / `docs/internals/ml_models.md` (0062 で追加済み)

`--experimental(-x)` フラグの再導入は過去のコミット `f1a98f35 feat: 実験的機能フラグを追加し pipeline コマンドを制御する` を参考にする (フラグ追加時のコード。 本 issue の新設計では「cmd take → try_run 内で experimental を検査」の form で、当時の「experimental で cmd take を gate」form とは異なる)。

成果物一覧:

- `src/subcommand_transcribe.rs` (新規、`#[cfg(feature = "candle")]`)
- `src/main.rs` (`--experimental(-x)` グローバルフラグ再導入 + transcribe dispatch + `transcribe_matched` に基づく `--emit-exit-metrics` silent 抑止 + warn ログ)
- `src/lib.rs` (`#[cfg(feature = "candle")] pub mod subcommand_transcribe;`)
- `src/text.rs` (`impl nojson::DisplayJson for TextFrame` と `impl nojson::DisplayJson for LanguageCode` を追加)
- `docs/command_transcribe.md` (新規)
- `testdata/e2e/transcribe/speech-en.mp4` / `speech-ja.mp4` (Opus in MP4、CC0)
- `testdata/README.md` (Common Voice セクションに Opus in MP4 派生形式サブブロックを追記)
- `e2e-tests/transcribe/test_*.py` (新規、`@pytest.mark.timeout(120)`) + `e2e-tests/pyproject.toml` の `testpaths` 追加
- `.github/workflows/e2e-test.yml` (candle feature 追加 + `protobuf-compiler` 追加 + model キャッシュ / ダウンロード step + `HISUI_ML_MODELS_DIR` + `HISUI_SILERO_VAD_MODEL_PATH` + `HISUI_CI` env、timeout 30 分)
- `CHANGES.md` (既存 2 エントリ削除 + `[ADD] hisui -x transcribe` 新設)

### 実装完了時点の追加・変更差分

polish 時点の設計から、実装 / レビュー対応の過程で以下を追加・変更した:

- JSON LINE 出力の各行に `"type":"transcript"` フィールドを付与し、`--emit-exit-metrics` の `"type":"metrics"` 行と JSON LINE stream 上で振り分け可能にした (併用時の silent 抑止は撤去)
- `TextFrame` / `WhisperTranscript` から `language` フィールドを削除した (`--language` 必須で動的推論がないため出力の language は指定値と同一になり冗長)
- `--model-dir` / `--language` にも env fallback (`HISUI_WHISPER_MODEL_DIR` / `HISUI_WHISPER_LANGUAGE`) を追加した (既存の `HISUI_SILERO_VAD_MODEL_PATH` / `HISUI_TRANSCRIBE_THREADS` と揃える)
- `setup_pipeline` の Err を握り潰していた経路を修正し、shutdown_pipeline helper (5 秒 timeout) で pipeline を安全に停止するよう変更した (setup 中の Err で pipeline が hang する経路を解消)
- `Mp4SampleReader` / `Mp4FileReader` の CTS オフセットチェックを track_kind 判定内に移動した (対象外 track の CTS で pipeline 全体を落とさないようにする)
- tokio 非同期 worker のスタックサイズを 32 MiB に拡張した (Whisper 推論の深いネストが 2 MiB / 8 MiB のスタックで overflow する実測を反映)
- `--experimental` 未指定エラーメッセージを英語に統一 (`transcribe subcommand requires --experimental (-x) flag`)
- e2e テスト補強 (`subprocess.run(timeout=110)` / 半角カナ・CJK 拡張 A / `start >= 0`)、`TextFrame::DisplayJson` の Option ミックス / キー順序テストを追加
