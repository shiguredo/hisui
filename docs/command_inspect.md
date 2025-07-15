# `inspect` コマンド

`inspect` コマンドは、録画ファイルの詳細情報を取得するための開発者向けコマンドです。

## 使用方法

```console
$ hisui inspect [OPTIONS] INPUT_FILE
```

### 引数

- `INPUT_FILE`: 情報取得対象の録画ファイル（.mp4 または .webm）

### オプション

- `--decode`: 指定された場合にはデコードまで行う
- `--openh264 <PATH>`: OpenH264 の共有ライブラリのパス（環境変数 `HISUI_OPENH264_PATH` でも設定可能）

## 出力内容

コマンドは以下の情報を JSON 形式で出力します：

- **基本情報**: ファイルパス、コンテナ形式
- **音声情報**: コーデック、総再生時間、サンプル数、各サンプルの詳細
- **映像情報**: コーデック、総再生時間、フレーム数、キーフレーム数、各フレームの詳細

### サンプル情報

各音声・映像サンプルには以下の情報が含まれます：

- `timestamp_us`: タイムスタンプ（マイクロ秒）
- `duration_us`: 継続時間（マイクロ秒）
- `data_size`: データサイズ（バイト）
- `keyframe`: キーフレームかどうか（映像のみ）
- `decoded_data_size`: デコード後のデータサイズ（`--decode` 指定時）
- `width`/`height`: 解像度（映像のみ、`--decode` 指定時）

## 使用例

```console
# 基本的な情報取得
$ hisui inspect archive.mp4

# デコードも含めた詳細情報取得
$ hisui inspect --decode archive.mp4

# OpenH264 ライブラリを指定
$ hisui inspect --openh264 /path/to/libopenh264.so archive.mp4
```

このコマンドは主にデバッグやファイル分析に使用されます。
