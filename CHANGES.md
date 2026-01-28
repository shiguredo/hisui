# 変更履歴

- UPDATE
  - 後方互換がある変更
- ADD
  - 後方互換がある追加
- CHANGE
  - 後方互換のない変更
- FIX
  - バグ修正

## develop

- [UPDATE] エンコーダーのインスタンス生成を実際の映像フレームが届くまで遅延させる
  - 今までは事前に解像度情報を指定していたが、ライブストリームの場合にはそれが難しいことがあるため遅延初期化をするようにする
  - @sile
- [UPDATE] indicatif crate のバージョンを 0.18.3 に上げる
  - @sile
- [UPDATE] log crate のバージョンを 0.4.29 に上げる
  - @sile
- [UPDATE] noargs crate のバージョンを 0.4.2 に上げる
  - @sile
- [UPDATE] orfail crate のバージョンを 2.0.0 に上げる
  - @sile
- [UPDATE] MP4 ファイルの読み書きに Mp4FileDemuxer および Mp4FileMuxer を使用する
  - 今までは shiguredo_mp4 の低レベル API を使っていたが、高レベル API に切り替える
  - @sile
- [UPDATE] hvc1 ボックスを含む入力 MP4 ファイルに対応する
  - 今までは H.265 では hev1 ボックスが使われている前提だったが、hvc1 ボックスにも対応する
  - @sile
- [UPDATE] shiguredo_mp4 のバージョンを 2026.1.0 にあげる
  - @sile
- [ADD] macOS で Audio Toolbox を使った AAC デコードに対応する
  - @sile
- [ADD] PyPI に `hiusi` を登録する GitHub Actions `pypi-publish.yml` を追加する
  - バージョンが `-canary.X` は `.devX` 形式に変換される
  - @voluntas
- [ADD] 実験的コマンドとして rtmp-outbound-endpoint を追加する
  - 入力ファイル（.mp4 or .webm）を RTMP サーバーとして配信するためのコマンド
  - `--experimental(-x)` フラグが指定されている場合にのみ利用可能となる
  - @sile
- [ADD] 実験的コマンドとして rtmp-publish を追加する
  - 入力ファイル（.mp4 or .webm）を RTMP サーバーに配信するためのコマンド
  - `--experimental(-x)` フラグが指定されている場合にのみ利用可能となる
  - @sile
- [ADD] 依存ライブラリに shiguredo_rtmp を追加する
  - @sile
- [CHANGE] コマンドライン引数に `--experimental(-x)` フラグを追加して `pipeline` サブコマンドはこのフラグ指定時にのみ有効になるようにする
  - `pipeline` サブコマンドは元々実験的機能扱いであったが、実験的機能を扱うためのフラグを追加して、より明確にハンドリングするようにする
  - @sile
- [CHANGE] 出力 MP4 ファイルが H.265 ストリームを含む場合は hvc1 ボックスを使用する
  - 今までは H.265 を表現するためには hev1 ボックスを使用していた
  - Apple 系のプレイヤーは hvc1 ボックスしかサポートしておらず、hev1 ボックスでは再生ができなかった
    - Apple 系プレイヤー以外は大抵 hev1 と hvc1 の両方をサポートしている
  - そのため H.265 用には hvc1 ボックスを使用することにする
  - hev1 と hvc1 は仕様や機能的にはほぼ同様なので、単に「より多くのプレイヤーが対応している方」を選択すればいい
    - もし今後 hev1 のみに対応している主要なプレイヤーが見つかった場合には、オプションでどちらのボックスを使用するかを指定可能にすることを検討する
  - @sile

### misc

- [ADD] Hisui Python バインディングテスト用の GitHub Actions `pytest.yml` を追加する
  - @voluntas
- [ADD] python/tests に Hisui Python バインディングのテストコードを追加する
  - @voluntas

## 2025.3.1

**リリース日**: 2025-11-27

- [FIX] FDK-ACC を使って合成を行う場合に SIGSEGV が発生する問題を修正する
  - shiguredo_fdk_aac 2025.1.0 でのバグだったため、それが修正された 2025.1.1 にバージョンをあげた
  - @sile
- [UPDATE] shiguredo_fdk_aac のバージョンを 2025.1.1 にあげる
  - @sile

## 2025.3.0

**リリース日**: 2025-11-06

