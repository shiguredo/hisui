# Hisui を利用してみる

## 注意

公開されているビルド済みバイナリは FDK-AAC には対応していません。FDK-AAC を利用する場合は自前でのビルドを行ってください。

自前でのビルドについては [ビルド方法](build.md) をご参照ください。

## リリースされたビルド済みのバイナリを使用して利用する

ビルド済みのバイナリを使用する場合は [Releases](https://github.com/shiguredo/hisui/releases) より環境に応じた最新のバイナリをダウンロードしてください。

```sh
$ curl -L https://github.com/shiguredo/hisui/releases/download/{ VERSION }/{ BINARY_NAME } -o hisui
$ chmod +x hisui
```

Hisui には録画ファイルの合成を行うための [compose](command_compose.md) コマンドがあります。
Sora が録画ファイルを保存したディレクトリを指定して、このコマンドを実行すると合成が始まります。

```sh
// 録画ディレクトリを確認
$ ls 録画ファイルの配置ディレクトリ/
report-{ RECORDING_ID }.json
archive-{ CONNECTION_ID }.json
archive-{ CONNECTION_ID }.mp4
...

// 合成を実行
$ ./hisui compose 録画ファイルの配置ディレクトリ/

// 合成結果ファイルを確認
$ ls 録画ファイルの配置ディレクトリ/output.mp4
```

## 好きなレイアウトやコーデックで合成したい

Hisui にはレイアウトという機能があり、そちらを利用することでより自由な合成が可能です。

もし、より複雑な合成を試されたい場合はぜひレイアウト機能を試してみてください。

詳細は [レイアウト機能](LAYOUT.md) のドキュメントをご参照ください。
