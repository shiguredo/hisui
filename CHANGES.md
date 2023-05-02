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

- [CHANGE] OpenH264 での エンコードに対応する
    - 現時点では WebM のみ対応
    - `--out-video-codec` オプションに H.264 の指定を追加する
    - H.264 のチューニングオプションを追加する (ヘルプオプションには Debug でのみ表示される)
        - `--openh264-threads` : エンコード時のスレッド数を指定するオプション [ default 1]
        - `--openh264-min-qp`  : 最小量子化パラメータを指定するオプション [0 - 51]
        - `--openh264-max-qp`  : 最大量子化パラメータを指定するオプション [0 - 51]
            - ※量子化パラメータは、エンコーダーが画像データを圧縮する際に、どれだけの情報を削除するかを制御するパラメータ
    - 現時点で発見されている課題
        - 合成した H.264 のファイルを再生時、シークバーを動かすと再生が止まる
    - @haruyama
- [CHANGE] Hisui のオプションに `--version` を追加し、バージョン出力を追加する
    - `Recording Composition Tool Hisui [バージョン]` で出力する
    - @haruyama
- [CHANGE] ビルドオプションに `--build-type-debug` を追加し、デバッグビルドを追加する
    - デバッグビルドを利用することで通常見えないオプションをヘルプオプションで見ることができるようになる
    - @torikizi
- [FIX] `misspell` パッケージのインストールを `go get -u` から `go install` を利用するよう変更する
    - @haruyama

## 2023.1.1

- [UPDATE] バージョンを 2023.1.1 に上げる
    - @torikizi
    - 依存ライブラリの `cpp-mp4` を `2023.1.1` にするアップデートも含む
    - https://github.com/shiguredo/hisui/pull/118
- [FIX] config の typo を修正
    - @torikizi
    - https://github.com/shiguredo/hisui/pull/117

## 2023.1.0

- [FIX] --out-audio-codec の説明が間違っているのを修正
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/110
- [CHANGE] tarball に "hisui-${HISUI_VERSION}" ディレクトリを含める
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/108
- [UPDATE] 依存ライブラリの更新
    - @haruyama
    - `boost` を `1.81.0` に
    - `CLI11` を `2.3.2` に
    - `fmt` を `9.1.0` に
    - `spdlog` を `1.11.0` に
    - `libvpx` を `v1.13.0` に
    - `cpp-mp4` を `2023.1.0` に
    - `stb` を `5736b15f7ea0ffb08dd38af21067c314d6a3aae9` に
    - https://github.com/shiguredo/hisui/pull/106
- [FIX] Safari, Windows Media Player での再生の問題を修正
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/104
- [CHANGE] layout: レイアウト指定ファイル, report-*.json, *.webm は sources から常に除外する
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/103
- [FIX] Core dump する場合の修正
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/101
- [FIX] オーバーラップする間隔の検査時に start < end な間隔のみを利用する
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/100
- [UPDATE] deprecated になった actions/create-release と actions/upload-release の利用をやめて softprops/action-gh-release を利用する
    - @melpon
- [UPDATE] GitHub Actions の各種バージョンを上げる
    - @melpon
- [ADD] Ubuntu 20.04 ARM64 ビルドに対応
    - @melpon
- [ADD] Ubuntu 22.04 に対応
    - @melpon

## 2022.1.0

- [UPDATE] 依存ライブラリの更新
    - @haruyama
    - `boost` を `1.78.0` に
    - `CLI11` を `2.1.2` に
    - `fmt` を `8.0.1` に
    - `spdlog` を `1.9.2` に
    - `rapidcsv` を `8.53` に
    - `libvpx` を `v.1.11.0` に
    - `cpp-mp4` を `2022.1.0` に
    - `stb` を `af1a5bc352164740c1cc1354942b1c6b72eacb8a` に
- [CHANGE] Boost::JSON を header-only で利用する
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/91
- [CHANGE] レイアウト機能
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/48

## 2021.3

- [ADD] [実験的機能] 画面共有合成機能を追加する
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/40
- [ADD] [実験的機能] 合成成功/失敗時にレポートを出力する
    - @haruyama
    - https://github.com/shiguredo/hisui/pull/30
- [UPDATE] `cpp-mp4` を `2021.3` に
    - @haruyama
- [UPDATE] `boost` を `1.76.0` に
    - @haruyama

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
