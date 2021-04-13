# 変更履歴

- CHANGE
    - 下位互換のない変更
- UPDATE
    - 下位互換がある変更
- ADD
    - 下位互換がある追加
- FIX
    - バグ修正

## develop

## 2021.2.3

- [FIX] PixelWidth/Height が 0 な VideoTrack を持つ WebM に対応するで混入したバグを修正する
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/39

## 2021.2.2

- [FIX] PixelWidth/Height が 0 な VideoTrack を持つ WebM に対応する
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/38

## 2021.2.1

- [FIX] --libvpx-therads が指定されていない場合の挙動を修正
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/34

## 2021.2

- [UPDATE] `libvpx` を `v1.10.0` に
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/31
- [CHANGE] libvpx のパラメータのデフォルト値を調整
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/31
- [UPDATE] --libvp9-row-mt コマンドラインオプションの追加
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/31
- [UPDATE] --libvp9-tile-columns コマンドラインオプションの追加
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/24
    - https://github.com/shiguredo/hisui/pull/28
- [UPDATE] WebM/MP4 Muxer の mux() を共通化
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/23
- [ADD] 音声の mix のみを行なう --audio-only コマンドラインオプションの追加
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/26

## 2021.1.1

- [FIX] std::async で作った Future を get() し例外を伝播させる
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/22
- [FIX] 解像度の変更が入っている H.264 の WebM を合成しようとすると落ちるのを修正
    - https://github.com/shiguredo/hisui/pull/21
    - @haruyama

## 2021.1

- [ADD] OpenH264 を利用した WebM 中の H.264 の decode
    - @haruyama
- [ADD] cpp-mp4 を利用した MP4 の出力
    - @haruyama
- [ADD] libfdk-aac を利用した MP4 への AAC の出力
    - @haruyama
- [UPDATE] `boost` を `1.75.0` に
    - @haruyama
- [UPDATE] `fmt` を `7.1.3` に
    - @haruyama
- [UPDATE] `spdlog` を `1.8.2` に
    - @haruyama
- [CHANGE] `nlohmann::json` から `boost::json` への切り替え
    - @haruyama

## 2020.1.1

- [FIX] Video のない WebM ファイルを利用した場合の取り扱いを修正
    - @haruyama

## 2020.1

**祝リリース**
