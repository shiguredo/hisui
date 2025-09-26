# `hisui compose` コマンド

`hisui compose` コマンドは、Sora が保存した録画ファイルを合成するためのコマンドです。

このコマンドは、録画されたメディアファイルを指定されたレイアウトに従って合成し、単一の動画ファイルとして出力します。

どのようなレイアウトが指定可能かについては [レイアウト機能](layout.md) のドキュメントをご参照ください。
デフォルトでは [layout-examples/compose-default.jsonc](../layout-examples/compose-default.jsonc) のレイアウトが使用されます。

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
  -h, --help                   このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version                バージョン番号を表示します
      --verbose                警告未満のログメッセージも出力します
  -l, --layout-file <PATH>     合成に使用するレイアウトファイルを指定します [env: HISUI_LAYOUT_FILE_PATH] [default: HISUI_REPO/layout-examples/compose-default.jsonc]
  -o, --output-file <PATH>     合成結果を保存するファイルを指定します [default: ROOT_DIR/output.mp4]
  -s, --stats-file <PATH>      合成中に収集した統計情報 (JSON) を保存するファイルを指定します
      --openh264 <PATH>        OpenH264 の共有ライブラリのパスを指定します [env: HISUI_OPENH264_PATH]
  -P, --no-progress-bar        指定された場合は、合成の進捗を非表示にします
  -T, --thread-count <INTEGER> 合成処理に使用するワーカースレッド数を指定します [env: HISUI_THREAD_COUNT] [default: 1]
```

## 実行例

### デフォルトレイアウトでの合成

```console
$ hisui compose /path/to/archive/RECORDING_ID/
  [00:00:09] [########################################] 27/27s (0s)
{
  "input_root_dir": "/path/to/archive/RECORDING_ID/",
  "input_audio_source_count": 2,
  "input_video_source_count": 2,
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
  "total_audio_decoder_processing_seconds": 0.064662641,
  "total_video_decoder_processing_seconds": 0.100056614,
  "total_audio_encoder_processing_seconds": 0.130539243,
  "total_video_encoder_processing_seconds": 8.984610705,
  "total_audio_mixer_processing_seconds": 0.007026693,
  "total_video_mixer_processing_seconds": 0.017125167,
}
```

### レイアウトファイルを指定しての合成

```console
$ hisui compose -l layout-examples/compose-default.jsonc /path/to/archive/RECORDING_ID/
  [00:00:09] [########################################] 27/27s (0s)
{
  "layout_file_path": "layout-examples/compose-default.jsonc",
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
  "total_audio_decoder_processing_seconds": 0.064662641,
  "total_video_decoder_processing_seconds": 0.100056614,
  "total_audio_encoder_processing_seconds": 0.130539243,
  "total_video_encoder_processing_seconds": 8.984610705,
  "total_audio_mixer_processing_seconds": 0.007026693,
  "total_video_mixer_processing_seconds": 0.017125167,
  "elapsed_seconds": 9.098501
}
...
```

## Tips

### 実行環境で利用可能なコーデックを確認する方法

Hisui でのエンコードおよびデコード時に利用可能なコーデックは、様々な要因によって変ります。
実際の環境でどのコーデックが使用できるかは [`hisui list-codecs`](command_list_codecs.md) コマンドで確認できます。

### 最適な映像エンコードパラメーターの決定方法

Hisui ではエンコーダー毎に、細かくエンコードパラメーターが指定できるようになっています。

具体的にどのパラメーターが最適かは、ユーザーの要件（例えば、映像品質を優先したいのか、それとも合成速度を優先したいのか）や実行環境によって変ります。

[`hisui tune`](command_tune.md) コマンドを利用することで、エンコーダーに詳しくなくても、最適なパラメーターの探索を行いやすくなっているので、ぜひ試してみてください。

### マルチスレッドで合成を行う方法

Hisui はデフォルトではシングルスレッドで合成処理を実行しますが、`--thread-count` オプションを指定することでマルチスレッドで処理させることができます。
もし一度に実行する Hisui プロセスがひとつだけで、できるだけ合成処理時間を短くしたい場合には、このオプションの値に CPU の物理コアの数を指定してみてください。

なお、この `--thread-count` オプションが制御するのは Hisui 自体のワーカースレッド数のみです。
映像エンコーダーは内部的に独自のマルチスレッド処理を行うことが多いですが、それらのスレッド数はこのオプションの影響を受けません。
エンコーダー内部のスレッド数を制御したい場合は、レイアウトファイルの中で
各エンコーダー固有の設定パラメーター（例：`libvpx_vp8_encode_params.threads` や `openh264_encode_params.thread_count` など）を使用してください。

