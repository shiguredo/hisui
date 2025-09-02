# `hisui vmaf` コマンド

`hisui vmaf` コマンドは、録画ファイルの合成結果のエンコード品質評価を行うためのコマンドです。

このコマンドは、参照映像（合成後の映像）と歪み映像（参照映像のエンコード後の映像）の
 VMAF（Video Multimethod Assessment Fusion）スコアを計算し、映像品質の客観的な評価を提供します。

具体的には、以下の処理を行います：

1. **参照映像の生成**: 録画ファイルを合成してエンコード前の生映像を出力
2. **歪み映像の生成**: 同じ合成映像をエンコード後、再度デコードした映像を出力
3. **VMAF 評価**: 両映像を比較して VMAF スコアを算出

これにより、エンコード処理による映像品質の劣化を定量的に測定できます。

このコマンドは、主に [`hisui tune`](command_tune.md) コマンドと組み合わせて、
映像エンコードパラメーターの調整に利用することを想定しています。

## 依存パッケージ

このコマンドを利用するためには https://github.com/Netflix/vmaf が提供する `vmaf` コマンドがシステムにインストールされている必要があります。

macOS の場合には以下のようにして、依存パッケージがインストールできます。

```console
$ brew install libvmaf
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
      --reference-yuv-file <PATH> 参照映像のYUVファイルの出力先を指定します [default: reference.yuv]
      --distorted-yuv-file <PATH> 歪み映像のYUVファイルの出力先を指定します [default: distorted.yuv]
      --vmaf-output-file <PATH>   vmaf コマンドの実行結果ファイルの出力先を指定します [default: vmaf-output.json]
      --openh264 <PATH>           OpenH264 の共有ライブラリのパスを指定します [env: HISUI_OPENH264_PATH]
  -c, --max-cpu-cores <INTEGER>   合成処理を行うプロセスが使用するコア数の上限を指定します [env: HISUI_MAX_CPU_CORES]
  -f, --frame-count <FRAMES>      変換するフレーム数を指定します [default: 1000]
```

## 実行例

```console
$ hisui vmaf /path/to/archive/RECORDING_ID/
# Compose for VMAF
  [00:00:09] [########################################] 1000/1000 (0s)
=> done

# Run vmaf command
VMAF version 3.0.0
675 frames ⠄⠀ 201.74 FPS
vmaf_v0.6.1: 96.361266
=> done

{
  "reference_yuv_file_path": "/path/to/archive/RECORDING_ID/reference.yuv",
  "distorted_yuv_file_path": "/path/to/archive/RECORDING_ID/distorted.yuv",
  "vmaf_output_file_path": "/path/to/archive/RECORDING_ID/vmaf-output.json",
  "encoder_name": "libvpx",
  "width": 642,
  "height": 240,
  "frame_rate": 25,
  "encoded_frame_count": 675,
  "elapsed_seconds": 12.4662,
  "vmaf_min": 82.988863,
  "vmaf_max": 100,
  "vmaf_mean": 96.361266,
  "vmaf_harmonic_mean": 96.351588
}
```

VMAF スコアの解釈方法については、VMAF 自体の[公式リポジトリ](https://github.com/Netflix/vmaf)や、
そこから辿れるドキュメントを参照してください。

