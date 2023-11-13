# Hisui をビルドする

Ubuntu 20.04 と 22.04 、x86_64 と arm64 でビルドを確認しています。

## 事前準備

hisui をクローンします。

```
git clone https://github.com/shiguredo/hisui.git
```

必要なライブラリをインストールします。

まず、Bazel をインストールする必要があります。
Lyra のビルドには Bazel バージョン 5.3.2 を使用するため、Bazelisk をインストールしてください。

https://bazel.build/install?hl=ja

### x86_64 ビルド時

```
sudo apt install cmake clang libc6-dev libstdc++-10-dev yasm libva-dev libdrm-dev python3-numpy g++
```

### Ubuntu 20.04 arm64 ビルド時

```
sudo apt install cmake clang-12 binutils-aarch64-linux-gnu libc6-dev-arm64-cross libstdc++-10-dev-arm64-cross yasm python3-numpy g++ libva-dev libdrm-dev

sudo update-alternatives --install /usr/bin/clang clang /usr/bin/clang-12 1
sudo update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++-12 1
```

### Ubuntu 22.04 arm64 ビルド時

```
sudo apt install cmake clang binutils-aarch64-linux-gnu libc6-dev-arm64-cross libstdc++-10-dev-arm64-cross yasm python3-numpy g++ libva-dev libdrm-dev
```

## ビルド

### Ubuntu 20.04 x86_64 ビルド

```
./build.bash ubuntu-20.04_x86_64
```

### Ubuntu 20.04 arm64 ビルド

```
./build.bash ubuntu-20.04_arm64
```

### Ubuntu 22.04 x86_64 ビルド

```
./build.bash ubuntu-22.04_x86_64
```

### Ubuntu 22.04 arm64 ビルド

```
./build.bash ubuntu-22.04_arm64
```

#### --use-fdk-aac を有効にしたバイナリをビルドする

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
