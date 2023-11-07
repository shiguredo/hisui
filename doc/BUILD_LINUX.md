# Hisui をビルドする

Ubuntu 20.04 と 22.04 、x86_64 と arm64 でビルドを確認しています。

## 事前準備

hisui をクローンします。

```
git clone https://github.com/shiguredo/hisui.git
```

必要なライブラリをインストールします。

```
sudo apt install build-essential
sudo apt install cmake clang libc6-dev libstdc++-10-dev yasm libva-dev libdrm-dev
sudo apt install python3-pip
pip install numpy
```

上記以外に Bazel をインストールする必要があります。
Lyra のビルドには Bazel バージョン 5.3.2 を使用するため、Bazelisk を使用してインストールすることをおすすめします。

https://bazel.build/install?hl=ja

## Ubuntu 20.04 x86_64 ビルド

```
./build.bash ubuntu-20.04_x86_64
```

## Ubuntu 20.04 arm64 ビルド

```
./build.bash ubuntu-20.04_arm64
```

## Ubuntu 22.04 x86_64 ビルド

```
./build.bash ubuntu-22.04_x86_64
```

## Ubuntu 22.04 arm64 ビルド

```
./build.bash ubuntu-22.04_arm64
```

### --use-fdk-aac を有効にしたバイナリをビルドする

FDK-AAC を有効にする場合は自前でのビルドが必要になります。

libfdk-aac-dev をインストールします。

```
sudo apt install libfdk-aac-dev
```

```
./build.bash --use-fdk-aac ubuntu-22.04_x86_64
```

## バイナリ

`release / ビルドを実行したアーキテクチャ名` の下に hisui バイナリが生成されます。
