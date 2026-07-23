# `hisui -x transcribe` コマンド (実験的機能)

`hisui -x transcribe` コマンドは、MP4 を入力に取り、
Whisper で音声を文字起こしして標準出力に JSON LINE (1 行 1 セグメント) で
出力する **実験的サブコマンド** です。 `--experimental` (`-x`) と組み合わせて起動します。

## 前提

- **`candle` feature を有効化してビルドする**必要があります (`cargo build --release --features candle`)
  - candle feature 無効ビルドでは transcribe サブコマンドは存在しません
  - candle-onnx のビルドには `protoc` (Ubuntu の `protobuf-compiler` 等) が必要です
- AAC を扱う場合の要件は [`docs/build.md`](build.md) を参照してください

## モデル取得

Whisper モデル (下記は whisper-tiny の例) と Silero VAD の ONNX モデルをダウンロードします。

```console
$ uv run scripts/download_ml_models.py --dest ml-models/ whisper-tiny silero-vad
```

配置先:

- Whisper モデルディレクトリ: `ml-models/whisper-tiny/` (`config.json` / `tokenizer.json` / `model.safetensors`)
- Silero VAD モデルファイル: `ml-models/silero-vad/onnx/model.onnx`

## 使用方法

```console
$ hisui -x transcribe -h
MP4 音声を Whisper で文字起こしします (実験的機能)

Usage: hisui ... transcribe --model-dir <PATH> --silero-vad-model <PATH> --language <CODE> [OPTIONS] INPUT_FILE

Example:
  $ hisui transcribe --model-dir ./ml-models/whisper-tiny --silero-vad-model ./ml-models/silero-vad/onnx/model.onnx --language ja /path/to/speech.mp4

Arguments:
  INPUT_FILE 文字起こし対象の MP4 ファイル (.mp4 / .m4a)

Options:
  -h, --help                    このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version                 バージョン番号を表示します
      --verbose                 警告未満のログメッセージも出力します
      --emit-exit-metrics       プロセス終了時に内部メトリクスを JSON Lines 形式で標準出力へ 1 行出力します [env: HISUI_EMIT_EXIT_METRICS]
  -x, --experimental            実験的サブコマンドの有効化フラグです
      --model-dir <PATH>        Whisper モデルディレクトリ (config.json / tokenizer.json / model.safetensors を含む) [env: HISUI_WHISPER_MODEL_DIR]
      --silero-vad-model <PATH> Silero VAD の ONNX モデルファイル (silero_vad.onnx) [env: HISUI_SILERO_VAD_MODEL_PATH]
      --language <CODE>         Whisper 言語指定 (ISO 639-1、`ja` / `en` 等) [env: HISUI_WHISPER_LANGUAGE]
      --transcribe-threads <N>  1 推論あたりの並列スレッド数を上書きします [env: HISUI_TRANSCRIBE_THREADS]
```

`--features fdk-aac` を有効にしてビルドした場合は、末尾に以下のオプションが追加されます。

```
      --fdk-aac <PATH>          FDK-AAC の共有ライブラリのパス [env: HISUI_FDK_AAC_PATH]
```

`--experimental` (`-x`) が指定されていない状態で `transcribe` を呼ぶと、非ゼロの終了コードを返します。

## 実行例

英語音声ファイルを文字起こしします。

```console
$ hisui -x transcribe \
    --model-dir ./ml-models/whisper-tiny \
    --silero-vad-model ./ml-models/silero-vad/onnx/model.onnx \
    --language en \
    ./sample.mp4
{"type":"transcript","start":0.96,"end":2.272,"text":"Hello, world.","no_speech_prob":0.05,"avg_logprob":-0.3}
```

選択された ML デバイス (cuda / metal / cpu) を確認する場合は `--verbose` を併用します
(標準エラーに INFO ログが出力されます)。

```console
$ hisui --verbose -x transcribe ...
```

## 出力フォーマット (JSON LINE)

1 セグメント = 1 行の JSON オブジェクトを標準出力に流します。 各行の末尾は `\n` です。

| フィールド        | 型           | 必須 | 意味                                                                                       |
|--------------------|---------------|------|--------------------------------------------------------------------------------------------|
| `type`             | string        | 必須 | 常に `"transcript"` (`--emit-exit-metrics` の `"metrics"` 行と区別する)                    |
| `start`            | number (秒)   | 必須 | セグメント開始時刻 (float、`Duration` を秒に変換)                                          |
| `end`              | number (秒)   | 必須 | セグメント終了時刻                                                                         |
| `text`             | string        | 必須 | 文字起こしされたテキスト                                                                   |
| `no_speech_prob`   | number        | 任意 | 発話がない確率 (0.0 - 1.0、Whisper 由来の幻覚指標)                                         |
| `avg_logprob`      | number        | 任意 | 平均 log probability (信頼度目安、Whisper 由来)                                            |

`no_speech_prob > 0.6` かつ `avg_logprob < -1.0` のセグメント、および空テキストのセグメントは出力しません。

`--emit-exit-metrics` を併用した場合、上記の `"type":"transcript"` 行に続いて末尾に `{"type":"metrics", ...}` の 1 行が追加で出力されます。

## 制約

- **音声トラックが複数含まれる MP4 では最初に見つかった対応コーデックのトラックのみ** を文字起こしします (対応コーデックの 2 つ目以降は警告ログなしで無視、非対応コーデックのトラックは警告ログ付きでスキップ)
- **出力は JSON LINE のみ**。 MP4 字幕トラック (WVTT 等) としての出力は非対応
- 実験的機能のため、CLI 仕様と JSON LINE スキーマは将来変更される可能性があります
