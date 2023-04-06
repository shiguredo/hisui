## FAQ

### 全般
### 分割録画した映像を合成できますか

Hisui は現在分割録画ファイルからの合成に対応していません。
Sora の録画ファイルは単一録画ファイルを生成してご利用ください。

### Windows の VirtualBox 上で Hisui を実行するとエラーが発生しました

Windows の VirtualBox 上で Hisui を実行した際にエラーが発生した事象が確認されています。
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

可能です。3 種類の方法があります。

- `--screen-capture-report` を利用するケース

    複数の Channel を使用した場合はこのオプションを使用すると便利です。

    例えば会議メンバーが参加している channel とは別に画面共有だけの channel を用意して配信し録画した場合の合成に役立ちます。

    `--screen-capture-report` を使用することで画面共有中の間だけ、他の音声はそのままに画面共有の画面だけを表示して合成をすることができます。

    実行コマンド例

    大きく表示したい映像の report ファイルを指定します。

    `./hisui -f .../report-2A0EVXFRVS7BSCEV4EQJ3MW4VC.json --screen-capture-report .../report-GACVGHQB953FX8GFG98AY3XGXR.json  --out-file test.mp4`

- `--screen-capture-connection-id` を利用するケース

    channel に接続している映像のうち特定の connection だけ大きくしたい場合などには `--screen-capture-connection-id` が便利です。

    同一 channel で画面共有をした場合、途中から画面共有をした映像を表示したい場合の合成に役立ちます。

    実行コマンド例

    大きく表示したい映像の connection id を指定します。

    `./hisui -f .../report-4Z3KF8X4GH1G75SWSVA2YAZ65R.json --screen-capture-connection-id JVNGWZB23124NCH76ZRV67HXV8 --out-file test.mp4`

- レイアウトファイルを利用するケース

    レイアウトファイルにて `video_sources` で表示しておきたい映像を指定することで、対象としていない映像は合成されません。

    この方法では合成データに指定した映像しか含まれないため、複数の映像を合成したい場合はご注意ください。

### Hisui で作成した VP9/AAC の MP4 を再生することは可能ですか

可能です。以下に再生可能な環境を記載します。

- Safari / Chrom / Edge / Firefox などのブラウザでの再生
- Windows10 標準アプリの 映画 & テレビ、 Windows11 の標準アプリのメディアプレイヤーでの再生

### Hisui を実行したらエラーになってしまいました

対象のファイルがないなど、様々な原因が考えられます。以下によくあるエラーを記載します。

| エラーメッセージ | エラー内容 | 解決例 |
| --- | --- | --- |
| `--in-metadata-file: 1 required TEXT:FILE missing` | 合成対象のメタデータが見つかりません | `-f` オプションで指定した `recording.report` の指定と場所を確認してみてください |
| `[error] setting up muxer failed:` | ファイル指定ミス | `-f` オプションで指定したファイルの場所を確認してみてください。分割録画だけの録画データを対象にしていないか確認してみてください |
| `[error] muxing failed: Unable to open:` | 指定した場所が見つからない | `--out-file` などのオプションで指定した場所を確認してみてください |
| `[error] setting up muxer failed: file is not found:` | archive-hoge.webm が見つからない | report-hoge.json で指定されているファイルを確認してみてください |
| `--layout: 1 required TEXT:FILE missing` | レイアウトファイル指定ミス | レイアウトファイルの指定している場所を確認してみてください |
| `[error] parsing layout metadata failed: pattern` | レイアウトファイル内のエラー | メッセージに出力されている内容を確認してレイアウトの設定を見直してみてください |
| `[error] parsing audio_source(./hoge/archive-hoge.json) failed: filename() and file_path() do not exsit` | JSON 指定のファイルが見つからない | レイアウトで指定しているファイルの webm が存在しているか確認してみてください |

### Hisui で合成をキャンセルしたときに mdatXXXX というファイルが生成されました

mdatXXXX は Hisui が合成をするときに作成する中間ファイルです。

`--mp4-muxer` が `Faststart` に設定されているときに生成されます。Hisui はデフォルトで `Faststart` になっているため、`--mp4-muxer Simple` とオプションを設定しない限り生成されます。

`--dir-for-faststart` オプションで mdatXXXX を生成する場所を指定することが可能です。デフォルトでは合成ファイルを出力する場所に作成されるようになっています。

### help を表示した時に出ていないオプションがあるようです

Hisui の help はビルド方法によってヘルプの内容が変化します。

例えば `--use-fdk-aac` オプションを使用してビルドを行うことで `--out-audio-codec` が表示されるようになります。

また、`--build-type-debug` オプションを使用してビルドすることで、`--out-video-bit-rate`,`--libvpx-threads`, `--libvp9-frame-parallel` といったチューニング関連のヘルプが表示されるようになります。

### コーデック
### 音声コーデックに AAC を指定できますか

Hisui では --use-fdk-aac オプションを使用して自前でビルドをすることで有効にすることが可能です。

詳細は [--use-fdk-aac を有効にしたバイナリをビルドする](BUILD_LINUX.md) を参照してください。

### AAC を使った合成をしたい

Docker や リリースバイナリではなく Hisui をご自身でビルドすることで利用が可能です。
ビルドについては [BUILD_LINUX](BUILD_LINUX.md) をご参照ください。

