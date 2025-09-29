# Docker を利用した Hisui の実行

Hisui は Docker イメージとして提供されており、環境構築なしですぐに利用することができます。

Docker イメージは GitHub Container Registry (ghcr.io) で公開されており、amd64 と arm64 の両アーキテクチャに対応しています。

## イメージの取得

```bash
# 最新の安定版を取得
docker pull ghcr.io/shiguredo/hisui:latest

# 特定のバージョンを取得
docker pull ghcr.io/shiguredo/hisui:2025.1.0

# Canary 版を取得（最新機能を試したい場合）
docker pull ghcr.io/shiguredo/hisui:2025.1.0-canary.8
```

## 使用方法

Docker で Hisui を実行する際は、録画ファイルへのアクセスのためにボリュームマウントが必要です。

```bash
docker run --rm -it -v <ホストのディレクトリ>:<コンテナ内のパス> ghcr.io/shiguredo/hisui:latest <コマンド> <引数>
```

## 実行例

### バージョン確認

```console
$ docker run --rm ghcr.io/shiguredo/hisui:latest --version
hisui 2025.1.0
```

### 利用可能なコーデック一覧の表示

```console
$ docker run --rm ghcr.io/shiguredo/hisui:latest list-codecs
Audio Decoders:
  OPUS
  AAC
  ...

Video Decoders:
  VP8
  VP9
  H264
  ...
```

### デフォルトレイアウトでの録画ファイル合成

```bash
# 録画ディレクトリをマウントして合成を実行
docker run --rm -it \
  -v $(pwd)/recordings:/recordings \
  ghcr.io/shiguredo/hisui:latest \
  compose /recordings/RECORDING_ID/

# 出力ファイルの確認
ls recordings/RECORDING_ID/output.mp4
```

### レイアウトファイルを指定しての合成

```bash
# レイアウトファイルと録画ディレクトリをマウント
docker run --rm -it \
  -v $(pwd)/recordings:/recordings \
  -v $(pwd)/my-layout.json:/layout.json \
  ghcr.io/shiguredo/hisui:latest \
  compose -l /layout.json /recordings/RECORDING_ID/
```

### 出力ファイル名を指定しての合成

```bash
docker run --rm -it \
  -v $(pwd)/recordings:/recordings \
  ghcr.io/shiguredo/hisui:latest \
  compose -o /recordings/RECORDING_ID/composed.mp4 /recordings/RECORDING_ID/
```

### 統計情報を出力しての合成

```bash
docker run --rm -it \
  -v $(pwd)/recordings:/recordings \
  ghcr.io/shiguredo/hisui:latest \
  compose -s /recordings/RECORDING_ID/stats.json /recordings/RECORDING_ID/

# 統計情報の確認
cat recordings/RECORDING_ID/stats.json
```

### 録画ファイルの詳細情報を取得

```bash
docker run --rm \
  -v $(pwd)/recordings:/recordings \
  ghcr.io/shiguredo/hisui:latest \
  inspect /recordings/RECORDING_ID/
```

## 注意事項

### マルチアーキテクチャ対応

Docker イメージは amd64（Intel/AMD）と arm64（Apple Silicon など）の両方に対応しています。
Docker が自動的にホストのアーキテクチャに適したイメージを選択するため、特別な指定は不要です。

### タグ戦略

- `latest`: 最新の安定版リリース
- `<version>`: 特定のバージョン（例: `2025.1.0`）
- `<version>-canary.<number>`: Canary リリース（開発版）

Canary リリースは最新機能を含みますが、安定性は保証されません。
本番環境では `latest` または特定のバージョンタグの使用を推奨します。

### 未対応コマンド

この Docker イメージには Hisui 本体のバイナリしか含まれていません。
そのため、外部パッケージのインストールが別途必要となる以下のコマンドには未対応となります。
- [`hisui tune`](./command_tune.md) コマンド
- [`hisui vmaf`](./command_vmaf.md) コマンド
