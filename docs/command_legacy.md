# `hisui legacy` コマンド

`hisui legacy` コマンドは、 [レガシー版 Hisui] とコマンドライン引数の互換性維持を目的としたコマンドです。

[レガシー版 Hisui]: https://github.com/shiguredo/hisui-legacy

このコマンドは、以前のバージョンの Hisui を使用していたユーザーが、
新しいバージョンに移行する際に既存のスクリプトやワークフローをできるだけ変更することなく使用できるように提供されています。

新規に Hisui を使う場合には、代わりに [`hisui compose`](command_compose.md) コマンドを使用してください。

## 注意事項

`hisui legacy` コマンドも [レガシー版 Hisui] と 100% の互換性がある訳ではありません。
詳しい差異については [hisui_legacy\.md](hisui_legacy.md) をご確認ください。

また `hisui legacy` コマンドは将来的に削除される可能性があるため、 [`hisui compose`](command_compose.md) への移行を推奨しています。

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
  -T, --worker-threads <INTEGER>                合成処理に使用するワーカースレッド数を指定します [env: HISUI_WORKER_THREADS] [default: 1]
      --out-stats-file <PATH>                   合成実行中に集めた統計情報 JSON の出力先ファイル
      --video-codec-engines                     OBSOLETE: 2025.1.0 以降では指定しても無視されます
      --mp4-muxer <IGNORED>                     OBSOLETE: 2025.1.0 以降では指定しても無視されます
      --dir-for-faststart <IGNORED>             OBSOLETE: 2025.1.0 以降では指定しても無視されます
      --out-container <IGNORED>                 OBSOLETE: 2025.1.0 以降では指定しても無視されます
      --h264-encoder <IGNORED>                  OBSOLETE: 2025.1.0 以降では指定しても無視されます
```
