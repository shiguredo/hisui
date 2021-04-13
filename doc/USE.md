# Hisui を利用してみる

## 注意

公開されている docker イメージやバイナリは FDK-AAC には対応していません。FDK-AAC を利用する場合は自前でのビルドを行ってください。

## docker 経由で利用する

Hisui は docker image を用意しています。これを使うことで x86_64 環境であれば気軽に Hisui を利用可能です。

- https://hub.docker.com/r/shiguredo/hisui
- https://github.com/orgs/shiguredo/packages/container/package/hisui

```
docker run -v /home/shiguredo/sora-2020.3/archive:/hisui ghcr.io/shiguredo/hisui:2021.2.3 -f /hisui/CSX77QY9F57V5BT72S62C28VS4/report-CSX77QY9F57V5BT72S62C28VS4.json
```

- -v で Sora の録画データがある archive フォルダを指定して下さい
    - docker 側のフォルダはどこでも良いですがここでは /hisui を利用しています
- -f で合成したい recording.report が生成するファイルを指定して下さい


## docker 経由で help を見る

```
$ docker run ghcr.io/shiguredo/hisui:2021.2.3 hisui --help
hisui
Usage: /usr/local/bin/hisui [OPTIONS]

Options:
  -h,--help                   Print this help message and exit
  -f,--in-metadata-file       Metadata filename (REQUIRED)
  --out-container             Output container type (WebM/MP4) default: WebM
  --out-video-codec           Video codec (VP8/VP9) default: VP9
  --out-video-frame-rate      Video frame rate (INTEGER/RATIONAL) default: 25)
  --out-file                  Output filename
  --max-columns               Max columns (POSITIVE INTEGER) default: 3
  --libvpx-cq-level           libvpx Constrained Quality level (NON NAGATIVE INTEGER) default: 10
  --libvpx-min-q              libvpx minimum (best) quantizer (NON NEGATIVE INTEGER) default: 3
  --libvpx-max-q              libvpx maximum (worst) quantizer (NON NEGATIVE INTEGER) default: 40
  --out-opus-bit-rate         Opus bit rate (kbps, POSITIVE INTEGER). default: 65536
  --out-aac-bit-rate          AAC bit rate (kbps, POSITIVE INTEGER). default: 64000
  --mp4-muxer                 MP4 muxer (Faststart/Simple). default: Faststart
  --dir-for-faststart         Directory for intermediate files of faststart muxer. default: metadata directory
  --openh264                  OpenH264 dynamic library path
  --verbose                   Verbose mode
  --audio-only                Audio only mode
  --show-progress-bar         Toggle to show progress bar. default: true
```



## 自前ビルドで利用したい場合

-f で合成したい recording.report が生成するファイルを指定して下さい。

```
./hisui -f report-CSX77QY9F57V5BT72S62C28VS4.json
```
