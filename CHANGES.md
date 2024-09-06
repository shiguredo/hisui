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

- [CHANGE] Lyra を Hisui から外し、 Lyra に関連するオプションやファイル、関数を削除する
  - Lyra 関連のファイルを削除
    - third_party/lyra を削除
    - src/audio/lyra を削除
  - Lyra 関連のビルド設定を削除
    - build.yml から Lyra 関連の設定を削除
    - build.bash から Lyra 関連の設定を削除
    - Doockerfile から Lyra 関連の設定を削除
  - `lyra_model_path` オプションを削除
  - `LyraHandler` クラスを削除
  - NOTICE から Lyra 関連の記述を削除
  - Lyra のバージョン定義を削除
  - ドキュメントから Lyra 関連の記述を削除
  - @torikizi
- [CHANGE] ビルド時に Bazel のインストールを行わないようにする
  - Lyra のために Bazel を利用していたので、関連して削除
  - @torikizi

## 2023.2.1

- [FIX] docker image 生成を修正する
  - @haruymaa

## 2023.2.0

- [UPDATE] 依存ライブラリの更新をする
  - `boost` を `1.83.0` にあげる
  - `cpp-mp4` を `2023.2.1` にあげる
  - `opus` を `1.4` にあげる
  - @haruyama
- [ADD] one VPL での H.264 エンコードに対応する
  - @haruyama
- [ADD] Lyra の デコードに対応する
  - @haruyama
- [ADD] SVT-AV1 での AV1 デコード/エンコードに対応する
  - @haruyama
- [ADD] OpenH264 での エンコードに対応する
  - `--out-video-codec` オプションに H.264 の指定を追加する
    - 画面共有合成機能では H.264 はサポートされない
  - H.264 のチューニングオプションを追加する (ヘルプオプションには Debug でのみ表示される)
    - `--openh264-threads` : エンコード時のスレッド数を指定するオプション [ default 1]
    - `--openh264-min-qp` : 最小量子化パラメータを指定するオプション [0 - 51]
    - `--openh264-max-qp` : 最大量子化パラメータを指定するオプション [0 - 51]
  - 現時点で発見されている課題
    - 合成した H.264 のファイルを再生時、シークバーを動かすと再生が止まる
  - @haruyama
- [ADD] Hisui のオプションに `--version` を追加し、バージョン出力を追加する
  - `Recording Composition Tool Hisui [バージョン]` で出力する
  - @haruyama
- [ADD] ビルドオプションに `--build-type-debug` を追加し、デバッグビルドを追加する
  - デバッグビルドを利用することで通常見えないオプションをヘルプオプションで見ることができるようになる
  - @torikizi
- [FIX] `misspell` パッケージのインストールを `go get -u` から `go install` を利用するよう変更する
  - @haruyama

## 2023.1.1

- [UPDATE] 依存ライブラリの `cpp-mp4` を `2023.1.1` にあげる
  - @torikizi
- [FIX] ヘルプで表示される config の typo を修正する
  - @torikizi

## 2023.1.0

- [FIX] --out-audio-codec の説明が間違っているのを修正する
  - @haruyama
- [CHANGE] tarball に "hisui-${HISUI_VERSION}" ディレクトリを含める
  - @haruyama
- [UPDATE] 依存ライブラリの更新をする
  - `boost` を `1.81.0` にあげる
  - `CLI11` を `2.3.2` にあげる
  - `fmt` を `9.1.0` にあげる
  - `spdlog` を `1.11.0` にあげる
  - `libvpx` を `v1.13.0` にあげる
  - `cpp-mp4` を `2023.1.0` にあげる
  - `stb` を `5736b15f7ea0ffb08dd38af21067c314d6a3aae9` にあげる
  - @haruyama
- [FIX] Hisui で合成した MP4 ファイルが再生環境によって再生できない問題を修正する
  - Safari, Windows Media Player, 映画＆テレビ での再生を修正する
  - @haruyama
- [CHANGE] レイアウトに `*` のみを指定した場合、全てのレイアウトを指定したものとして扱うよう修正する
  - layout: レイアウト指定ファイル, report-_.json, _.webm は sources から常に除外する
  - @haruyama
- [FIX] 例外と null 参照を修正する
  - 一部のケースで core dump していたのを修正する
  - @haruyama
- [FIX] オーバーラップする間隔の検査時に start < end な間隔のみを利用する
  - レイアウトを使用したとき start = end なファイルが生成されていた場合エラーになっていたので修正する
  - @haruyama
- [UPDATE] deprecated になった actions/create-release と actions/upload-release の利用をやめて softprops/action-gh-release を利用する
  - @melpon
- [UPDATE] GitHub Actions の各種バージョンを上げる
  - @melpon
- [ADD] Ubuntu 20.04 ARM64 ビルドに対応する
  - @melpon
- [ADD] Ubuntu 22.04 に対応する
  - @melpon

## 2022.1.0

