# Processor ID / Track ID の命名規則

## 概要

`media_pipeline` 上の各プロセッサとトラックには一意な ID（`ProcessorId` / `TrackId`）が割り当てられる。
この ID はメトリクス（`/metrics`）やログに表示されるため、ID を見ただけでそのリソースの役割を特定できる命名規則が必要である。

## 命名規則

ID はコロン `:` 区切りのセグメントで構成する。
数値型のセグメント（`source_key`、`run_id`）は末尾に置き、左から読んで「何のカテゴリの、何のコンポーネントか」がすぐ分かるようにする。

### カテゴリ一覧

| カテゴリ | 形式 | 用途 |
|----------|------|------|
| `program` | `program:{component}` | 合成パイプライン（常駐ミキサー） |
| `input` | `input:{component}:{source_key}` | ローカルソース |
| `output` | `output:{name}:{component}:{run_id}` | 下流出力 |
| `sora_source` | `sora_source:{media_kind}:{source_key}` | Sora リモートトラック |

### セグメントの意味

| セグメント | 型 | 説明 |
|-----------|-----|------|
| `component` | 文字列 | プロセッサやトラックの種別（例: `video_mixer`, `raw_video`, `mp4_writer`） |
| `source_key` | 数値 | input の識別子（input UUID に対応） |
| `name` | 文字列 | 出力の種別名（例: `record`, `stream`, `hls`, `mpeg_dash`） |
| `run_id` | 数値 | 出力の世代番号（停止→再開のたびにインクリメント） |
| `media_kind` | 文字列 | メディアの種別（`video` / `audio`） |

### `run_id` が output にだけ必要な理由

- **program**: 常駐パイプラインであり再起動しないため世代管理が不要
- **input**: program に従属するため同上
- **output**: 配信や録画は停止→再開を繰り返す。旧世代のプロセッサがクリーンアップ中に新世代が起動することがあるため、`run_id` で区別する必要がある

### `sora_source` を `input:` に含めない理由

通常の input（`rtsp_subscriber`、`color_source` 等）は自律的な source processor を持ち、自分でフレームを pipeline に publish する。
一方 `sora_source` は source processor を持たず、track ID だけを確保する。実際のフレーム publish は `sora_subscriber` プロセッサ側の `AttachSoraSourceTrack` で行われる。
このライフサイクルの違いから、`sora_source` は独立したカテゴリとして扱う。

## 具体例

### program

```
program:video_mixer
program:mixed_video
program:audio_mixer
program:mixed_audio
```

### input

```
input:rtsp_subscriber:0
input:raw_video:0
input:raw_audio:0
input:color_source:0
input:video_device_source:0
input:audio_device_source:0
input:mp4_source:0
input:png_source:0
input:rtmp_inbound:0
input:srt_inbound:0
```

### output

```
output:record:mp4_writer:0
output:stream:rtmp_publisher:0
output:stream:video_encoder:0
output:stream:encoded_video:0
output:hls:v0_hls_writer:1
output:hls:v1_scaler:1
output:mpeg_dash:v0_dash_writer:0
output:rtmp_outbound:endpoint:0
output:sora_publisher:publisher:0
output:sora_subscriber:my_sub_name
```

NOTE: `sora_subscriber` の末尾は `run_id` ではなく subscriber 名（文字列）である。

### sora_source

```
sora_source:video:0
sora_source:audio:0
```
