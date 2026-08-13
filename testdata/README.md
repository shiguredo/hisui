# 外部由来のテストデータの出自

## `archive-h264-resolution-change.mp4`

多エントリ `stsd` (sample_entry が 1 トラック内で切り替わる) の解像度変更の回帰テスト用データ。
nvcodec デコーダーが sample_entry 変化に伴う VPS / SPS / PPS 更新を追従できることを検証する。

- **出所**: hotfix/2025.3.3 (PR #328) で追加されたものを develop へ復元したもの (git 履歴由来のバイナリ)
- **ライセンス**: なし (合成データ)
- **構成**: 多エントリ stsd (entry_count=118)。15 fps × 3 秒 = 45 フレームで、キーフレームが frame 0 / 15 / 30 にある
  - frame 0..15 → 320x240
  - frame 15..30 → 224x160
  - frame 30..45 → 320x240

## `h264-resolution-change.mp4` / `h265-resolution-change.mp4`

単一 `stsd` + ビットストリーム内パラメータセット変化 (解像度変更) の回帰テスト用データ。
VideoToolbox デコーダーがフレームデータ内の SPS/PPS/VPS 変化を追従できることを検証する。

- **出所**: ffmpeg で生成した合成動画 (640x480 と 320x320 を concat)
- **ライセンス**: なし (合成データ)
- **生成コマンド** (ffmpeg):
  ```
  # H.264 (前半 640x480 と後半 320x320 を concat で結合する)
  # 後半キーフレームに in-band SPS/PPS が入るのは、x264 がストリーム開始時に SPS/PPS を
  # 置き、concat の h264_mp4toannexb がキーフレームへ挿入するため (明示指定なし)。
  # この前提が崩れると回帰テストがデータ原因で失敗するため、再生成時は必ず
  # hisui inspect --decode で後半キーフレームに SPS(7) + PPS(8) が含まれることを確認する。
  ffmpeg -f lavfi -i "color=c=blue:s=640x480:d=1:r=25" -c:v libx264 -preset ultrafast -profile:v baseline -pix_fmt yuv420p a.mp4
  ffmpeg -f lavfi -i "color=c=red:s=320x320:d=1:r=25" -c:v libx264 -preset ultrafast -profile:v baseline -pix_fmt yuv420p b.mp4
  printf "file 'a.mp4'\nfile 'b.mp4'\n" > list.txt
  ffmpeg -f concat -safe 0 -i list.txt -c copy h264-resolution-change.mp4
  # H.265 (hev1。repeat-headers=1 で各キーフレームに VPS/SPS/PPS を入れる)
  ffmpeg -f lavfi -i "color=c=blue:s=640x480:d=1:r=25" -c:v libx265 -preset ultrafast -pix_fmt yuv420p -x265-params repeat-headers=1:bframes=0 a.mp4
  ffmpeg -f lavfi -i "color=c=red:s=320x320:d=1:r=25" -c:v libx265 -preset ultrafast -pix_fmt yuv420p -x265-params repeat-headers=1:bframes=0 b.mp4
  printf "file 'a.mp4'\nfile 'b.mp4'\n" > list.txt
  ffmpeg -f concat -safe 0 -i list.txt -c copy h265-resolution-change.mp4
  ```
- 注意: H.265 は `bframes=0` が必須 (B フレームがあると MP4 リーダーが composition_time_offset でエラーになる)

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
