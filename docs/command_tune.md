# `hisui tune` コマンド

`hisui tune` コマンドは、映像のエンコードパラメーターの最適化を行うためのコマンドです。

このコマンドは、[Optuna](https://optuna.org/) を使用して、指定されたレイアウトファイル内のエンコードパラメーターを自動的に調整し、**合成実行時間の最小化**と **映像品質（VMAF スコア）の最大化**という 2 つの目的を両立する最適なパラメーターセットを探索します。

## 最適化（エンコードパラメーターの探索・調整） のモチベーション

使用するコーデックやエンコーダーによっては、エンコードパラメータ次第でエンコード後の画像の品質やエンコード時間が大きく変わることがあります。
通常は、エンコードは合成処理全体の中で一番重い処理ですが、 エンコードパラメーターによってエンコード時間に数倍の差が出ることも珍しくありません。

しかし、一般に、最適なエンコードパラメーターは要件（例えば画質を優先したいのか、それともエンコード処理を軽くしたいのか、ビットレートをできるだけ抑えたいのか）や、実際の環境（マシンスペックや OS）、合成の条件（例えば解像度やフレームレートの値） などによって変わります。

そのため、合成を効果的・効率的に行うには、個々のユースケースに合わせた調整が重要になります。
`hisui tune` はそれを手軽に行えるようにするためのコマンドです。

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

## 探索用のレイアウトファイル

`hisui tune` コマンドで使用するレイアウトファイルは基本的には通常のものと同様です。
ただし、JSON オブjエクトのメンバーの値が `null` の場合には、
それが Optuna によって提案された値に置換された上で `hisui vmaf` コマンドに渡される点が異なります。

例えば以下は、デフォルトで使われる [tune-libvpx-vp8.json](../layout-examples/tune-libvpx-vp8.json) の内容を一部抜粋したものです:

```json
{
  "resolution": "1280x720",
  "video_codec": "VP8",
  "video_bitrate": 1000000,
  "frame_rate": 30,
  ...
  "libvpx_vp8_encode_params": {
    "threads": 1,
    "keyframe_interval": 300,
    "min_quantizer": null,
    "max_quantizer": null,
    "cq_level": null,
    "deadline": null,
    ...
  }
}
```

この中の `resolution` や `libvpx_vp8_encode_params.threads` などには `null` ではない値が指定されているので、
各トライアルで固定の値が使われます。

一方、`libvpx_vp8_encode_params.min_quantizer` や `libvpx_vp8_encode_params.cq_level` には `null` が指定されているので、
各トライアルで別々の、Optuna が提案した値が使われることになります。

つまり、「探索したいパラメーターには `null` を指定する」という点が通常のレイアウトファイルとの差異となります。

## 探索空間定義ファイル

上述の通り、レイアウトファイルで指定するのはあくまでも「特定のパラメーターを探索対象に含めるかどうか」ということだけです。
「各パラメーターの具体的な探索範囲」は、別途 `--search-space-file` で指定したファイルで定義することになります。

例えば、以下は、デフォルトの探索空間定義である [full.json](../search-space-examples/full.json) からの抜粋です。

```json
  ...,
  "libvpx_vp8_encode_params.cq_level": {
    "min": 0,
    "max": 63
  },
  "libvpx_vp8_encode_params.deadline": [
    "best",
    "good",
    "realtime"
  ],
  ...,
```

この例では、`cq_level` の値を 0 から 63 の範囲内から、`deadline` の値を `["best", "good", "realtime"]` の中から、
選択するような探索空間定義となっています。

`hisui tune` は以下の 3 つの形式の範囲定義をサポートしています:
- 整数範囲: `{"min": MIN_INT, "max": MAX_INT}`
- 小数範囲: `{"min": MIN_FLOAT, "max": MAX_FLOAT}`
- 値リスト: `[ JSON_VALUE ]`

なおユースケース毎に修正が必要なレイアウトファイルとは異なり、
通常は、探索空間定義については `full.json` をそのまま使って問題ありません。

## Tips

### 探索に使用するレイアウトファイルの作成方法

`hisui` リポジトリには各コーデック・エンコーダー毎に参考にできるレイアウトファイルが用意されています:
- VP8 (libvpx): [tune-libvpx-vp8.json](../layout-examples/tune-libvpx-vp8.json)
- VP9 (libvpx): [tune-libvpx-vp9.json](../layout-examples/tune-libvpx-vp9.json)
- AV1 (SVT-AV1): [tune-svt-av1.json](../layout-examples/tune-svt-av1.json)
- H.264 (OpenH264): [tune-openh264.json](../layout-examples/tune-openh264.json)
- H.264 (Video Toolbox): [tune-video-toolbox-h264.json](../layout-examples/tune-video-toolbox-h264.json)
- H.265 (Video Toolbox): [tune-video-toolbox-h265.json](../layout-examples/tune-video-toolbox-h265.json)

これらをベースにした上で、`video_layout` や `resolution` などの項目を各自のユースケースに合わせて修正するのが簡単です。

なお、`hisui tune` では音声の合成は行われないので、音声関連の項目がレイアウトファイルに含まれていても、単に無視されます。

### 探索に使用する探索空間定義ファイルはどうすべきか

基本的には、デフォルトの [full.json](../search-space-examples/full.json) をそのまま使えば大丈夫です。

ただし、以下のような場合は `full.json` を修正して使用するのがいいです:
- `full.json` に含まれていないパラメーターを調整したい場合
  - 例えば（通常は行わないですが）`video_bitrate` や `frame_frame` の値を調整したい場合には、自分で探索範囲を定義する必要があります
- `full.json` の探索範囲を狭めたい場合
  - デフォルトの探索空間定義はかなり広めになっています
  - 以前の探索の知見などから、必要な探索範囲がある程度判明している時には、デフォルトよりも範囲を限定することで、より効率的な探索が行えるようになります

### 探索対象から除外した方がいいパラメーター

`hisui tune` の仕組みは柔軟なので、やろうと思えば レイアウト JSON 内のほぼ全ての項目を探索対象に含めることができます。

しかし、以下のパラメーターは探索対象から除外して、事前に決めた固定値を使用すべきです:
- 解像度 (`resolution`):
  - `hisui vmaf` コマンドでは「合成後の画像（参照画像）」と「それをエンコードした後の画像」を比較して VMAF スコアを計算します
  - つまり、あくまでも「エンコードによる画質劣化度合い」を求めているのであって、「合成（主にリサイズ）による画質劣化の度合い」は考慮されません
  - レイアウトの `resolution` の値が変わると、参照画像自体が変わることになるので、それをもとに計算された VMAF スコア同時を直接的に比較することはできません
- コーデック (`video_codec`):
  - `hisui vmaf` コマンドでは VMAF スコアを計算するために、合成画像をエンコードした後に再度デコードして生画像を取得しています
  - この追加のデコード処理のコストはコーデック（正確にはデコーダー）によって変わります
  - つまり、コーデックを探索すると、デコード処理が軽いコーデックの方が誤って優先されてしまうことになります

また、一般論として、探索対象のパラメーターが少ないほど探索効率が上がるので、
事前に適切な値が判明しているパラメーターは、値を固定して探索対象から除外するのが望ましいです。

### 異なるコーデックの探索中の合成実行時間はそのまま比較できないので注意

エンコードの後に追加のデコード処理があって、このコストがコーデックによって変わる。
探索後に `hisui compose` を実行して比較するのがいい。

### 探索は実際に環境や入力ストリームにできるだけ合わせて行うのが大事

TODO

### 探索時間を短縮する方法

実行環境やエンコーダーによっては、一回のトライアルの評価に長時間掛かることがある

- `--frame-count` で指定する値を小さくする


### 以前の探索の続きから再開したい場合

同じストレージファイルとスタディ名でコマンドを実行する

### 探索空間やレイアウトファイルを変更して探索を行いたい場合

ストレージファイルの削除やスタディ名の変更を忘れずに行う必要がある。

### トライアルが失敗した場合にどうすべきか

- エンコーダーとパラメーターセットの組み合わせによってはどうしようもないこともある（この場合は無視する）
- TODO

### 実際のエンコードビットレートの確認方法

レイアウト JSON でビットレートを指定できるけど、それが必ずしも正確に守られるとは限らない。
`hisui compose` を実行すると実際のビットレートが出力されるので、最終的にはそれで確認するといい。

### 実際の合成時間の確認方法


### ディスク使用量が大きいのをなんとかしたい

- YUVデータなどが残っているのが原因
- トライアルの評価後は使用されないので、削除してしまっても問題ない

### トライアル回数をどうすべきか

- 最適な値はケースバイケース
- デフォルトの 100 は多くの場合によく動作するデフォルト値
- 目安としては、最適解の集合がしばらく更新されなくなるまで探索を継続する、というのがある


### Optuna の可視化機能や optuna dashboard について

TODO



