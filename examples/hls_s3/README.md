# hls_s3

Hisui の OBSWS API を使い、MP4 ファイルを入力として S3 互換オブジェクトストレージに HLS セグメントを出力するサンプル。

## 前提

- Hisui が OBSWS モードで起動していること
- S3 互換ストレージが起動していること
- 出力先バケットが作成済みであること

## 起動方法

```bash
# 1. S3 互換ストレージを起動する（ポート 9000 で待ち受ける想定）

# 2. バケットを作成する
curl -X PUT http://127.0.0.1:9000/hls-test

# 3. Hisui を起動する
hisui -x obsws --canvas-width 320 --canvas-height 320 --frame-rate 15

# 4. サンプルを実行する
cargo run -p hls_s3 -- \
  --input-mp4-path testdata/red-320x320-h264-aac.mp4 \
  --s3-bucket hls-test \
  --s3-prefix live \
  --s3-endpoint http://127.0.0.1:9000 \
  --s3-path-style
```

Ctrl+C で停止する。

## オプション

| オプション | デフォルト | 説明 |
|-----------|-----------|------|
| `--input-mp4-path` | (必須) | 入力 MP4 ファイルパス |
| `--s3-bucket` | (必須) | S3 バケット名 |
| `--s3-prefix` | `hls` | オブジェクトキーの prefix |
| `--s3-region` | `us-east-1` | S3 リージョン |
| `--s3-endpoint` | `http://127.0.0.1:9000` | S3 エンドポイント URL |
| `--s3-path-style` | (フラグ) | パススタイル URL を使用する |
| `--s3-access-key` | `admin` | アクセスキー ID |
| `--s3-secret-key` | `admin` | シークレットアクセスキー |
| `--segment-format` | `fmp4` | セグメントフォーマット (`mpegts` / `fmp4`) |
| `--host` | `127.0.0.1` | Hisui OBSWS 接続先ホスト |
| `--port` | `4455` | Hisui OBSWS 接続先ポート |
| `-v` | - | 詳細ログを出力する |

## 出力例

```
 INFO TCP 接続完了: 127.0.0.1:4455
 INFO WebSocket 接続完了
 INFO obsws セッション確立
 INFO CreateInput 送信: testdata/red-320x320-h264-aac.mp4
 INFO CreateInput 成功
 INFO SetOutputSettings 送信: bucket=hls-test, prefix=live, endpoint=http://127.0.0.1:9000
 INFO SetOutputSettings 成功
 INFO StartOutput 送信
 INFO HLS S3 出力開始
 INFO GetOutputStatus: {"outputActive":true, ... ,"outputPath":"s3://hls-test/live/playlist.m3u8"}
 INFO Ctrl+C で停止します
```

S3 上のオブジェクト:

```
live/init.mp4            # fMP4 init segment
live/playlist.m3u8       # メディアプレイリスト
live/segment-000000.m4s  # セグメント
live/segment-000001.m4s
live/segment-000002.m4s
...
```
