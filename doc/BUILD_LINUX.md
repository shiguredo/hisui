# Hisui をビルドする

Ubuntu 20.04 x86_64 でのみビルドを確認しています。

## 事前準備

hisui をクローンします。

```
git clone https://github.com/shiguredo/hisui.git
```

必要なライブラリをインストールします。

```
sudo apt install cmake clang libc6-dev libstdc++-10-dev yasm
```

## ビルド

```
./build.bash ubuntu-20.04_x86_64
```

### --use-fdk-aac を有効にしたバイナリをビルドする

FDK-AAC を有効にする場合は自前でのビルドが必要になります。

libfdk-aac-dev をインストールします。

```
sudo apt install libfdk-aac-dev
```

```
./build.bash --use-fdk-aac ubuntu-20.04_x86_64
```

## バイナリ

release ディレクトリに hisui バイナリが生成されます。
