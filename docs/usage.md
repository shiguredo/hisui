# Hisui を利用してみる

## リリースされたビルド済みのバイナリを使用する

ビルド済みのバイナリを使用する場合は [Releases](https://github.com/shiguredo/hisui/releases) より環境に応じた最新のバイナリをダウンロードしてください。

```bash
curl -L https://github.com/shiguredo/hisui/releases/download/{ VERSION }/{ BINARY_NAME } -o hisui
chmod +x hisui
```

なお、自前でのビルドについては [ビルド方法](build.md) をご参照ください。

## Docker イメージを使用する

Docker を使用することで、環境構築なしで Hisui を利用することもできます。
詳細は [Docker を利用した Hisui の実行](docker.md) をご参照ください。

## 利用可能なコマンド一覧

Hisui は以下のコマンドを提供しています。目的に合った項目を参照してください。

- [`list-codecs`](command_list_codecs.md) - 利用可能なコーデックの一覧を表示するコマンド
- [`inspect`](command_inspect.md) - MP4 ファイルの詳細情報を取得するコマンド
- [`server`](obsws/PROTOCOL_STATUS.md) - OBS WebSocket 互換サーバーを起動するコマンド
- [`transcribe`](command_transcribe.md) - MP4 の音声を Whisper で文字起こしするコマンド (実験的機能)
