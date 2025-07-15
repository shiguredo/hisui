# `hisui compose` コマンド

`hisui compose` コマンドは、録画ファイルを合成して最終的な動画ファイルを生成するためのコマンドです。

このコマンドは、録画されたメディアファイルを指定されたレイアウトに従って合成し、単一の動画ファイルとして出力します。

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

### 基本的な実行

```console
$ hisui compose /path/to/archive/RECORDING_ID/
  [00:00:09] [########################################] 27/27s (0s)
```

### レイアウトファイルを指定した実行

```console
$ hisui compose -l layout-examples/compose-default.json /path/to/archive/RECORDING_ID/
  [00:00:09] [########################################] 27/27s (0s)
```

## 主要オプション

- `--layout-file` (`-l`): 合成に使用するレイアウトファイルを指定します。指定しない場合は、デフォルトのレイアウトが使用されます。
- `--output-file` (`-o`): 合成結果の出力ファイル名を指定します。デフォルトは `output.mp4` です。
- `--stats-file` (`-s`): 合成処理の統計情報を JSON 形式で保存するファイルを指定します。
- `--max-cpu-cores` (`-c`): 合成処理で使用するCPUコア数の上限を指定します。
- `--no-progress-bar` (`-P`): 進捗バーを非表示にします。

## 関連コマンド

- [`hisui vmaf`](command_vmaf.md): 合成後の映像品質評価を行う際に使用
- [`hisui tune`](command_tune.md): エンコードパラメーターの調整に使用
