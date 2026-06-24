# testdata/fonts/

テキストオーバーレイ描画機能のテスト用フォント。
hisui の本番ランタイムでは使用しない (`--default-font` 等で指定するフォントとは別)。

## ファイル

| ファイル | 説明 |
|---|---|
| `PublicSans-Regular.ttf` | Public Sans Regular フォント本体 |
| `OFL.txt` | SIL Open Font License 1.1 本文と著作権表示 |

## ライセンス

Public Sans は **SIL Open Font License 1.1** で配布されている。
Copyright 2015 The Public Sans Project Authors。
詳細は [`OFL.txt`](OFL.txt) を参照。

## 出典

[uswds/public-sans](https://github.com/uswds/public-sans) (United States Web Design System)

## 取得手順

```sh
mkdir -p testdata/fonts
curl -sSL -o testdata/fonts/PublicSans-Regular.ttf \
  https://raw.githubusercontent.com/uswds/public-sans/develop/fonts/ttf/PublicSans-Regular.ttf
curl -sSL -o testdata/fonts/OFL.txt \
  https://raw.githubusercontent.com/uswds/public-sans/develop/OFL.txt
```
