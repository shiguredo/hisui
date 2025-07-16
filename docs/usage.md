# Hisui を利用してみる

## リリースされたビルド済みのバイナリを使用して合成する

ビルド済みのバイナリを使用する場合は [Releases](https://github.com/shiguredo/hisui/releases) より環境に応じた最新のバイナリをダウンロードしてください。

```console
$ curl -L https://github.com/shiguredo/hisui/releases/download/{ VERSION }/{ BINARY_NAME } -o hisui
$ chmod +x hisui
```

Hisui には録画ファイルの合成を行うための [compose](command_compose.md) コマンドがあります。
Sora が録画ファイルを保存したディレクトリを指定して、このコマンドを実行すると合成が始まります。

```console
# 録画ディレクトリを確認
$ ls 録画ファイルの配置ディレクトリ/
report-{ RECORDING_ID }.json
archive-{ CONNECTION_ID }.json
archive-{ CONNECTION_ID }.mp4
...

# 合成を実行
$ ./hisui compose 録画ファイルの配置ディレクトリ/

# 合成結果ファイルを確認
$ ls 録画ファイルの配置ディレクトリ/output.mp4
```

自前でのビルドについては [ビルド方法](build.md) をご参照ください。

## 好きなレイアウトで合成したい

Hisui にはレイアウトという機能があり、そちらを利用することでより自由な合成が可能です。

`compose` コマンドでレイアウト指定を省略した場合には、
入力ストリーム群をグリッド上に配置するデフォルトレイアウトが使用されます。

もし、より複雑な合成を試されたい場合はぜひレイアウト機能を試してみてください。

詳細は [レイアウト機能](layout.md) のドキュメントをご参照ください。

## コーデックやエンコードパラメーターを指定して合成したい

Hisui では合成時に使用するエンコードコーデックやエンコードパラメーターを変更することが可能です。
これらを指定することで、用途に応じて、変換時間と画質のどちらを優先するか、などを細かく制御できます。

指定方法の詳細は [エンコードコーデックやパラメーターの指定](encode.md) のドキュメントをご参照ください。

また [tune](command_tune.md) コマンドを利用することで、エンコードパラメーターの自動調整を行うことができます。

## 利用可能なコマンド一覧

- [`compose`](command_compose.md) - Sora が保存した録画ファイルを合成するためのコマンド
- [`legacy`](command_legacy.md) - レガシー版 Hisui との互換性維持用コマンド
- [`list-codecs`](command_list_codecs.md) - 利用可能なコーデックの一覧を表示するコマンド
- [`tune`](command_tune.md) - 映像エンコードパラメーターの最適化を行うコマンド
- [`vmaf`](command_vmaf.md) - 録画ファイルの品質評価（VMAF スコア計算）を行うコマンド
- [`inspect`](command_inspect.md) - 録画ファイルの詳細情報を取得するコマンド
