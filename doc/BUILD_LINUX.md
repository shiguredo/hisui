# Hisui をビルドする

Ubuntu 20.04 と 22.04 、x86_64 と arm64 でビルドを確認しています。

## 事前インストール

必要なライブラリをインストールします。

```
sudo apt install cmake clang libc6-dev libstdc++-10-dev yasm
```

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

release ディレクトリに hisui バイナリが生成されます。
