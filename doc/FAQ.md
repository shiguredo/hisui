## FAQ

### 全般
### 分割録画した映像を合成できますか

hisui は現在分割録画ファイルからの合成に対応していません。
Sora の録画ファイルは単一録画ファイルを生成するようにしてご利用ください。

### Windows の VirtualBox 上で hisui を実行するとエラーが発生しました

Windows の VirtualBox 上で hisui を実行した際にエラーが発生した事象が確認されています。
Hyper-V を無効にすることで事象が解消されることを確認しています。

#### 参考
https://forums.virtualbox.org/viewtopic.php?f=6&t=101917

### 合成データを出力する場所を指定することは可能ですか

可能です。 `--out-file` を使用して指定します。このオプションの有無や指定方法によって出力方法も変わりますので以下に記載します。 

| layout | `--out-file` | パス指定 | ファイル名 | 結果 |
| --- | --- | --- | --- | --- |
| 無 | 無 | 無 | 無 | report ファイルの格納場所に report ファイルと同じ名前で出力 |
| 無 | 有 | 無 | 有 | report ファイルの格納場所に指定した名前で出力 |
| 無 | 有 | 有 | 有 | 指定した場所に指定した名前で出力 |
| 有 | 無 | 無 | 無 | layout ファイルの格納場所に layout ファイルと同じ名前で出力 |
| 有 | 有 | 無 | 有 | layout ファイルの格納場所に指定した名前で出力 |
| 有 | 有 | 有 | 有 | 指定した場所に指定した名前で出力 |

### 特定の映像だけを表示するような合成にすることは可能ですか

可能です。2種類の方法があります。

- `--screen-capture-report` を利用するケース

複数の Channel を使用した場合はこのオプションを使用すると便利です。

例えば会議メンバーが参加している channel とは別に画面共有だけの channel を用意して配信し録画した場合の合成に役立ちます。

`--screen-capture-report` を使用することで画面共有中の間だけ、画面共有の画面が全面になる合成を作ることができます。

実行コマンド例

大きく表示したい映像の report ファイルを指定します。

`./hisui -f .../report-2A0EVXFRVS7BSCEV4EQJ3MW4VC.json --screen-capture-report .../report-GACVGHQB953FX8GFG98AY3XGXR.json  --out-file test.mp4`

- `--screen-capture-connection-id` を利用するケース

channel に接続している映像のうち特定の connection だけ大きくしたい場合などには `--screen-capture-connection-id` が便利です。

同一 channel で画面共有をした場合、途中から説明をされた方を大きく表示したい場合の合成に役立ちます。

実行コマンド例

大きく表示したい映像の connection id を指定します。

`./hisui -f .../report-4Z3KF8X4GH1G75SWSVA2YAZ65R.json --screen-capture-connection-id JVNGWZB23124NCH76ZRV67HXV8 --out-file test.mp4`

### コーデック
### 音声コーデックに AAC を指定できますか

hisui では --use-fdk-aac オプションを使用して自前でビルドをすることで有効にすることが可能です。

