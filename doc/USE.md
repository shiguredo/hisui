# Hisui を利用してみる

## 注意

公開されている docker イメージやバイナリは FDK-AAC には対応していません。FDK-AAC を利用する場合は自前でのビルドを行ってください。

自前でのビルドについては [BUILD_LINUX](BUILD_LINUX.md) をご参照ください。

## docker 経由で利用する

Hisui は docker image を用意しています。これを使うことで気軽に Hisui を利用可能です。

- https://hub.docker.com/r/shiguredo/hisui

```
docker run -v /home/shiguredo/sora/archive:/hisui -it shiguredo/hisui:2023.2.1-ubuntu-22.04 -f /hisui/CSX77QY9F57V5BT72S62C28VS4/report-CSX77QY9F57V5BT72S62C28VS4.json
```

- -v で Sora の録画データがある archive フォルダを指定して下さい
  - docker 側のフォルダはどこでも良いですがここでは /hisui を利用しています
- -f で合成したい recording.report が生成するファイルを指定して下さい

Lyra のファイルを変換したい場合はオプションを指定せずそのままのコマンドで利用可能です。

## docker 経由で help を見る

```
$ docker run -it shiguredo/hisui:2023.2.1-ubuntu-22.04 --help
hisui
Usage: /usr/local/bin/hisui [OPTIONS]

Options:
  -h,--help                   Print this help message and exit
  -f,--in-metadata-file       Metadata filename (REQUIRED)
  --version                   Print version and exit
  --out-container             Output container type (WebM/MP4). default: WebM
  --out-video-codec           Video codec (VP8/VP9/H264/AV1). default: VP9
  --out-video-frame-rate      Video frame rate (INTEGER/RATIONAL). default: 25
  --out-file                  Output filename
  --max-columns               Max columns (POSITIVE INTEGER). default: 3
  --libvpx-cq-level           libvpx Constrained Quality level (NON NEGATIVE INTEGER). default: 30
  --libvpx-min-q              libvpx minimum (best) quantizer (NON NEGATIVE INTEGER). default: 10
  --libvpx-max-q              libvpx maximum (worst) quantizer (NON NEGATIVE INTEGER). default: 50
  --out-opus-bit-rate         Opus bit rate (kbps, POSITIVE INTEGER). default: 65536
  --out-aac-bit-rate          AAC bit rate (kbps, POSITIVE INTEGER). default: 64000
  --mp4-muxer                 MP4 muxer (Faststart/Simple). default: Faststart
  --dir-for-faststart         Directory for intermediate files of faststart muxer. default: metadata directory
  --openh264                  OpenH264 dynamic library path
  --verbose                   Verbose mode
  --audio-only                Audio only mode
  --video-codec-engines       Show video codec engines and exit.
  --h264-encoder              H264 encoder (OneVPL/OpenH264). default: OneVPL
  --show-progress-bar         Toggle to show progress bar. default: true
  --layout                    Layout Metadata File
  --lyra-model-path           Path to directory containing Lyra TFLite files


Experimental Options:
  --screen-capture-report     Screen capture metadata filename
  --screen-capture-connection-id
                              Screen capture connection id
  --screen-capture-width      Width for screen-capture (NON NEGATIVE multiple of 4). default: 960
  --screen-capture-height     Height for screen-capture (NON NEGATIVE multiple of 4). default: 640
  --screen-capture-bit-rate   Bit rate for screen-capture (kbps). default: 1000
  --mix-screen-capture-audio  Mix screen-capture audio. default: false
  --success-report            Directory for success report
  --failure-report            Directory for failure report
```

## 自前ビルドで利用したい場合

-f で合成したい recording.report が生成するファイルを指定して下さい。

```
./hisui -f report-CSX77QY9F57V5BT72S62C28VS4.json
```

## 好きなレイアウトで合成したい

Hisui にはレイアウトという機能があり、そちらを利用することでより自由な合成が可能です。

もし、より複雑な合成を試されたい場合はぜひレイアウト機能を試してみてください。

詳細は [レイアウト機能](LAYOUT.md) のドキュメントをご参照ください。
