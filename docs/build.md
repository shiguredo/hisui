# ビルド方法

## ビルドに必要な依存パッケージのインストール

Ubuntu の場合には以下のようにして、ビルドに必要なパッケージをインストールしてください。

```
$ sudo apt-get install -y meson ninja-build nasm yasm build-essential autoconf automake libtool pkg-config yasm cmake
```

## Hisui 本体のビルド方法

Hisui は Rust のビルドツールである [Cargo](https://doc.rust-lang.org/cargo/) を使って以下のようにビルドします。

```console
// crates.io からビルドする場合（まだ canary リリースしかないのでバージョン指定が必須）
$ cargo install hisui@2025.1.0-canary.0

// リポジトリ指定でビルドする場合
$ cargo install --git https://github.com/shiguredo/hisui.git

// ローカルに clone してからビルドする場合
$ git clone https://github.com/shiguredo/hisui.git
$ cd hisui/
$ cargo install --path .
```

FDK-AAC を使った AAC エンコードを行う場合には `--features fdk-aac` の指定が必要になります。

```console
$ cargo install hisui@2025.1.0-canary.0 --features fdk-aac
```

## ビルド結果の確認方法

`hisui -h` を実行してみてください。

```console
$ hisui -h
Recording Composition Tool Hisui

Usage: hisui --in-metadata-file <PATH> [OPTIONS]

Example:
  $ hisui --in-metadata-file /path/to/report-$RECORDING_ID.json

Options:
  -h, --help                                    このヘルプメッセージを表示します
      --version                                 バージョン番号を表示します
      --codec-engines                           利用可能なエンコーダ・デコーダの一覧を JSON 形式で表示します
  -f, --in-metadata-file <PATH>                 Sora が生成した録画メタデータファイルを指定して合成を実行します
      --layout <PATH>                           Hisui のレイアウトファイルを指定して合成を実行します
      --out-file <PATH>                         合成結果を保存するファイルのパス
      --out-video-codec <VP8|VP9|H264|H265|AV1> 映像のエンコードコーデック [default: VP9]
      --out-audio-codec <Opus|AAC>              音声のエンコードコーデック [default: Opus]
      --out-video-frame-rate <INTEGER|RATIONAL> 合成後の映像のフレームーレート [default: 25]
      --max-columns <POSITIVE_INTEGER>          入力映像を配置するグリッドの最大カラム数 [default: 3]
      --audio-only                              音声のみを合成対象にします
      --openh264 <PATH>                         OpenH264 の共有ライブラリのパス
      --libvpx-cq-level <NON_NEGATIVE_INTEGER>  libvpx のエンコードパラメータ [default: 30]
      --libvpx-min-q <NON_NEGATIVE_INTEGER>     libvpx のエンコードパラメータ [default: 10]
      --libvpx-max-q <NON_NEGATIVE_INTEGER>     libvpx のエンコードパラメータ [default: 50]
      --out-opus-bit-rate <BPS>                 Opus でエンコードする際のビットレート [default: 65536]
      --out-aac-bit-rate <BPS>                  AAC でエンコードする際のビットレート [default: 64000]
      --show-progress-bar <true|false>          `true` が指定された場合には合成の進捗を表示します [default: true]
      --verbose                                 警告未満のログメッセージも出力します
  -c, --cpu-cores <INTEGER>                     合成処理を行うプロセスが使用するコア数の上限を指定します
      --out-stats-file <PATH>                   合成実行中に集めた統計情報 JSON の出力先ファイル
      --video-codec-engines                     OBSOLETE: 2025.1.0 以降では指定しても無視されます
      --mp4-muxer <IGNORED>                     OBSOLETE: 2025.1.0 以降では指定しても無視されます
      --dir-for-faststart <IGNORED>             OBSOLETE: 2025.1.0 以降では指定しても無視されます
      --out-container <IGNORED>                 OBSOLETE: 2025.1.0 以降では指定しても無視されます
      --h264-encoder <IGNORED>                  OBSOLETE: 2025.1.0 以降では指定しても無視されます
```
