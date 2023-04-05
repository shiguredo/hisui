# レイアウト機能

## 用語

- X の倍数に丸める: 値が X の倍数になるよう数値を小さくなる方向に変更する(変更しなくてよい場合はしない)。

## 利用方法

--layout オプションでレイアウト設定を指定する JSON ファイルを指定して利用する。

例:

```
hisui --layout layout.json
```

## レイアウト設定

[Composing Video Recordings using Twilio Programmable Video - Twilio](https://www.twilio.com/docs/video/api/compositions-resource) に準ずる。
`audio_sources`, `audio_sources_excluded`, Region(後述) 内の `video_sources`, `video_sources_excluded` には、Sora の archive-*.json のパス/パターンを指定する。

### audio_sources

音声のソースとして用いる archive-*.json のパスの配列を指定する。相対パスの場合は、レイアウト設定を指定する JSON ファイルのあるパスからの相対パスを探す。

`*` をワイルドカードとして利用できる。

### audio_sources_excluded

audio_sources のうち除外するソースのパターンの配列を指定する。

`*` をワイルドカードとして利用できる。

### bitrate

目標 bitrate を指定する。

現状 video の bitrate 指定にそのまま利用している。 audio の分を引くべきか? Opus の場合は encode 前には明確にはわからない

現状 100 未満なら 100 にしている。 0 ないし未定義(キーがない)場合は適当に計算している。

## format

出力フォーマットを指定する。 `webm` か `mp4` を指定できる。

## resolution

映像の解像度を文字列で {幅}x{高さ} の形式で指定する。(例: "640x480", "1280x720")
幅、高さの最小値は 16。

現状 4 の倍数に丸める。
現状 3840x3840 まで許容している。

## trim

音声、映像のソースのすべてが存在しない時間間隔について、
`false` であれば 0 時間始まりのもののみ、`true` の場合はすべてについて出力からカットする。

## video_layout

映像の配置を指定する。

Region 名(string) と Region の内容(object) の複数個の組からなる。

### Region/Grid/Cell

Region は内部を Grid で指定される区画に区切られる。区切られた部分を Cell を呼ぶ。

#### Grid の決め方

Grid の行と列の最大値は `max_columns` と `max_rows` によって指定される。
ただし、 該当キーがない場合と 0 の場合は未指定となる。

Twilio だと 0 は許可していないが、現状 0 を未指定と同様に扱っている。

##### max_columns, max_rows が両方指定されていない場合

`cells_excluded` と `reuse` (後述) を考慮した最大同時ソース数が収まるように、かつ行と列の数がなるべく同じになるように、行と列の数を決定する。
ただし、行の数は列の数よりも 1 大きくてもよい。

##### max_columns, max_rows の片方のみ指定されている場合

`cells_excluded` と `reuse` (後述) を考慮した最大同時ソース数よりも指定されている数が等しいか大きい場合は、最大同時ソース数行x1列 ないし 1行x最大同時ソース数列の Grid となる。
それよりも大きい場合は、`max_columns` 行ないし `max_rows`列となり、列ないし行が必要なだけ追加される。

##### max_columns, max_rows が両方指定されている場合

`cells_excluded` と `reuse` (後述) を考慮した最大同時ソース数よりも `max_columns` * `max_rows` が等しいか小さい場合は、`max_columns` 行 x `max_rows` 列となる。

`max_columns` > `max_rows` の場合、
`max_columns` のほうが最大同時ソース数よりも大きいか等しい場合は 最大同時ソース数行x1列 となる。
そうでない場合は `max_columns` 行 ソースが収まる 列 となる。

`max_columns` <= `max_rows` の場合も、行と列が入れ替わった形で同様となる。

#### Cell のサイズ

Region は Grid によって Cell に分割される。Cell 間には枠線が 2 pixel 配置される。

Region のサイズ(`width` ないし `height`[後述]) が、動画のサイズ(`resolution` で決定される幅ないし高さ)と等しい場合は、Region の両端に枠線が入らない。そうでない場合は枠線が入る。
各 Cell のサイズは 4 の倍数となる (2 の倍数でもいいかもしれない)。
このため、たとえば 240x160 の動画だけの Region についてそれより大きな(640x480 など)ベースに対して配置する場合、スケールを発生させないようにするには 244x164 のRegion を指定する必要がある。
スケールのコストは高くないので、気にしなくてもいいのかもしれない。

これらを考慮して 1 つの Cell のサイズがなるべく大きくなるように Cell のサイズを決定する。


#### Cell の状態

Cell には次の状態が存在する:

- Fresh: まだ利用されていない
- Used: 現在利用されている
- Idel: 以前利用されていたが現在は利用されていない
- Excluded: `cells_excluded` で指定された Cell で利用されることはない

### cells_excluded

一番左上の Cell の index を 0 として、左から右、上から下の順で index が振られた Cell のうち、映像を表示しない index の配列を指定する。

### height

Region の高さを指定する。キーがない場合と 0 の場合は `y_pos` から決定される (resolution.height - y_pos)。 16 未満の場合、resolution.height からはみ出る場合はエラーとなる。

現状 2 の倍数に丸めている。

### max_columns

前述

### max_rows

前述

### reuse

Cell へのソースの配置の仕方を指定する。
現状 `video_sources` での指定の最初から順に配置していく。(`connection_id` 対応などすると変わる可能性がある)

- `none`: Fresh な Cell にのみ新しいソースを配置する
- `show_oldest`: Fresh な Cell に加えて Idle な Cell にも 新しいソースを配置する
- `show_newest` : さらに, Used な Cell の中で 新しいソースよりも Cell の持つソースの開始時間が前の Cell のうち、最小の終了時間のソースを持つ Cell に新しいソースを配置する

### video_sources

映像のソースとして用いる archive-*.json のパスの配列を指定する。相対パスの場合は、レイアウト設定を指定する JSON ファイルのあるパスからの相対パスを探す。

`*` をワイルドカードとして利用できる。

### video_sources_excluded

video_sources のうち除外するソースのパターンの配列を指定する。

`*` をワイルドカードとして利用できる。

### width

Region の幅を指定する。 キーがない場合と 0 の場合は `x_pos` から決定される (resolution.width - x_pos)。 16 未満の場合、resolution.width からはみ出る場合はエラーとなる。

現状 2 の倍数に丸めている。

### x_pos

Region の左上の位置の x を指定する。キーがない場合は 0。[0, resolution.width] の範囲でない場合はエラーとなる。

### y_pos

Region の左上の位置の y を指定する。キーがない場合は 0。[0, resolution.height] の範囲でない場合はエラーとなる。

### z_pos

Region の z 軸での位置を指定する。キーがない場合は 0。[-99, 99] の範囲でない場合はエラーとなる。

現状 同じ z_pos の Region が複数あった場合にどういう順序で描画されるかは未定義。

## ソース

### ソースからの WebM ファイルの探索

archive-*.json 中の `filename`, `file_path` の順に WebM ファイルを探索する。

どちらについても、

- 絶対パスの場合: その絶対パスのみを探索する
- 相対パスの場合: まずレイアウト設定ファイルのパスからの相対パスを探索し、見つからない場合レイアウト設定ファイルのパスに、相対パスの basename があるかどうか探索する

### 時間

archive-*.json の `start_time`, `stop_time` を利用する。プログラムでは double としてパースしている。

### ビデオソースの描画順序

現状 `video_sources` で定義された順序に、`video_sources_excluded`, `reuse` を考慮して描画される。

現状 `connection_id` はパースしているがソースの同一性の判定には利用していない。