### H.264 を使った合成をしたい

H.264 の合成をする場合は OpenH264 を用意した上で --openh264 でライブラリファイルを指定してください

### OpenH264 を指定してもエラーになってしまいました

まず `--openh264` で指定しているライブラリのパスと権限に誤りがないことを確認してください。

上記のことを確認した上で動作しない場合 Ubuntu のバージョンと openh264 の組み合わせによって動作しないことがありますので、以下のようにすることで解決する可能性があります。

- Ubuntu20.04 をご利用の方

openh264 の Version 2.3.1 では openh264 で要求されるライブラリのバージョンを満たしていないため Version 2.3.0 をご利用ください。

- Ubuntu22.04 をご利用の方

openh264 で要求されるライブラリのバージョンを満たしています。Version 2.3.1 以上が動作するので

取得した openh264 のバイナリがご利用の OS とあっているかご確認ください。

### レイアウト
### レイアウトを途中で切り替えることは可能ですか

可能です。以下に記載するようなケースで利用が可能です。

- 会議などで画面共有をした時にレイアウトを変更したい
- 途中参加したタイミングでレイアウトを変更したい

レイアウトの変更では録画データが持っている `start_time` と `stop_time` が重要になります。全ての録画データが同一スタートである場合は途中で切り替えることはできません。

詳細は [レイアウト機能](LAYOUT.md) のドキュメントを参照してください。

### 特定のメンバーだけ目立つようにレイアウトを指定することは可能ですか

可能です。レイアウトでは `video_layout` によって `video_sources` をどのように配置するか設定することが可能です。

詳細は [レイアウト機能](LAYOUT.md) のドキュメントを参照してください。

### video_sources や audio_sources の指定で "*" を指定することは可能ですか

可能です。ただし以下の条件があります。

- もしレイアウトファイル、Sora が生成する録画関連のファイル以外のファイルが存在する場合 `excluded` を使用して除外する必要があります
- layout ファイルは録画データと同じ場所にいる必要があります

別の指定方法として `<recording_id>/archive-*.json` を使用することで、`excluded` を指定することなく全てのファイルを対象にすることが可能ですのでこちらを利用することも検討してみてください。

### 一部の音声ファイルや録画ファイルを対象外にすることは可能ですか

対象外にしたいファイルを `excluded` で指定することで可能です。

### レイアウトのサンプルはどこかにありますか

基本的には [Composing Video Recordings using Twilio Programmable Video - Twilio](https://www.twilio.com/docs/video/api/compositions-resource) に準じます。

一例を [レイアウト機能](https://github.com/shiguredo/hisui/blob/develop/doc/LAYOUT.md) に記載していますので、そちらと合わせてご参照ください。

### エンコード
### 複数の CPU コアを用いて映像をエンコードできますか

Hisui で利用している [libvpx](https://github.com/webmproject/libvpx/) には、
いくつかのマルチスレッドを利用する機能がありますが、
Hisui のデフォルトでは off にしています。
Hisui が生成するそれほど解像度の高くない映像には、あまり効果が見られないためです。

Hisui は次のオプションを用意しています。 `--libvpx` で始まるものは VP8/VP9 共通で、 `--libvp9` で始まるものは VP9 の場合にしか効果がありません。

これらのオプションを help で確認したい場合、`--build-type-debug` オプションをつけて Hisui をビルドしてください。

- `--libvpx-threads`
    - 映像のエンコードに利用するスレッド数を指定します. Hisui でのデフォルトは 0 でマルチスレッド機能は無効です。
- `--libvp9-tile-columns`
    - 複数スレッドでのエンコーディングのために映像の列単位での分割を指定します。 2^(指定した数) の分割が行なわれます。 Hisui でのデフォルトは 0 です。
- `--libvp9-row-mt`
    - 1 を指定すると行ベースの非決定的マルチスレディングが有効になります。 Hisui でのデフォルトは 0 で無効です。

[Recommended Settings for VOD Media  |  Google Developers](https://developers.google.com/media/vp9/settings/vod) の "Tiling and Threading Recommendations" が
`threads` と `tile-columns` を決定する参考になります。

解像度の高くない映像でも `gamemoderun` コマンドを利用すると, マルチスレッドでのエンコードが高速化する場合がありました。
Ubuntu の場合は次でインストールされます。

```
sudo apt install gamemode
```

`gamemoderun` やコマンドラインオプションを組合せて録画合成する例を示します。

```
gamemoderun ./hisui -f ~/report-XXX.json --libvpx-threads 4 --libvp9-tile-columns 2 --libvp9-row-mt 1
```

#### 参考

- [The WebM Project | VP8 Encode Parameter Guide](https://www.webmproject.org/docs/encoder-parameters/)
- [Recommended Settings for VOD Media  |  Google Developers](https://developers.google.com/media/vp9/settings/vod)
- [mrintrepide/VP9 Encode Guide.md](https://gist.github.com/mrintrepide/3033c35ee9557e66cff7806f48dbd339)
- [VP9 Encoding Guide - wiki](http://wiki.webmproject.org/ffmpeg/vp9-encoding-guide)
- [FeralInteractive/gamemode: Optimise Linux system performance on demand](https://github.com/FeralInteractive/gamemode)

