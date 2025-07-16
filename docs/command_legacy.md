# `hisui legacy` コマンド

`hisui legacy` コマンドは、レガシー版 Hisui との互換性を維持するためのコマンドです。

このコマンドは、以前のバージョンの Hisui を使用していたユーザーが、新しいバージョンに移行する際に既存のスクリプトやワークフローを変更することなく使用できるように提供されています。

## 使用方法

```console
$ hisui legacy -h
Recording Composition Tool Hisui

Usage: hisui ... legacy --in-metadata-file <PATH> [OPTIONS]

Example:
  $ hisui legacy --in-metadata-file /path/to/report-$RECORDING_ID.json

Options:
  -h, --help                                    このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version                                 バージョン番号を表示します
      --verbose                                 警告未満のログメッセージも出力します
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
      --show-progress-bar <true|false>          true が指定された場合には合成の進捗を表示します [default: true]
  -c, --max-cpu-cores <INTEGER>                 合成処理を行うプロセスが使用するコア数の上限を指定します
      --out-stats-file <PATH>                   合成実行中に集めた統計情報 JSON の出力先ファイル
```

## 主な機能

### 録画メタデータファイルからの合成

`--in-metadata-file` オプションを使用して、Sora が生成した録画メタデータファイルから直接合成を実行できます。

```console
$ hisui legacy --in-metadata-file /path/to/report-RECORDING_ID.json
```

### レイアウトファイルの指定

`--layout` オプションを使用して、カスタムレイアウトファイルを指定できます。

```console
$ hisui legacy --layout /path/to/layout.json --in-metadata-file /path/to/report-RECORDING_ID.json
```

### 音声のみの合成

`--audio-only` オプションを使用して、音声のみを合成対象にできます。

```console
$ hisui legacy --in-metadata-file /path/to/report-RECORDING_ID.json --audio-only
```

## 新しい `compose` コマンドとの違い

`hisui legacy` コマンドは、レガシー版 Hisui との互換性維持を目的としているため、以下の点で新しい `compose` コマンドとは異なります：

- 録画メタデータファイル（`--in-metadata-file`）からの直接合成をサポート
- グリッドレイアウトのカラム数指定（`--max-columns`）をサポート
- より詳細なエンコードパラメータの調整オプションを提供
- 一部の廃止された引数についても警告を出力しつつ無視して実行

## 移行について

新しい Hisui では、`compose` コマンドの使用が推奨されています。`legacy` コマンドは互換性維持のために提供されていますが、将来的には削除される可能性があります。

新しい `compose` コマンドでは、より直感的な引数構造と、改善されたレイアウト機能を利用できます。詳細については [`hisui compose` コマンド](command_compose.md)のドキュメントをご参照ください。
