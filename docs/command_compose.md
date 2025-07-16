# `hisui compose` コマンド

`hisui compose` コマンドは、Sora が保存した録画ファイルを合成するためのコマンドです。

このコマンドは、録画されたメディアファイルを指定されたレイアウトに従って合成し、単一の動画ファイルとして出力します。

どのようなレイアウトが指定可能かについては [レイアウト機能](layout.md) のドキュメントをご参照ください。
デフォルトでは [layout-examples/compose-default.json](../layout-examples/compose-default.json) のレイアウトが使用されます。

## 使用方法

```console
$ hisui compose -h
Recording Composition Tool Hisui

Usage: hisui ... compose [OPTIONS] ROOT_DIR

Example:
  $ hisui compose /path/to/archive/RECORDING_ID/

Arguments:
  ROOT_DIR 合成処理を行う際のルートディレクトリを指定します

Options:
  -h, --help                    このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version                 バージョン番号を表示します
      --verbose                 警告未満のログメッセージも出力します
  -l, --layout-file <PATH>      合成に使用するレイアウトファイルを指定します [env: HISUI_LAYOUT_FILE_PATH]
  -o, --output-file <PATH>      合成結果を保存するファイルを指定します [default: output.mp4]
  -s, --stats-file <PATH>       合成中に収集した統計情報 (JSON) を保存するファイルを指定します
      --openh264 <PATH>         OpenH264 の共有ライブラリのパスを指定します [env: HISUI_OPENH264_PATH]
  -P, --no-progress-bar         指定された場合は、合成の進捗を非表示にします
  -c, --max-cpu-cores <INTEGER> 合成処理を行うプロセスが使用するコア数の上限を指定します [env: HISUI_MAX_CPU_CORES]
```

## 実行例

### デフォルトレイアウトでの合成

```console
$ hisui compose /path/to/archive/RECORDING_ID/
  [00:00:09] [########################################] 27/27s (0s)
{
  "input_root_dir": "/path/to/archive/RECORDING_ID/",
  "input_audio_file_count": 2,
  "input_video_file_count": 2,
  "output_file_path": "/path/to/archive/RECORDING_ID/output.mp4",
  "output_audio_codec": "OPUS",
  "output_audio_encoder_name": "opus",
  "output_audio_duration_seconds": 26.96,
  "output_audio_bitrate": 58566,
  "output_video_codec": "VP8",
  "output_video_encoder_name": "libvpx",
  "output_video_duration_seconds": 27,
  "output_video_bitrate": 375525,
  "output_video_width": 642,
  "output_video_height": 240,
  "elapsed_seconds": 9.098501
}
```

### レイアウトファイルを指定しての合成

```console
$ hisui compose -l layout-examples/compose-default.json /path/to/archive/RECORDING_ID/
  [00:00:09] [########################################] 27/27s (0s)
{
  "layout_file_path": "layout-examples/compose-default.json",
  "input_root_dir": "/path/to/archive/RECORDING_ID/",
  "input_audio_file_count": 2,
  "input_video_file_count": 2,
  "output_file_path": "/path/to/archive/RECORDING_ID/output.mp4",
  "output_audio_codec": "OPUS",
  "output_audio_encoder_name": "opus",
  "output_audio_duration_seconds": 26.96,
  "output_audio_bitrate": 58566,
  "output_video_codec": "VP8",
  "output_video_encoder_name": "libvpx",
  "output_video_duration_seconds": 27,
  "output_video_bitrate": 375525,
  "output_video_width": 642,
  "output_video_height": 240,
  "elapsed_seconds": 9.098501
}
...
```

## Tips

### Hisui が使用する CPU コア数を制限する方法

TODO: 複数の合成処理を同時に実行するような用途では、お互いが CPU リソースを食い合わないように、各プロセスが使用するコア数を制限できるようにすることも重要なので、その方法を記載する。

### 実行環境で利用可能なコーデックを確認する方法

Hisui でのエンコードおよびデコード時に利用可能なコーデックは、様々な要因によって変ります。
実際の環境でどのコーデックが使用できるかは [`hisui list-codecs`](command_list_codecs.md) コマンドで確認できます。

### 最適な映像エンコードパラメーターの決定方法

Hisui ではエンコーダー毎に、細かくエンコードパラメーターが指定できるようになっています。

具体的にどのパラメーターが最適かは、ユーザーの要件（例えば、映像品質を優先したいのか、それとも合成速度を優先したいのか）や実行環境によって変ります。

[`hisui tune`](command_tune.md) コマンドを利用することで、エンコーダーに詳しくなくても、最適なパラメーターの探索を行いやすくなっているので、興味のある人は試してみてください。
