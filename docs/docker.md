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

Docker で Hisui を実行する際は、入出力ファイルへのアクセスのためにボリュームマウントが必要です。

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

### 録画ファイルの詳細情報を取得

```bash
docker run --rm \
  -v $(pwd)/recordings:/recordings \
  ghcr.io/shiguredo/hisui:latest \
  inspect /recordings/RECORDING_ID/archive-CONNECTION_ID.mp4
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
