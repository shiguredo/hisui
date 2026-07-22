# 外部由来のテストデータの出自

## `speech-en-16k-mono-s16le.pcm` / `speech-ja-16k-mono-s16le.pcm`

英語と日本語の短尺発話音声。

- **出所**: [Mozilla Common Voice](https://commonvoice.mozilla.org/)
- **クリップ ID**:
  - 英語: `common_voice_en_100540` (約 2.3 秒)
  - 日本語: `common_voice_ja_19486650` (約 3.3 秒)
- **ライセンス**: **CC0** (Common Voice のクリップライセンス)
- **形式**: 16 kHz mono / signed 16-bit little-endian raw PCM (Common Voice の mp3 を 16 kHz
  mono raw PCM にダウンサンプリング・変換したもの)
- **変換コマンド** (ffmpeg):
  ```
  ffmpeg -i common_voice_en_100540.mp3 -ac 1 -ar 16000 -f s16le speech-en-16k-mono-s16le.pcm
  ffmpeg -i common_voice_ja_19486650.mp3 -ac 1 -ar 16000 -f s16le speech-ja-16k-mono-s16le.pcm
  ```

### 派生形式: `e2e/transcribe/speech-en.mp4` / `speech-ja.mp4` (Opus in MP4)

上記の raw PCM から `hisui -x transcribe` の e2e テスト用に生成した Opus in MP4 (音声のみ)。
出所・クリップ ID・ライセンスは上記のとおり。 Linux CI で `--fdk-aac` を追加せずに扱えるよう
Opus を採用している。

- **変換コマンド** (ffmpeg、raw PCM から libopus + MP4 コンテナへ再エンコード):
  ```
  ffmpeg -f s16le -ar 16000 -ac 1 -i speech-en-16k-mono-s16le.pcm \
    -c:a libopus -b:a 64k -ar 48000 -movflags +faststart \
    e2e/transcribe/speech-en.mp4
  ffmpeg -f s16le -ar 16000 -ac 1 -i speech-ja-16k-mono-s16le.pcm \
    -c:a libopus -b:a 64k -ar 48000 -movflags +faststart \
    e2e/transcribe/speech-ja.mp4
  ```