詳細は [--use-fdk-aac を有効にしたバイナリをビルドする
](https://github.com/shiguredo/hisui/blob/develop/doc/BUILD_LINUX.md#--use-fdk-aac-%E3%82%92%E6%9C%89%E5%8A%B9%E3%81%AB%E3%81%97%E3%81%9F%E3%83%90%E3%82%A4%E3%83%8A%E3%83%AA%E3%82%92%E3%83%93%E3%83%AB%E3%83%89%E3%81%99%E3%82%8B) を参照してください。

### レイアウト
### レイアウトを途中で切り替えることは可能ですか

可能です。以下に記載するようなケースで利用が可能です。

- 会議などで画面共有をした時にレイアウトを変更したい
- 途中参加したタイミングでレイアウトを変更したい

レイアウトの変更では録画データが持っている `start_time` と `stop_time` が重要になります。全ての録画データが同一スタートであったりする場合は途中で切り替えることはできません。

### 特定のメンバーだけ目立つようにレイアウトを指定することは可能ですか

可能です。レイアウトでは `video_layout` によって `video_sources` をどのように配置することが可能です。

詳細は [レイアウト機能](https://github.com/shiguredo/hisui/blob/develop/doc/LAYOUT.md) のドキュメントを参照してください。

### video_sources や audio_sources の指定で "*" を指定することは可能ですか

可能です。ただし以下の条件があります。

- `excluded` を使用して `archive-*.json` 以外のファイルを除外する必要があります
- layout ファイルは録画データと同じ場所にいる必要がああります

別の指定方法として `<recording_id>/archive-*.json` を使用することで、`excluded` を指定することなく全てのファイルを対象にすることが可能ですのでこちらを利用することも検討してみてください。

### 一部の録画データを対象外にすることは可能ですか

可能です。２種類の方法があります。

- layout を使用して `excluded` を指定する
- layout を使用せず `report-hoge.json` ファイルから対象にしたい録画の部分を削除する

### レイアウトのサンプルはどこかにありますか

基本的には [Composing Video Recordings using Twilio Programmable Video - Twilio](https://www.twilio.com/docs/video/api/compositions-resource) に準じます。

一例を [レイアウト機能](https://github.com/shiguredo/hisui/blob/develop/doc/LAYOUT.md) に記載していますので、そちらと合わせてご参照ください。

### エンコード
### 複数の CPU コアを用いて映像をエンコードできますか

hisui で利用している [libvpx](https://github.com/webmproject/libvpx/) には, 
いくつかのマルチスレッドを利用する機能がありますが,
hisui のデフォルトでは off にしています.
hisui が生成するそれほど解像度の高くない映像には, あまり効果が見られないためです.

hisui は次のオプションを用意しています. `--libvpx` で始まるものは VP8/VP9 共通で, `--libvp9` で始まるものは VP9 の場合にしか効果がありません.

- `--libvpx-threads`
    - 映像のエンコードに利用するスレッド数を指定します. hisui でのデフォルトは 0 でマルチスレッド機能は無効です.
- `--libvp9-tile-columns`
    - 複数スレッドでのエンコーディングのために映像の列単位での分割を指定します. 2^(指定した数) の分割が行なわれます. hisui でのデフォルトは 0 です.
- `--libvp9-row-mt`
    - 1 を指定すると行ベースの非決定的マルチスレディングが有効になります. hisui でのデフォルトは 0 で無効です.

[Recommended Settings for VOD Media  |  Google Developers](https://developers.google.com/media/vp9/settings/vod) の "Tiling and Threading Recommendations" が
`threads` と `tile-columns` を決定する参考になります.

解像度の高くない映像でも `gamemoderun` コマンドを利用すると, マルチスレッドでのエンコードが高速化する場合がありました.
Ubuntu の場合は次でインストールされます. 

```
sudo apt install gamemode
```

`gamemoderun` やコマンドラインオプションを組合せて録画合成する例を示します.

```
gamemoderun ./hisui -f ~/report-XXX.json --libvpx-threads 4 --libvp9-tile-columns 2 --libvp9-row-mt 1
```

#### 参考

- [The WebM Project | VP8 Encode Parameter Guide](https://www.webmproject.org/docs/encoder-parameters/)
- [Recommended Settings for VOD Media  |  Google Developers](https://developers.google.com/media/vp9/settings/vod)
- [mrintrepide/VP9 Encode Guide.md](https://gist.github.com/mrintrepide/3033c35ee9557e66cff7806f48dbd339)
- [VP9 Encoding Guide - wiki](http://wiki.webmproject.org/ffmpeg/vp9-encoding-guide)
- [FeralInteractive/gamemode: Optimise Linux system performance on demand](https://github.com/FeralInteractive/gamemode)

### AAC を使った合成をしたい

hisui を自前でビルドすることで利用が可能です。
ビルドについては [BUILD_LINUX](BUILD_LINUX.md) をご参照ください。