- [UPDATE] hisui が直接に依存するパッケージのバージョンは厳密一致で指定するようにする
  - 今までは `log = "0.4.28"` のように指定していたが、これでは `cargo install hisui` の際に SemVer 的に互換性のある最新バージョンが使われてしまう
  - 通常はこの挙動でも問題はないが、依存パッケージ側の暗黙のバージョン更新によって hisui のビルドや動作に突然失敗するようになるリスクがある
  - それを防止するために、hisui が直接依存するパッケージのバージョン指定を厳密一致 (log の例なら `log = "=0.4.28"`）で行うようにする
  - なお、以下のケースは例外となる:
    - テスト用のパッケージはユーザー影響がないため、通常のバージョン指定のままにする
    - crates/* 以下の crate が依存するパッケージのバージョン指定は通常のままにする
      - ライブラリ用の crate でバージョン指定を厳しくすると、それが hisui 以外で使われるようになった時に、その利用側の別の依存のバージョン指定とコンフリクトして面倒なことになる可能性がある
      - もし依存先の依存のバージョンも厳密に制御したい場合には、hisui の Cargo.toml の中で、それらの厳密なバージョンを指定する方が望ましい
  - @sile

## 2025.2.0

**リリース日**: 2025-10-20

- [CHANGE] デコーダーおよびエンコーダーの名前を JSON に載せる際のキー名を "..._engine" 形式に統一する
  - compose コマンドの出力 JSON の `output_audio_encoder_name` を `output_audio_encode_engine` に変更する
  - compose コマンドの出力 JSON の `output_video_encoder_name` を `output_video_encode_engine` に変更する
  - vmaf コマンドの出力 JSON の `encoder_name` を `encode_engine` に変更する
  - @sile
- [UPDATE] shiguredo_libyuv のバージョンを 2025.2.0 に更新する
  - nv12 と i420 の相互変換関数が追加された
  - @sile
- [ADD] libvpx feature を追加する
  - デフォルトで有効
  - 無効にした場合には libvpx を用いた VP8 / VP9 のエンコードおよびデコードが行えなくなる
  - 主に CI 環境で、 libvpx が不要なテストのビルド時間短縮用に使用する目的
  - 内部利用前提であるため、ユーザーが無効化する必要はない（そのため公開ドキュメントにもこの feature は記載していない）
  - @sile
- [ADD] shiguredo_nvcodec を依存に追加する
  - @sile
- [ADD] NVIDIA Video Codec 対応を追加する
  - NVIDIA Video Codec SDK 13 を利用したデコードおよびエンコードに対応
    - 対応エンコードコーデックは H.264 / H.265 / AV1
    - 対応デコードコーデックは H.264 / H.265 / VP8 / VP9 / AV1
  - 動作には Linux の NVIDIA GPU 環境が必要 (CUDA ドライバー必須)
  - デフォルトでは無効で feature フラグで `nvcodec` を指定してビルドすると有効になる
    - ただし Ubuntu 24.04 (x86_64) 向けのビルド済みバイナリでは nvcodec 対応が有効になっている
  - 合わせてレイアウトファイルに以下の項目を追加（詳細はドキュメントを参照）
    - nvcodec_h264_encode_params
    - nvcodec_h265_encode_params
    - nvcodec_av1_encode_params
    - nvcodec_h264_decode_params
    - nvcodec_h265_decode_params
    - nvcodec_vp8_decode_params
    - nvcodec_vp9_decode_params
    - nvcodec_av1_decode_params
  - @sile
- [ADD] レイアウトファイルに video_encode_engines と video_decode_engines を追加する
  - 合成に使用するビデオエンコーダーとデコーダーを明示的に指定できるようにする
  - video_encode_engines: 映像エンコード時に使用するエンコーダーの候補を配列で指定（先頭のものほど優先される）
  - video_decode_engines: 映像デコード時に使用するデコーダーの候補を配列で指定（先頭のものほど優先される）
  - 指定可能な値（特定の features が有効な場合にのみ指定可能なものも含む）:
    - エンコーダー: "libvpx", "nvcodec", "openh264", "svt_av1", "video_toolbox"
    - デコーダー: "libvpx", "nvcodec", "openh264", "dav1d", "video_toolbox"
  - 未指定の場合は、その環境で利用可能なエンコーダーおよびデコーダーが全て候補となる（今まで通りの挙動）
  - @sile
- [ADD] macos-26 向けのリリースを追加する
  - @voluntas
- [ADD] macos-14 向けのリリースを追加する
  - @voluntas
- [CHANGE] legacy サブコマンドを削除する
  - Hisui 2025.1.x で提供されていた `hisui legacy` サブコマンドを削除
  - 代わりに `hisui compose` サブコマンドを使用すること
  - 詳細は [マイグレーションガイド](./docs/migrate_hisui_legacy.md) を参照
  - @sile
- [CHANGE] ビルド用 CUDA Toolkit のバージョンを 13.0.2 にする
  - @voluntas
- [FIX] Ubuntu 22.04 向けリリースビルドを追加する
  - x86_64 および arm64 アーキテクチャ向け
  - @voluntas
- [FIX] レイアウトファイルで `"audio_codec": "OPUS"` を指定するとエラーになるのを修正する
  - 値として "Opus" を期待する実装になっていたが、全て大文字が正しいので修正する
  - @sile

### misc

- [ADD] ci.yml にビルドバイナリを artifact としてアップロードするステップを追加する
  - @voluntas

## 2025.1.0

**祝リリース**
