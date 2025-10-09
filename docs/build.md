# ビルド方法

## ビルドに必要な依存パッケージのインストール

### Ubuntu の場合

Ubuntu の場合には以下のようにして、ビルドに必要なパッケージをインストールしてください。

```bash
sudo apt-get install -y meson ninja-build nasm yasm build-essential autoconf automake libtool pkg-config yasm cmake clang
```

### macOS の場合

macOS の場合には以下のようにして、ビルドに必要なパッケージをインストールしてください。

```bash
brew install meson ninja nasm yasm cmake automake autoconf libtool pkg-config
```

## Hisui 本体のビルド方法

Hisui は Rust のビルドツールである [Cargo](https://doc.rust-lang.org/cargo/) を使って以下のようにビルドします。

なお、必要な Rust バージョンは [`Cargo.toml`](../Cargo.toml) の `rust-version` を参照してください。

```bash
# crates.io からビルドする場合
cargo install hisui

# リポジトリ指定でビルドする場合
cargo install --git https://github.com/shiguredo/hisui.git

# ローカルに clone してからビルドする場合
git clone https://github.com/shiguredo/hisui.git
cd hisui/
cargo install --path .
```

上のいずれかの方法でビルドした hisui のバイナリは
`$HOME/.cargo/bin/hisui` のようなディレクトリに配置されます。
アンインストールする場合には `cargo uninstall hisui` を実行してください。

### NVIDIA Video Codec を使った H.264 / H.265 / AV1 のデコードおよびエンコードを有効にする場合

CUDA が利用できる環境で、以下のように `--features nvcodec` を指定して Hisui をビルドしてください。

```bash
cargo install hisui --features nvcodec
```

#### CUDA Toolkit のインストール

nvcodec 機能を有効にするには、CUDA Toolkit がインストールされている必要があります。

CUDA Toolkit は [NVIDIA の公式サイト](https://developer.nvidia.com/cuda-downloads) からダウンロードできます。

インストール後、`cuda.h` が以下のいずれかの場所に存在することを確認してください：

- デフォルトパス: `/usr/local/cuda/include/cuda.h`
- 環境変数で指定したパス: `$CUDA_INCLUDE_PATH/cuda.h`

デフォルトパス以外に CUDA をインストールした場合は、環境変数 `CUDA_INCLUDE_PATH` を設定してください：

```bash
export CUDA_INCLUDE_PATH=/path/to/cuda/include
cargo install hisui --features nvcodec
```

### FDK-AAC を使った AAC エンコードを有効にする場合

Ubuntu で FDK-AAC を使った AAC エンコードを行う場合には `libfdk-aac-dev` パッケージをインストールした上で、
`--features fdk-aac` を指定して Hisui をビルドする必要があります。

```bash
sudo apt-get install -y libfdk-aac-dev
cargo install hisui --features fdk-aac
```

なお macOS の場合には Apple Audio Toolbox を用いた AAC エンコードが自動で有効になるため、 FDK-AAC を利用する必要はありません。

## ビルド結果の確認方法

`hisui -h` を実行してみてください。

```console
$ hisui -h
Recording Composition Tool Hisui

Usage: hisui [OPTIONS] <COMMAND>

Commands:
  inspect     録画ファイルの情報を取得します
  list-codecs 利用可能なコーデック一覧を表示します
  compose     録画ファイルの合成を行います
  vmaf        VMAF を用いた映像エンコード品質の評価を行います
  tune        Optuna を用いた映像エンコードパラメーターの調整を行います
  pipeline    ユーザー定義のパイプラインを実行します（実験的機能）

Options:
  -h, --help    このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version バージョン番号を表示します
      --verbose 警告未満のログメッセージも出力します
```
