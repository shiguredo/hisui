# レイアウト機能

Hisui では JSON 形式のレイアウトファイルを使用することでより細かな合成の設定が可能です。
レイアウトファイルでは合成する音声/映像の制御や、合成時の映像の場所など自由に設定することができます。
ここではレイアウトで使用できるパラメータやレイアウト例について記載します。

## 用語

- X の倍数に丸める: 値が X の倍数になるよう数値を小さくなる方向に変更します(変更しなくてよい場合はしません)。

## 利用方法

`--layout` オプションでレイアウト設定を指定します。 レイアウト設定は JSON ファイルを指定して利用します。

実行コマンド例:

```
hisui --layout layout.json
```

## レイアウト例

```
{
  "audio_sources": [
    "./archive-*.json"
  ],
  "audio_sources_excluded": [
    "./archive-7VZRQBE3ZS5ES9BV5VGVF4TSTW.json"
  ],
  "video_layout": {
    "grid": {
      "video_sources": [
        "./archive-*.json"
      ]
    }
  },
  "trim": true,
  "format": "webm",
  "resolution": "640x360",
  "bitrate": 1000
}
```

レイアウトに設定しているパラメータの説明

このレイアウトはマス目状に映像を配置して合成します。
合成結果はスクリーンショットのようになります。

