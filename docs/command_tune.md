# `hisui tune` コマンド

`hisui tune` コマンドは、映像のエンコードパラメーターの最適化を行うためのコマンドです。

このコマンドは、[Optuna](https://optuna.org/) を使用して、指定されたレイアウトファイル内のエンコードパラメーターを自動的に調整し、**合成実行時間の最小化**と **映像品質（VMAF スコア）の最大化**という 2 つの目的を両立する最適なパラメーターセットを探索します。

## 最適化（エンコードパラメーターの探索・調整） のモチベーション

使用するコーデックやエンコーダーによっては、エンコードパラメータ次第でエンコード後の画像の品質やエンコード時間が大きく変わることがあります。
通常は、エンコードは合成処理全体の中で一番重い処理ですが、 エンコードパラメーターによってエンコード時間に数倍の差が出ることも珍しくありません。

しかし、一般に、最適なエンコードパラメーターは要件（例えば画質を優先したいのか、それともエンコード処理を軽くしたいのか、ビットレートをできるだけ抑えたいのか）や、実際の環境（マシンスペックや OS）、合成の条件（例えば解像度やフレームレートの値） などによって変わります。

そのため、合成を効果的・効率的に行うには、個々のユースケースに合わせた調整が重要になります。
`hisui tune` はそれを手軽に行えるようにするためのコマンドです。

## 最適化の流れ

`hisui tune` を利用してエンコードパラメーターの調整を行う際の流れは、典型的には次のようになります。

1. ベースとするレイアウトファイルを決定する
2. 上のレイアウトファイルの中で、調整対象となるパラメーターを決定する
3. 探索で使用する録画ファイルを用意する
4. `hisui tune` を実行する
5. `hisui tune` によって見つかったレイアウトファイル群の中から、よさそうな候補をいくつか選択する
6. `hisui compose` で、候補レイアウトファイルを使って実際に合成を行い、最終的に採用するレイアウトファイルを決定する
7. 必要に応じて、最終的なレイアウトファイルの内容を手で微調整する


## 依存パッケージ

このコマンドを利用するためには、以下のパッケージがシステムにインストールされている必要があります。

- `optuna` - パラメーター最適化ツール
- `vmaf` - 映像品質評価ツール（[`hisui vmaf`](command_vmaf.md) コマンドと共通）

macOS の場合には以下のようにして、依存パッケージがインストールできます
（[uv](https://docs.astral.sh/uv/) はPython用のパッケージマネージャーです）：

```console
$ brew install libvmaf
$ uv tool install optuna
```

### Ubuntu で利用する場合

Ubuntu では依存パッケージをビルドする必要があります。

#### vmaf のビルド

1. 必要なパッケージをインストール

```console
$ sudo apt-get update
$ sudo apt-get install ninja-build meson nasm
```

2. vmaf [ソースコード](https://github.com/Netflix/vmaf/releases) をダウンロード
3. ソースコードの展開とビルド

```console
# X.Y.Z はダウンロードした vmaf のバージョン
$ tar -xzf vmaf-X.Y.Z.tar.gz
$ cd vmaf-X.Y.Z/libvmaf
$ meson build --buildtype release
$ ninja -vC build
$ sudo ninja -vC build install
```

#### optuna のインストール

macOS と同様に uv を利用してインストールします。

```console
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
  -l, --layout-file <PATH>       パラメータ調整に使用するレイアウトファイルを指定します [default: HISUI_REPO/layout-examples/tune-libvpx-vp9.jsonc]
  -s, --search-space-file <PATH> 探索空間定義ファイル（JSON）のパスを指定します [default: HISUI_REPO/search-space-examples/full.jsonc]
      --tune-working-dir <PATH>  チューニング用に使われる作業ディレクトリを指定します [default: ROOT_DIR/hisui-tune/]
      --study-name <NAME>        Optuna の study 名を指定します [default: hisui-tune]
  -n, --trial-count <INTEGER>    実行する試行回数を指定します [default: 100]
  -t, --trial-timeout <SECONDS>  各試行トライアルのタイムアウト時間（秒）を指定します（超過した場合は失敗扱い）
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

`hisui tune` コマンドは以下の2つの指標を同時に最適化します。

1. **実行時間（最小化）** - 映像エンコード処理にかかる時間を短縮
2. **VMAF スコア平均値（最大化）** - 映像品質を向上

これらは多目的最適化問題として扱われ、Optuna のパレートフロント探索によって、両方の目的を考慮した最適解の集合（パレート解）が見つけられます。

多目的最適化の場合には単一の最適解は定まらないので、
トレードオフを含んだ最適解の集合の中から最終的に使用する解（パラメーターセット）を選択するのは
ユーザーの責務となります。

## 実行例

### デフォルト設定での実行

オプションを指定しなかった場合には、以下のデフォルト設定で最適化が実行されます。
- レイアウトファイル: [layout-examples/tune-libvpx-vp9.jsonc](../layout-examples/tune-libvpx-vp9.jsonc)
- 探索空間定義ファイル: [search-space-examples/full.jsonc](../search-space-examples/full.jsonc)

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
$ "hisui" "vmaf" "--layout-file" "/path/to/trial-0/layout.jsonc" ...

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
    $ hisui compose -l /path/to/trial-0/layout.jsonc /path/to/archive/RECORDING_ID/

...
```

`hisui tune` コマンドの出力には、以下のような情報が含まれています。
- `====== INFO ======`
  - 探索（最適化）の基本情報が表示されます
- `====== OPTUNA TRIAL ({I}/{N}) ======`
  - Optuna の各トライアルの情報が表示されます
- `====== BEST TRIALS (sorted by execution time) ======`
  - 探索によって見つかった最適解の集合が表示されます
  - 表示タイミングは以下の通りです
    - `hisui tune` コマンドを実行して、最初のトライアルの完了後
    - 新しい最適解が発見されて、最適解集合が更新された後
    - `hisui tune` コマンドが指定のトライアル回数の実行を完了して終了する時

最適解集合の表示には、
そのパラメーターセットを使って合成を行うコマンドの例（`$ hisui compose -l ...`）も含まれているので、
見つかった最適解の合成を簡単に試すことができます。

`hisui tune` コマンドの探索結果についての出力は必要最低限のものとなっていますが、
Optuna の可視化機能やダッシュボードを活用することで、より詳細な確認や分析が可能となります。
- 可視化機能: [Optuna Documentation - Visualization](https://optuna.readthedocs.io/en/stable/tutorial/10_key_features/005_visualization.html)
- ダッシュボード: [Optuna Dashboard](https://github.com/optuna/optuna-dashboard)

なお `[I 2025-07-16 12:35:43,172] ...` という形式のログ出力は Optuna によるものです。

## 探索用のレイアウトファイル

`hisui tune` コマンドで使用するレイアウトファイルは基本的には通常のものと同様です。
ただし、JSON オブジェクトのメンバーの値が `null` の場合には、
それが Optuna によって提案された値に置換された上で `hisui vmaf` コマンドに渡される点が異なります。

例えば以下は、デフォルトで使われる [tune-libvpx-vp9.jsonc](../layout-examples/tune-libvpx-vp9.jsonc) の内容を一部抜粋したものです。

```json
{
  "resolution": "1280x720",
  "video_codec": "VP9",
  "video_bitrate": 1000000,
  "frame_rate": 30,
  ...
  "libvpx_vp9_encode_params": {
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

この中の `resolution` や `libvpx_vp9_encode_params.threads` などには `null` ではない値が指定されているので、
各トライアルで固定の値が使われます。

一方、`libvpx_vp9_encode_params.min_quantizer` や `libvpx_vp9_encode_params.cq_level` には `null` が指定されているので、
各トライアルで別々の、Optuna が提案した値が使われることになります。

つまり、「探索したいパラメーターには `null` を指定する」という点が通常のレイアウトファイルとの差異となります。

## 探索空間定義ファイル

上述の通り、レイアウトファイルで指定するのはあくまでも「特定のパラメーターを探索対象に含めるかどうか」ということだけです。
「各パラメーターの具体的な探索範囲」は、別途 `--search-space-file` で指定したファイルで定義することになります。

例えば、以下は、デフォルトの探索空間定義である [full.jsonc](../search-space-examples/full.jsonc) からの抜粋です。

```json
  ...,
  "libvpx_vp9_encode_params.cq_level": {
    "min": 0,
    "max": 63
  },
  "libvpx_vp9_encode_params.deadline": [
    "best",
    "good",
    "realtime"
  ],
  ...,
```

この例では、`cq_level` の値を 0 から 63 の範囲内から、`deadline` の値を `["best", "good", "realtime"]` の中から、
選択するような探索空間定義となっています。

`hisui tune` は以下の 3 つの形式の範囲定義をサポートしています。
- 整数範囲: `{"min": MIN_INT, "max": MAX_INT}`
- 小数範囲: `{"min": MIN_FLOAT, "max": MAX_FLOAT}`
- 値リスト: `[ JSON_VALUE ]`

なおユースケース毎に修正が必要なレイアウトファイルとは異なり、
通常は、探索空間定義については `full.jsonc` をそのまま使って問題ありません。

## Tips

### 探索に使用するレイアウトファイルの作成方法

`hisui` リポジトリには各コーデック・エンコーダー毎に参考にできるレイアウトファイルが用意されています。
- VP8 (libvpx): [tune-libvpx-vp8.jsonc](../layout-examples/tune-libvpx-vp8.jsonc)
- VP9 (libvpx): [tune-libvpx-vp9.jsonc](../layout-examples/tune-libvpx-vp9.jsonc)
- AV1 (SVT-AV1): [tune-svt-av1.jsonc](../layout-examples/tune-svt-av1.jsonc)
- H.264 (OpenH264): [tune-openh264.jsonc](../layout-examples/tune-openh264.jsonc)
- H.264 (Video Toolbox): [tune-video-toolbox-h264.jsonc](../layout-examples/tune-video-toolbox-h264.jsonc)
- H.265 (Video Toolbox): [tune-video-toolbox-h265.jsonc](../layout-examples/tune-video-toolbox-h265.jsonc)

これらをベースにした上で、`video_layout` や `resolution` などの項目を各自のユースケースに合わせて修正するのが簡単です。

なお、`hisui tune` では音声の合成は行われないので、音声関連の項目がレイアウトファイルに含まれていても、単に無視されます。

### 探索に使用する探索空間定義ファイルはどうすべきか

基本的には、デフォルトの [full.jsonc](../search-space-examples/full.jsonc) をそのまま使えば大丈夫です。

ただし、以下のような場合は `full.jsonc` を修正して使用するのをお勧めします。
- `full.jsonc` に含まれていないパラメーターを調整したい場合
  - 例えば（通常は行わないですが）`video_bitrate` や `frame_rate` の値を調整したい場合には、自分で探索範囲を定義する必要があります
- `full.jsonc` の探索範囲を狭めたい場合
  - デフォルトの探索空間定義はかなり広めになっています
  - 以前の探索の知見などから、必要な探索範囲がある程度判明している時には、デフォルトよりも範囲を限定することで、より効率的な探索が行えるようになります

### 探索対象から除外した方がいいパラメーター

`hisui tune` の仕組みは柔軟なので、やろうと思えば レイアウト JSON 内のほぼ全ての項目を探索対象に含めることができます。

しかし、以下のパラメーターは探索対象から除外して、事前に決めた固定値を使用すべきです。
- 解像度 (`resolution`)
  - `hisui vmaf` コマンドでは「合成後の画像（参照画像）」と「それをエンコードした後の画像」を比較して VMAF スコアを計算します
  - つまり、あくまでも「エンコードによる画質劣化度合い」を求めているのであって、「合成（主にリサイズ）による画質劣化の度合い」は考慮されません
  - レイアウトの `resolution` の値が変わると、参照画像自体が変わることになるので、それをもとに計算された VMAF スコア同士を直接的に比較することはできません
  - ただし、解像度（とフレームレート）自体は合成処理時間や品質に大きく影響するパラメーターではあるので、用途に合わせた適切な値を指定することは重要です
- コーデック (`video_codec`)
  - `hisui vmaf` コマンドでは VMAF スコアを計算するために、合成画像をエンコードした後に再度デコードして生画像を取得しています
  - この追加のデコード処理のコストはコーデック（正確にはデコーダー）によって変わります
  - つまり、コーデックを探索すると、デコード処理が軽いコーデックの方が誤って優先されてしまうことになります

また、一般論として、探索対象のパラメーターが少ないほど探索効率が上がるので、
事前に適切な値が判明しているパラメーターは、値を固定して探索対象から除外するのが望ましいです。

### 探索結果から最終的に使用するパラメーターセット（レイアウト JSON）をどうやって選択するか

`hisui tune` コマンドは、処理時間と映像品質のトレードオフを考慮して、
最適解の集合（`BEST TRIALS`） の探索を行いますが、単一の最適解が得られる訳ではありません。

そのため、最適解の集合の中から、実際に使用するパラメーターセットを選択するのは利用者の役目となります。
具体的にどれを選択すべきかはケースバイケースとなるので一概には言えませんが、一例としては以下のような方法が考えられます。

- 映像品質をできるだけ優先したい場合は、VMAFスコアが一番高い結果を選択する
- 処理時間を優先したい場合には、処理時間が短い結果から順に映像品質を確認して、最初に許容可能となった結果を選択する

なお、VMAF スコアはあくまでも、映像品質を測るための指標のひとつに過ぎず、人間の感覚を完全に反映しているとは限らないので、
有望なパラメーターセットに関しては、実際に合成してみて結果を確認するのも重要です。

### 探索の実行環境や使用する録画ファイルについて

これらは可能な限り、実際の運用環境や録画ファイルの内容に合わせておくのが重要です。

例えば、探索用にダミーの静止画のような録画を使うと、
静止画向けのエンコードパラメーターが優先されてしまって、意味のない探索となってしまう可能性があります。

また探索時には、録画ファイルの先頭部分（`--frame-count` で指定したフレーム数分）のみが考慮されるので、
その部分が録画ファイル全体の傾向をよく反映しているようなものを選ぶことが望ましいです。

### 実際のエンコードビットレートや合成処理時間などを確認する方法

レイアウト JSON でビットレートの指定はできますが、エンコーダーがそれを厳密に守る保証はなく、
さまざまな理由で、実際のビットレートは、指定値よりも大きくなったり小さくなったりすることがあります。

また、各トライアルの処理時間は、相対値の比較には使えますが、絶対値の比較にはあまり意味がありません
（例えば、あるトライアルの処理時間が別のトライアルの半分だとしても、実際の合成処理時間も同様に半分になるとは限りません）。

これらのメトリクスの正確な値を知りたい場合には、
トライアルで使用されたレイアウトファイルを使って、実際に `hisui compose` コマンドを実行するのが確実です。

### 以前の探索の続きから再開したい場合

`ROOT_DIR` 引数や `--study-name` オプションの値を変えずに `hisui tune` コマンド
を実行した場合は、（Optuna の機能で）自動で前回の続きから探索が再開されます。

これは「 `--trial-count` で指定した回数のトライアルは完了したけど、もう少し探索行いたい」といった場合に便利です。

### 探索空間やレイアウトファイルを変更して探索を行いたい場合

上述の通り、デフォルトでは `hisui tune` コマンドは前回の探索結果を引き継ぎます。
ただし、探索空間などが変わった場合は引き継ぎではなく、一から探索を開始するのが望ましいです。
その場合は、`--study-name` オプションで異なる名前を指定することで、新しい探索が開始できます。

また、`--tune-working-dir` オプションで指定した作業ディレクトリ以下にある
Optuna のストレージファイル（`optuna.db`）を削除することでも、探索履歴がクリアできます。

### トライアル回数をどうすべきか

`--trial-count` オプションでトライアルの実行回数が指定できます。
この値が大きいほど、よりよいパラメーターセットが見つかる可能性が高くなりますが、探索に掛かる時間も長くなります。

「最適なトライアル回数が何か」はケースバイケースなので、一概には言えませんが、
デフォルトの 100 は多くの場合によく動作する値なので、そのまま使っても問題ありません
（内部で Optuna を使っているので、途中で Ctrl+C で中断したとしても、探索を簡単に再開できます）。

探索をどこで終わりにするか、のひとつの目安としては「最適解の集合がしばらく更新されなくなったら」というものがあります。

### トライアルが失敗した場合にどうすべきか

Optuna が提案したパラメーターセットの組み合わせによっては、
エンコーダーのバリデーションによってエラーになることがあります。

このような組み合わせの発生を、事前に完全に防止するのは難しいので、
発生頻度が稀なのであれば、気にせずに探索を継続して問題ありません。

もし発生頻度が高いようであれば、探索空間を見直した方がいいでしょう。
各トライアルの実行時には、実際に実行された `hisui vmaf` コマンドがターミナルに表示されるので、
それを参考にして、問題となったレイアウトファイルを修正しつつ `hisui vmaf` コマンドを直接叩けば、
具体的にどのパラメーターの組み合わせが失敗の原因になっているのかを特定できます。

また、もし常に失敗するようであれば、
Hisui や依存ライブラリー（例えば OpenH264 の動的ライブラリー）のビルドに失敗している可能性が高いです。

### 各トライアルの実行時間を短くしたい

実行環境やエンコーダーによっては、一回のトライアルの評価に長時間掛かることがあります。

その場合は `--frame-count` オプションで小さな値を指定すると、各トライアルの実行時間を短くすることができます。
ただし、VMAF スコア計算の際に参照できる映像フレームの数が減るため、品質評価結果の信頼度が下ることになります。

また大半のトライアルの実行は許容可能な時間内で終わるけれど、一部のトライアルだけ極端に長い場合には
`--trial-timeout` オプションを指定することで、そういったトライアルを途中で中断できるようになります。

### 実行済み探索の最適解集合のみを確認したい

`--trial-count 0` を指定することで、新たなトライアルを実行せずに既存の最適解集合のみを表示できます。
