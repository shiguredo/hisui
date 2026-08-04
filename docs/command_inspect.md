# `hisui inspect` コマンド（開発者向け）

`hisui inspect` コマンドは、MP4 ファイルの詳細情報を取得するためのコマンドです。
このコマンドは主にデバッグやファイル分析に使用されます。

## 使用方法

```console
$ hisui inspect -h
録画ファイルの情報を取得します

Usage: hisui ... inspect [OPTIONS] INPUT_FILE

Example:
  $ hisui inspect /path/to/archive.mp4

Arguments:
  INPUT_FILE 情報取得対象の録画ファイル(.mp4)

Options:
  -h, --help              このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version           バージョン番号を表示します
      --verbose           警告未満のログメッセージも出力します
      --emit-exit-metrics プロセス終了時に内部メトリクスを JSON Lines 形式で標準出力へ 1 行出力します [env: HISUI_EMIT_EXIT_METRICS]
  -x, --experimental      実験的サブコマンドの有効化フラグです
      --decode            指定された場合にはデコードまで行います
      --openh264 <PATH>   OpenH264 の共有ライブラリのパス [env: HISUI_OPENH264_PATH]
```

## 実行例

MP4 ファイルを指定して実行すると、ファイルのパース結果が JSON 形式で出力されます。

```console
$ hisui inspect /path/to/archive.mp4 | head
{
  "path": "/path/to/archive.mp4",
  "format": "mp4",
  "audio_codec": "OPUS",
  "audio_duration_us": 26960000,
  "audio_sample_count": 1348,
  "audio_samples": [
    { "timestamp_us": 0, "duration_us": 20000, "data_size": 3 },
    { "timestamp_us": 20000, "duration_us": 20000, "data_size": 3 },
    { "timestamp_us": 40000, "duration_us": 20000, "data_size": 3 },
...
```
