# `hisui tune` コマンド

`hisui tune` コマンドは、映像エンコードパラメータの最適化を行うためのコマンドです。

このコマンドは、[Optuna](https://optuna.org/) を使用して、指定されたレイアウトファイル内のエンコードパラメータを自動的に調整し、**実行時間の最小化**と **VMAF スコアの最大化**という2つの目的を両立する最適なパラメータセットを探索します。

## 依存パッケージ

このコマンドを利用するためには、以下のパッケージがシステムにインストールされている必要があります：

- `optuna` - パラメータ最適化フレームワーク
- `vmaf` - 映像品質評価ツール（[`hisui vmaf`](command_vmaf.md) コマンドと共通）

macOS の場合には以下のようにして、依存パッケージがインストールできます：

```console
$ brew install vmaf
$ pip install optuna
```

## 使用方法

```console
$ hisui tune -h
Recording Composition Tool Hisui

Usage: hisui ... tune [OPTIONS] ROOT_DIR

Example:
  $ hisui tune /path/to/archive/RECORDING_ID/

Arguments:
  ROOT_DIR 調整処理を行う際のルートディレクトリを指定します

Options:
  -h, --help                     このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version                  バージョン番号を表示します
      --verbose                  警告未満のログメッセージも出力します
  -l, --layout-file <PATH>       パラメータ調整に使用するレイアウトファイルを指定します
  -s, --search-space-file <PATH> 探索空間定義ファイル（JSON）のパスを指定します
      --tune-working-dir <PATH>  チューニング用に使われる作業ディレクトリを指定します [default: hisui-tune/]
      --study-name <NAME>        Optuna の study 名を指定します [default: hisui-tune]
  -n, --trial-count <INTEGER>    実行する試行回数を指定します [default: 100]
      --openh264 <PATH>          OpenH264 の共有ライブラリのパスを指定します [env: HISUI_OPENH264_PATH]
  -c, --max-cpu-cores <INTEGER>  調整処理を行うプロセスが使用するコア数の上限を指定します [env: HISUI_MAX_CPU_CORES]
  -f, --frame-count <FRAMES>     調整用にエンコードする映像フレームの数を指定します [default: 300]
```

## 最適化メトリクス

`hisui tune` コマンドは以下の2つの指標を同時に最適化します：

1. **実行時間（最小化）** - 映像エンコード処理にかかる時間を短縮
2. **VMAF スコア平均値（最大化）** - 映像品質を向上

これらは多目的最適化問題として扱われ、Optuna のパレートフロント探索によって、両方の目的を考慮した最適解の集合（パレート解）が見つけられます。

## 実行例

### デフォルト設定での実行

```console
$ hisui tune /path/to/archive/RECORDING_ID/
====== INFO ======
layout file to tune:    DEFAULT
search space file:      DEFAULT
tune working dir:       /path/to/archive/RECORDING_ID/hisui-tune/
optuna storage: sqlite:///path/to/archive/RECORDING_ID/hisui-tune/optuna.db
optuna study name:      hisui-tune
optuna trial count:     100
tuning metrics: [Execution Time (minimize), VMAF Score Mean (maximize)]
tuning parameters (7):
  video_toolbox_h265_encode_params.allow_open_gop:       [true,false]
  video_toolbox_h265_encode_params.allow_temporal_compression:   [true,false]
  video_toolbox_h265_encode_params.max_frame_delay_count:        {"min":1,"max":16}
  video_toolbox_h265_encode_params.maximize_power_efficiency:    [true,false]
  video_toolbox_h265_encode_params.profile_level:        ["main","main10"]
  video_toolbox_h265_encode_params.real_time:    [true,false]
  video_toolbox_h265_encode_params.use_parallelization:  [true,false]

====== CREATE OPTUNA STUDY ======
[I 2025-07-16 12:35:41,907] A new study created in RDB with name: hisui-tune

====== OPTUNA TRIAL (1/100) ======
=== SAMPLE PARAMETERS ===
[I 2025-07-16 12:35:42,360] Asked trial 0 with parameters {'video_toolbox_h265_encode_params.allow_open_gop': False, 'video_toolbox_h265_encode_params.allow_temporal_compression': True, ...}.

=== EVALUATE PARAMETERS ===
$ "hisui" "vmaf" "--layout-file" "/path/to/trial-0/layout.json" ...

# Compose for VMAF
  [00:00:00] [########################################] 10/10 (0s)
=> done

# Run vmaf command
VMAF version 3.0.0
10 frames ⢋⠀ 0.00 FPS
vmaf_v0.6.1: 90.988820
=> done

[I 2025-07-16 12:35:43,172] Told trial 0 with values [0.5039638, 90.98882] and state 1.

====== BEST TRIALS (sorted by execution time) ======
Trial #0
  Execution Time:        0.5040s
  VMAF Score Mean:       90.9888
  Parameters:
    video_toolbox_h265_encode_params.allow_open_gop:     false
    video_toolbox_h265_encode_params.allow_temporal_compression:         true
    video_toolbox_h265_encode_params.max_frame_delay_count:      7
    video_toolbox_h265_encode_params.maximize_power_efficiency:  false
    video_toolbox_h265_encode_params.profile_level:      "main"
    video_toolbox_h265_encode_params.real_time:  true
    video_toolbox_h265_encode_params.use_parallelization:        true
  Compose Command:
    $ hisui compose -l /path/to/trial-0/layout.json /path/to/archive/RECORDING_ID/

...
```

### カスタムレイアウトファイルを使用した実行

```console
$ hisui tune -l layout-examples/tune-video-toolbox-h265.json /path/to/archive/RECORDING_ID/ -f 10
```

## 出力ファイル

実行時に以下のファイルが作成されます：

- `hisui-tune/optuna.db` - Optuna の実行履歴データベース
- `hisui-tune/hisui-tune/trial-N/layout.json` - 各試行で使用されたレイアウトファイル
- `hisui-tune/hisui-tune/trial-N/metrics.json` - 各試行の評価結果
- `hisui-tune/hisui-tune/trial-N/reference.yuv` - 参照映像（合成前）
- `hisui-tune/hisui-tune/trial-N/distorted.yuv` - 歪み映像（合成後）
- `hisui-tune/hisui-tune/trial-N/vmaf-output.json` - VMAF評価結果

## 実用的な使用方法

1. **少ないフレーム数で予備実行**: `-f 10` などで短時間でパラメータの傾向を確認
2. **最適化完了後の実際の合成**: 出力される `Compose Command` を使用して実際の合成を実行
3. **複数回実行**: 同じ `--study-name` を使用することで前回の結果を継続して最適化可能

このコマンドは、映像エンコードの品質と処理時間のバランスを取った最適なパラメータを見つけるのに非常に有効です。
