# `hisui tune` コマンド

`hisui tune` コマンドは、映像のエンコードパラメーターの最適化を行うためのコマンドです。

このコマンドは、[Optuna](https://optuna.org/) を使用して、指定されたレイアウトファイル内のエンコードパラメーターを自動的に調整し、**合成実行時間の最小化**と **映像品質（VMAF スコア）の最大化**という 2 つの目的を両立する最適なパラメーターセットを探索します。

TODO: 最適化を行うモチベーションについて少し書く


## 依存パッケージ

このコマンドを利用するためには、以下のパッケージがシステムにインストールされている必要があります：

- `optuna` - パラメーター最適化ツール
- `vmaf` - 映像品質評価ツール（[`hisui vmaf`](command_vmaf.md) コマンドと共通）

macOS の場合には以下のようにして、依存パッケージがインストールできます
（[uv](https://docs.astral.sh/uv/) はPython用のパッケージマネージャーです）：

```console
$ brew install vmaf
$ uv tool install optuna
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

## Optuna による最適化の概要

### 最適化の流れ

Optuna による最適化は、以下のような流れとなります:
1. ユーザーがパラメーターの探索空間を指定する
   - `hisui tune` コマンドの `--layout-file` および `--search-space-file` の指定がこれに該当
2. Optuna は、探索空間の中から次に探索するパラメーターセットをサンプリングする
3. Hisui はサンプリングされたパラメーターセットを `hisui vmaf` コマンドを使って評価する
   - Optunaの用語では「パラメーターセットのサンプリングと評価」をまとめたものを「トライアル」と呼称
4. Hisui は評価結果を Optuna にフィードバックする
5. Optuna は次のトライアルでのサンプリングの参考にするために、フィードバック結果を探索履歴に反映する
   - Optuna の探索履歴は SQLite のデータベースファイルに格納されている
6. 2 に戻って、次のトライアルを開始する
   - これを`--trial-count`で指定の回数に達するまで繰り返す

なおこの一連の流れは `hisui tune` によってラップされているため、ユーザーが細かく意識する必要はありません。

### 最適化メトリクス

`hisui tune` コマンドは以下の2つの指標を同時に最適化します：

1. **実行時間（最小化）** - 映像エンコード処理にかかる時間を短縮
2. **VMAF スコア平均値（最大化）** - 映像品質を向上

これらは多目的最適化問題として扱われ、Optuna のパレートフロント探索によって、両方の目的を考慮した最適解の集合（パレート解）が見つけられます。

多目的最適化の場合には単一の最適解は定まらないので、
トレードオフを含んだ最適解の集合の中から最終的に使用する解（パラメーターセット）を選択するのは
ユーザーの責務となります。

## 実行例

### デフォルト設定での実行

オプションを指定しなかった場合には、以下のデフォルト設定で最適化が実行されます:
- レイアウトファイル: [layout-examples/tune-libvpx-vp8.json](../layout-examples/tune-libvpx-vp8.json)
- 探索空間定義ファイル: [search-space-examples/full.json](../search-space-examples/full.json)

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

`hisui tune` コマンドの出力には、以下のような情報が含まれています:
- `====== INFO ======`
  - 探索（最適化）の基本情報が表示されます
- `====== OPTUNA TRIAL ({I}/{N}) ======`
  - Optuna の各トライアルの情報が表示されます
- `====== BEST TRIALS (sorted by execution time) ======`
  - 探索によって見つかった最適解の集合が表示されます
  - 表示タイミングは以下の通りです:
    - `hisui tune` コマンドを実行して、最初のトライアルの完了後
    - 新しい最適解が発見されて、最適解集合が更新された後
    - `hisui tune` コマンドが指定のトライアル回数の実行を完了して終了する時

最適解集合の表示には、
そのパラメーターセットを使って合成を行うコマンドの例（`$ hisui compose -l ...`）も含まれているので、
見つかった最適解を簡単に試すことができます。

なお `[I 2025-07-16 12:35:43,172] ...` という形式のログ出力は Optuna によるものです。

## 探索に使用するレイアウトファイルの形式

TODO

## 探索空間ファイルの形式

TODO

## Tips

### 探索に使用するレイアウトファイルをどうするか

TODO

### デフォルト以外の探索空間を使った方がいい場合

TODO

### 探索時のCPUコア数制限について

TODO: 実際の合成時の条件に合わせておくのがいい

### 探索時間を短縮する方法

実行環境やエンコーダーによっては、一回のトライアルの評価に長時間掛かることがある

- `--frame-count` で指定する値を小さくする

### 実際のエンコードビットレートの確認方法

レイアウト JSON でビットレートを指定できるけど、それが必ずしも正確に守られるとは限らない。
`hisui compose` を実行すると実際のビットレートが出力されるので、最終的にはそれで確認するといい。

### 以前の探索の続きから再開したい場合

同じストレージファイルとスタディ名でコマンドを実行する

### 探索空間やレイアウトファイルを変更して探索を行いたい場合

ストレージファイルの削除やスタディ名の変更を忘れずに行う必要がある。

### トライアルが失敗した場合にどうすべきか

- エンコーダーとパラメーターセットの組み合わせによってはどうしようもないこともある（この場合は無視する）
- TODO

### ディスク使用量が大きいのをなんとかしたい

- YUVデータなどが残っているのが原因
- トライアルの評価後は使用されないので、削除してしまっても問題ない

### トライアル回数をどうすべきか

- 最適な値はケースバイケース
- デフォルトの 100 は多くの場合によく動作するデフォルト値
- 目安としては、最適解の集合がしばらく更新されなくなるまで探索を継続する、というのがある

### 異なるコーデックの探索中の合成実行時間はそのまま比較できないので注意

エンコードの後に追加のデコード処理があって、このコストがコーデックによって変わる。
探索後に `hisui compose` を実行して比較するのがいい。

### 探索時に固定しておいた方がいいパラメーター群

TODO
