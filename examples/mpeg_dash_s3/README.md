# mpeg_dash_s3

Hisui の OBSWS API を使い、MP4 ファイルを入力として S3 互換オブジェクトストレージに MPEG-DASH セグメントを出力するサンプル。

## 前提

- Hisui が OBSWS モードで起動していること
- S3 互換ストレージが起動していること
- 出力先バケットが作成済みであること

## 起動方法

```bash
# 1. S3 互換ストレージを起動する（ポート 9000 で待ち受ける想定）

# 2. バケットを作成する
curl -X PUT http://127.0.0.1:9000/dash-test

# 3. Hisui を起動する
hisui -x obsws --canvas-width 320 --canvas-height 320 --frame-rate 15

# 4. サンプルを実行する
cargo run -p mpeg_dash_s3 -- \
  --input-mp4-path testdata/red-320x320-h264-aac.mp4 \
  --s3-bucket dash-test \
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
| `--s3-prefix` | `dash` | オブジェクトキーの prefix |
| `--s3-region` | `us-east-1` | S3 リージョン |
| `--s3-endpoint` | `http://127.0.0.1:9000` | S3 エンドポイント URL |
| `--s3-path-style` | (フラグ) | パススタイル URL を使用する |
| `--s3-access-key` | `admin` | アクセスキー ID |
| `--s3-secret-key` | `admin` | シークレットアクセスキー |
| `--host` | `127.0.0.1` | Hisui OBSWS 接続先ホスト |
| `--port` | `4455` | Hisui OBSWS 接続先ポート |
| `-v` | - | 詳細ログを出力する |

## 出力例

```
 INFO TCP connected: 127.0.0.1:4455
 INFO WebSocket connected
 INFO obsws session established
 INFO CreateInput: testdata/red-320x320-h264-aac.mp4
 INFO CreateInput succeeded
 INFO SetOutputSettings: bucket=dash-test, prefix=live, endpoint=http://127.0.0.1:9000
 INFO SetOutputSettings succeeded
 INFO StartOutput requested
 INFO MPEG-DASH S3 output started
 INFO GetOutputStatus: {"outputActive":true, ... ,"outputPath":"s3://dash-test/live/manifest.mpd"}
 INFO Press Ctrl+C to stop
```

S3 上のオブジェクト:

```
live/init.mp4            # fMP4 init segment
live/manifest.mpd        # MPD マニフェスト
live/segment-000000.m4s  # セグメント
live/segment-000001.m4s
live/segment-000002.m4s
...
```
