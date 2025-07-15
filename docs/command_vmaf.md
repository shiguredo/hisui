# `hisui vmaf` コマンド（開発者向け）

`hisui vmaf` コマンドは、録画ファイルの品質評価を行うためのコマンドです。

このコマンドは、参照映像（合成前）と歪み映像（合成後）の VMAF（Video Multimethod Assessment Fusion）スコアを計算し、映像品質の客観的な評価を提供します。

このコマンドは、主に [`hisui tune`](command_tune.md) コマンドと組み合わせて、
エンコードパラメーターの調整に利用することを想定しています。

## 依存パッケージ

このコマンドを利用するためには https://github.com/Netflix/vmaf が提供する `vmaf` コマンドがシステムにインストールされている必要があります。

macOS の場合には以下のようにして、依存パッケージがインストールできます。

```
$ brew install vmaf
```

## 使用方法

```console
$ hisui vmaf -h
Recording Composition Tool Hisui

Usage: hisui ... vmaf [OPTIONS] ROOT_DIR

Example:
  $ hisui vmaf /path/to/archive/RECORDING_ID/

Arguments:
  ROOT_DIR 合成処理を行う際のルートディレクトリを指定します

Options:
  -h, --help                      このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version                   バージョン番号を表示します
      --verbose                   警告未満のログメッセージも出力します
  -l, --layout-file <PATH>        合成に使用するレイアウトファイルを指定します [env: HISUI_LAYOUT_FILE_PATH]
      --reference-yuv-file <PATH> 参照映像（合成前）のYUVファイルの出力先を指定します [default: reference.yuv]
      --distorted-yuv-file <PATH> 歪み映像（合成後）のYUVファイルの出力先を指定します [default: distorted.yuv]
      --vmaf-output-file <PATH>   vmaf コマンドの実行結果ファイルの出力先を指定します [default: vmaf-output.json]
      --openh264 <PATH>           OpenH264 の共有ライブラリのパスを指定します [env: HISUI_OPENH264_PATH]
  -c, --max-cpu-cores <INTEGER>   合成処理を行うプロセスが使用するコア数の上限を指定します [env: HISUI_MAX_CPU_CORES]
  -f, --frame-count <FRAMES>      変換するフレーム数を指定します [default: 1000]
```

## 実行例

```console
$ hisui vmaf /path/to/録画ディレクトリ/
# Compose for VMAF
  [00:00:09] [########################################] 1000/1000 (0s)
=> done

# Run vmaf command
VMAF version 3.0.0
675 frames ⠄⠀ 201.74 FPS
vmaf_v0.6.1: 96.361266
=> done

{
  "reference_yuv_file_path": "/path/to/録画ディレクトリ/reference.yuv",
  "distorted_yuv_file_path": "/path/to/録画ディレクトリ/distorted.yuv",
  "vmaf_output_file_path": "/path/to/録画ディレクトリ/vmaf-output.json",
  "encoder_name": "libvpx",
  "width": 642,
  "height": 240,
  "frame_rate": 25,
  "encoded_frame_count": 675,
  "encoded_byte_size": 1267400,
  "encoded_duration_seconds": 27,
  "elapsed_seconds": 12.4662,
  "vmaf_min": 82.988863,
  "vmaf_max": 100,
  "vmaf_mean": 96.361266,
  "vmaf_harmonic_mean": 96.351588
}
```
