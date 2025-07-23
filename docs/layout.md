# レイアウト機能

Hisui では、レイアウトを定義した JSON ファイルを使うことで、複数映像ソースを合成する際の配置を細かく指定できます。
また、レイアウト JSON では、映像ソースの配置以外に、映像や音声のエンコード設定も指定できます。

[`hisui compose`](./compose_command.md) コマンドでレイアウトを使用する際は、
`-l` オプションでレイアウトファイルを指定します：

```console
$ hisui compose -l /path/to/layout.json /path/to/archive/RECORDING_ID/
```

レイアウトファイルを指定しない場合は [layout-examples/compose-default.json](../layout-examples/compose-default.json) が使用されます。

## 関連ドキュメント

## 関連ドキュメント

- [レイアウト JSON の詳細な仕様](./layout_spec.md)
- [リージョン（映像の配置方法指定）について](./layout_region.md)
- [コーデックやエンコードパラメーターの指定方法](./layout_encode_params.md)

## レイアウトの概要

## レイアウトの例: グリッド（デフォルトレイアウト）

## レイアウトの例: TODO

## レイアウトの例: TODO