[![Image from Gyazo](https://i.gyazo.com/b9f94a258cacf6968a407c3d611345f3.png)](https://gyazo.com/b9f94a258cacf6968a407c3d611345f3)

それぞれのパラメータについて以下に説明します。

- `audio_sources` : 格納されているファイル全てを相対パスで指定します
- `audio_sources_excluded` : ファイルの中から音声を入れないファイルを指定します
- `video_layout` : 映像レイアウトをこの中に指定します
  - `grid` : 分かりやすい名称をつけてレイアウトを指定します
    - `video_sources` : 合成する映像ファイルを全て指定します
- `trim` : 音声も映像もない時に trim を有効にするかどうか指定します
- `format` : 出力するフォーマットを指定します
- `resolution` : 出力する映像のサイズを指定します
- `bitrate` : 出力する映像のビットレートを指定します

## レイアウト設定

Hisui レイアウトは [Composing Video Recordings using Twilio Programmable Video - Twilio](https://www.twilio.com/docs/video/api/compositions-resource) に準じます。
Hisui のレイアウトでは `audio_sources`, `audio_sources_excluded`, Region(後述) 内の `video_sources`, `video_sources_excluded` に、Sora の `archive-*.json` のパス/パターンを指定します。

## レイアウト用設定パラメータ

レイアウトで使用可能な設定パラメータについて以下に記載します。

### audio_sources

音声のソースとして用いる `archive-*.json` のパスの配列を指定します。相対パスの指定も可能です。 パスは JSON ファイルのあるパスからの相対パスを探します。

`*` をワイルドカードとして利用することができます。

`*` を使用する場合は Sora の録画データと同じディレクトリにレイアウトファイルがあること、Sora の録画データ以外のファイルが無いようにする必要があります。

### audio_sources_excluded

audio_sources のうち除外するソースのパターンの配列を指定します。

`*` をワイルドカードとして利用することができます。

`*` を使用する場合は Sora の録画データと同じディレクトリにレイアウトファイルがあること、Sora の録画データ以外のファイルが無いようにする必要があります。

### bitrate

目標 bitrate を指定します。

video の bitrate 指定に利用しています。

<!-- audio  の分を引くべきか? Opus の場合は encode 前には明確にはわからないため 100 未満なら 100 にしています。-->

0 ないし未定義(キーがない場合)は適宜計算しています。

## format

出力フォーマットを指定します。 `webm` か `mp4` を指定することができます。

## resolution

映像の解像度を文字列で {幅}x{高さ} の形式で指定します。(例: "640x480", "1280x720")
幅、高さの最小値は 16 です。

4 の倍数に丸めています。
最大は 3840x3840 まで許容しています。

## trim

音声、映像のソースのすべてが存在しない時間間隔について、
`false` であれば 0 時間始まりのもののみ、`true` の場合はすべてについて出力からカットします。

## video_layout

映像の配置を指定します。

Region 名(string) と Region の内容(object) の複数個の組から構成されます。

### Region/Grid/Cell

Region は映像を配置するスペースです。その内部を Grid で指定される区画に区切られます。区切られた部分を Cell と呼びます。

#### Grid の決め方

Grid の行と列の最大値は `max_columns` と `max_rows` によって指定されます。
ただし、 該当キーがない場合と 0 の場合は未指定となります。

Twilio では 0 は許可していませんが、 Hisui では 0 を未指定と同様に扱っています。

##### max_columns, max_rows が両方指定されていない場合

`cells_excluded` と `reuse` (後述) を考慮した最大同時ソース数が収まるように、かつ行と列の数がなるべく同じになるように、行と列の数を決定します。
ただし、行の数は列の数よりも 1 大きくても問題ありません。

##### max_columns, max_rows の片方のみ指定されている場合

`cells_excluded` と `reuse` (後述) を考慮した最大同時ソース数よりも指定されている数が等しいか大きい場合は、最大同時ソース数行 x1 列 ないし 1 行 x 最大同時ソース数列の Grid となります。
それよりも大きい場合は、`max_columns` 行ないし `max_rows`列となり、列ないし行が必要なだけ追加されます。

##### max_columns, max_rows が両方指定されている場合

`cells_excluded` と `reuse` (後述) を考慮した最大同時ソース数よりも `max_columns` x `max_rows` が等しいか小さい場合は、`max_columns` 行 x `max_rows` 列となります。

`max_columns` > `max_rows` の場合、
`max_columns` のほうが最大同時ソース数よりも大きいか等しい場合は 最大同時ソース数行 x1 列 となります。
そうでない場合は `max_columns` 行 ソースが収まる 列 となります。

`max_columns` <= `max_rows` の場合も、行と列が入れ替わった形で同様となります。

#### Cell のサイズ

Region は Grid によって Cell に分割されます。Cell 間には枠線が 2 pixel 配置されます。

Region のサイズ(`width` ないし `height`[後述]) が、動画のサイズ(`resolution` で決定される幅ないし高さ)と等しい場合は、Region の両端に枠線が入りません。そうでない場合は枠線が入ります。
各 Cell のサイズは 4 の倍数となります。 <!-- (2 の倍数でもいいかもしれない) -->
このため、たとえば 240x160 の動画だけの Region についてそれより大きな(640x480 など)ベースに対して配置する場合、スケールを発生させないようにするには 244x164 の Region を指定する必要があります。

<!-- (スケールのコストは高くないので、気にしなくてもいいのかもしれない。) -->

これらを考慮して 1 つの Cell のサイズがなるべく大きくなるように Cell のサイズを決定します。

#### Cell の状態

Cell には次の状態が存在します:

- Fresh: まだ利用されていない状態
- Used: 現在利用されている状態
- Idel: 以前利用されていたが現在は利用されていない状態
- Excluded: `cells_excluded` で指定された Cell で利用されることはありません

### cells_excluded

一番左上の Cell の index を 0 として、左から右、上から下の順で index が振られた Cell のうち、映像を表示しません index の配列を指定します。

### height

Region の高さを指定します。キーがない場合と 0 の場合は `y_pos` から決定されます (resolution.height - y_pos)。 16 未満の場合、resolution.height からはみ出る場合はエラーとなります。

2 の倍数に丸めています。

### max_columns

前述

### max_rows

前述

### reuse

Cell へのソースの配置の仕方を指定します。
`video_sources` での指定の最初から順に配置していきます。(`connection_id` 対応などすると変わる可能性があります)

- `none`: Fresh な Cell にのみ新しいソースを配置します
- `show_oldest`: Fresh な Cell に加えて Idle な Cell にも 新しいソースを配置します
- `show_newest` : さらに, Used な Cell の中で 新しいソースよりも Cell の持つソースの開始時間が前の Cell のうち、最小の終了時間のソースを持つ Cell に新しいソースを配置します

### video_sources

映像のソースとして用いる `archive-*.json` のパスの配列を指定します。相対パスの場合は、レイアウト設定を指定します。 JSON ファイルのあるパスからの相対パスを探すようになっています。

`*` をワイルドカードとして利用できます。

`*` を使用する場合は Sora の録画データと同じディレクトリにレイアウトファイルがあること、Sora の録画データ以外のファイルが無いようにする必要があります。

### video_sources_excluded

video_sources のうち除外するソースのパターンの配列を指定します。

`*` をワイルドカードとして利用できます。

`*` を使用する場合は Sora の録画データと同じディレクトリにレイアウトファイルがあること、Sora の録画データ以外のファイルが無いようにする必要があります。

### width

Region の幅を指定します。 キーがない場合と 0 の場合は `x_pos` から決定されます (resolution.width - x_pos)。 16 未満の場合、resolution.width からはみ出る場合はエラーとなります。

2 の倍数に丸めています。

### x_pos

Region の左上の位置の x を指定します。キーがない場合は 0。[0, resolution.width] の範囲でない場合はエラーとなります。

### y_pos

Region の左上の位置の y を指定します。キーがない場合は 0。[0, resolution.height] の範囲でない場合はエラーとなります。

### z_pos

Region の z 軸での位置を指定する。キーがない場合は 0。[-99, 99] の範囲でない場合はエラーとなります。

同じ z_pos の Region が複数あった場合にどういう順序で描画されるかは未定義です。

## ソース

### ソースからの WebM ファイルの探索

`archive-*.json` 中の `filename`, `file_path` の順に WebM ファイルを探索します。

どちらについても、

- 絶対パスの場合: その絶対パスのみを探索します
- 相対パスの場合: まずレイアウト設定ファイルのパスからの相対パスを探索し、見つからない場合レイアウト設定ファイルのパスに、相対パスの basename があるかどうか探索します

### 時間

合成する時間は ` archive-*.json` の `start_time`, `stop_time` を利用します。プログラムでは double としてパースしています。

### ビデオソースの描画順序

`video_sources` で定義された順序に、`video_sources_excluded`, `reuse` を考慮して描画されます。

`connection_id` はパースしているがソースの同一性の判定には利用していません。