- [UPDATE] 依存ライブラリの更新をする
  - `boost` を `1.78.0` にあげる
  - `CLI11` を `2.1.2` にあげる
  - `fmt` を `8.0.1` にあげる
  - `spdlog` を `1.9.2` にあげる
  - `rapidcsv` を `8.53` にあげる
  - `libvpx` を `v.1.11.0` にあげる
  - `cpp-mp4` を `2022.1.0` にあげる
  - `stb` を `af1a5bc352164740c1cc1354942b1c6b72eacb8a` にあげる
  - @haruyama
- [CHANGE] Boost::JSON を header-only で利用する
  - @haruyama
- [CHANGE] レイアウト機能を追加する
  - `--layout` オプションを追加する
  - JSON 形式で作成したレイアウトファイルを利用して自由に合成する機能を追加する
  - @haruyama

## 2021.3

- [ADD] [実験的機能] 画面共有合成機能を追加する
  - `--screen-capture-report` を指定して合成すると他の合成データより優先して表示する
  - 実験的機能として画面共有合成機能オプションを追加する
    - `--screen-capture-report` : 画面共有のメタデータを指定するオプション
    - `--screen-capture-connection-id` : 画面共有の Connection ID を指定するオプション
    - `--screen-capture-width` : 画面共有の width を指定するオプション (正の整数で、4 の倍数であること) [default: 960]
    - `--screen-capture-height` : 画面共有の height を指定するオプション (正の整数で、4の倍数であること) [default: 640]
    - `--screen-capture-bit-rate` : 画面共有のビットレートを指定するオプション (Kbps) [default: 1000]
    - `--mix-screen-capture-audio` : 画面共有の音声を合成するか指定するオプション [default: false]
  - 注意点
    - `--screen-capture-report` で指定されたものの中で時間が重なった合成データがある場合は そのうちの 1 つのみを利用する.
  - @haruyama
- [ADD] [実験的機能] 合成成功/失敗時にレポートを出力する機能を追加する
  - レポートは指定したディレクトリに出力する
  - 実験的機能として合成成功/失敗時にレポートを出力するオプションを追加する
    - `--success-report` : 合成成功時にレポートを出力するオプション (`{utc_datetime}_ {recoding_id}_success.json`)
    - `--failure-report ` : 合成失敗時にレポートを出力するオプション (`{utc_datetime}_ {recoding_id}_failure.json`)
  - 合成失敗レポートはコマンドライン引数の処理での失敗時には出力しない
  - レポートの対象は以下のようにする
    - 入力 (各ファイルごとに)
      - 音声のデコーダー情報 (codec, channels, duration)
      - 映像のデコーダー情報 (codec, duration)
      - 映像の解像度の変化 (timestamp, widht, height)
    - 出力(container, mux_type, video_codec, audio_codec, duration)
    - hisui 自体と利用ライブラリのバージョン
  - @haruyama
- [UPDATE] `cpp-mp4` を `2021.3` にあげる
  - @haruyama
- [UPDATE] `boost` を `1.76.0` にあげる
  - @haruyama

## 2021.2.3

- [FIX] PixelWidth/Height が 0 な VideoTrack を持つ WebM に対応するで混入したバグを修正する
  - @haruyama

## 2021.2.2

- [FIX] PixelWidth/Height が 0 な VideoTrack を持つ WebM に対応する
  - libwebm で不正なファイルとして扱われるため patch をあて, hisui 側で不正と判定するように変更する
  - @haruyama

## 2021.2.1

- [FIX] --libvpx-therads が指定されていない場合の挙動を修正する
  - @haruyama

## 2021.2

- [UPDATE] `libvpx` を `v1.10.0` にあげる
  - @haruyama
- [CHANGE] libvpx のパラメータのデフォルト値を調整する
  - @haruyama
- [UPDATE] --libvp9-row-mt コマンドラインオプションを追加する
  - @haruyama
- [UPDATE] --libvp9-tile-columns コマンドラインオプションを追加する
  - @haruyama
- [UPDATE] WebM/MP4 Muxer の mux() を共通化する
  - @haruyama
- [ADD] 音声の mix のみを行なう --audio-only コマンドラインオプションの追加する
  - @haruyama

## 2021.1.1

- [FIX] std::async で作った Future を get() し例外を伝播させる
  - @haruyama
- [FIX] 解像度の変更が入っている H.264 の WebM を合成しようとすると落ちるのを修正する
  - @haruyama

## 2021.1

- [ADD] OpenH264 を利用した WebM 中の H.264 の decode に対応する
  - @haruyama
- [ADD] cpp-mp4 を利用した MP4 の出力に対応する
  - @haruyama
- [ADD] libfdk-aac を利用した MP4 への AAC の出力に対応する
  - @haruyama
- [UPDATE] `boost` を `1.75.0` にあげる
  - @haruyama
- [UPDATE] `fmt` を `7.1.3` にあげる
  - @haruyama
- [UPDATE] `spdlog` を `1.8.2` にあげる
  - @haruyama
- [CHANGE] `nlohmann::json` から `boost::json` へ切り替える
  - @haruyama

## 2020.1.1

- [FIX] Video のない WebM ファイルを利用した場合の取り扱いを修正する
  - @haruyama

## 2020.1

**祝リリース**
