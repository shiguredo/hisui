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

- [CHANGE] compose サブコマンドで `--stats-file` を指定した場合に出力される統計 JSON の内容を調整する
  - トップレベルの `worker_threads` が削除される
  - `processors` から `progress_bar` が削除される
  - `processors` の各要素から `total_processing_seconds` が削除される
  - `video_mixer` では `output_video_resolution` が削除され、`output_video_width` / `output_video_height` が追加される
  - `webm_audio_reader` / `webm_video_reader` では `input_files` が削除され、`current_input_file` / `total_sample_count` が追加される
  - @sile
- [ADD] server サブコマンドの JSON-RPC で Video Device source を作成できるようにする
  - JSON-RPC に `createVideoDeviceSource` メソッドを追加する
  - `outputVideoTrackId` にカメラ映像を配信できる
  - `deviceId` 未指定時は既定デバイスを利用する
  - `processorId` 未指定時は `videoDeviceSource:default` または `videoDeviceSource:<deviceId>` を既定値として利用する
  - @sile
- [ADD] server サブコマンドに `--ui-remote-url` オプションを追加する
  - 指定された場合、ローカルエンドポイント以外への GET リクエストを指定 URL にリバースプロキシする
  - 未指定の場合は従来通り 404 を返す
  - @voluntas
- [ADD] server サブコマンドの JSON-RPC で WHIP publisher を作成できるようにする
  - JSON-RPC に `createWhipPublisher` メソッドを追加する
  - `outputUrl` を指定して `inputVideoTrackId` / `inputAudioTrackId` のトラックを WHIP で配信できる
  - `bearerToken` で Authorization ヘッダーの Bearer トークンを指定できる
  - `videoCodecPreferences` で映像コーデック優先順を指定できる
  - @sile
- [ADD] server サブコマンドの JSON-RPC で WHEP subscriber を作成できるようにする
  - JSON-RPC に `createWhepSubscriber` メソッドを追加する
  - `inputUrl` を指定して WHEP の受信映像を `outputVideoTrackId` に配信できる
  - `processorId` 未指定時は `inputUrl` を既定値として利用する
  - 現時点では音声受信 (`outputAudioTrackId`) は未対応
  - @sile
- [ADD] server サブコマンドの JSON-RPC で PNG file source を作成できるようにする
  - JSON-RPC に `createPngFileSource` メソッドを追加する
  - `path` で指定した PNG を `outputVideoTrackId` に固定 FPS で繰り返し配信できる
  - `frameRate` の既定値は `1`
  - @sile
- [ADD] `VideoFormat` に `I420A` を追加する
  - `PngFileSource` の出力にアルファ付き I420 を使えるようにする
  - `VideoRealtimeMixer` は `I420A` を受け取った場合にアルファ合成して `I420` で出力する
  - 既存の `VideoMixer` は従来通り `I420` のみを対象とする
  - @sile
- [ADD] 環境変数 `HISUI_WEBRTC_LOG` で WebRTC ネイティブログを有効化できるようにする
  - `verbose` / `info` / `warning` / `error` / `none` を指定できる
  - @sile
- [ADD] server サブコマンドに `--startup-rpc-file` オプションを追加する
  - 起動時に実行する RPC リストを指定することができる機能
  - @sile
- [ADD] server サブコマンドに `--http-listen-address` オプションを追加する
  - HTTP サーバーのリッスンアドレスを指定可能にする
  - デフォルトは `127.0.0.1`
  - @sile
- [ADD] 実験的な server サブコマンドを追加する
  - `hisui server --http-port <PORT>` で HTTP サーバーを起動する
  - `/.ok` は 204 No Content を返す
  - `/bootstrap` は WebRTC 向けの SDP ベースのブートストラップを処理する:
    - `POST /bootstrap` かつ `Content-Type: application/sdp` の場合は 201 Created で SDP を返す
    - 条件に合わない場合は 400 / 404 / 405 / 409 / 415 / 500 を返す
  - `/rpc` では、JSON-RPC リクエストを処理する:
    - listTracks
    - listProcessors
    - createMp4FileSource
    - createVideoMixer
    - HTTP メソッドが POST 以外の場合は 405 Method Not Allowed を返す
    - JSON-RPC 通知（id なし）の場合は 204 No Content を返す
  - それ以外のパスには 404 Not Found を返す
  - @voluntas
- [ADD] server サブコマンドの `/metrics` で Prometheus 形式の Stats を JSON でも取得できるようにする
  - `GET /metrics?format=json` で `prom2json` 準拠の JSON を返す
  - 既存の `GET /metrics` は従来どおり Prometheus text 形式を返す
  - @sile
- [ADD] 依存ライブラリに shiguredo_http11 を追加する
  - @voluntas
- [ADD] 依存ライブラリに shiguredo_webrtc を追加する
  - @sile
- [UPDATE] Linux ビルドに必要なパッケージに `libx11-dev` を追加する
  - @sile
- [UPDATE] エンコーダーのインスタンス生成を実際の映像フレームが届くまで遅延させる
  - 今までは事前に解像度情報を指定していたが、ライブストリームの場合にはそれが難しいことがあるため遅延初期化をするようにする
  - @sile
- [UPDATE] log crate のバージョンを 0.4.29 に上げる
  - @sile
- [UPDATE] noargs crate のバージョンを 0.4.2 に上げる
  - @sile
- [UPDATE] MP4 ファイルの読み書きに Mp4FileDemuxer および Mp4FileMuxer を使用する
  - 今までは shiguredo_mp4 の低レベル API を使っていたが、高レベル API に切り替える
  - @sile
- [UPDATE] hvc1 ボックスを含む入力 MP4 ファイルに対応する
  - 今までは H.265 では hev1 ボックスが使われている前提だったが、hvc1 ボックスにも対応する
  - @sile
- [UPDATE] shiguredo_openh264 のバージョンを 2026.1.0-canary.0 にあげる
  - このバージョンから shiguredo_openh264 crate のリポジトリが https://github.com/shiguredo/openh264-rs に独立したので、hisui のワークスペースからは削除されている
  - @sile
- [UPDATE] shiguredo_mp4 のバージョンを 2026.1.0 にあげる
  - @sile
- [ADD] FDK-AAC を使った AAC デコードに対応する
  - @sile
- [ADD] macOS で Audio Toolbox を使った AAC デコードに対応する
  - @sile
- [ADD] PyPI に `hiusi` を登録する GitHub Actions `pypi-publish.yml` を追加する
  - バージョンが `-canary.X` は `.devX` 形式に変換される
  - @voluntas
- [ADD] server サブコマンドの JSON-RPC で RTMP inbound endpoint を作成できるようにする
  - JSON-RPC に `createRtmpInboundEndpoint` メソッドを追加する
  - `inputUrl` と `outputAudioTrackId` / `outputVideoTrackId` を指定して RTMP を受信し、指定トラックへ配信できる
  - `processorId` 未指定時は `rtmpInboundEndpoint` を既定値として利用する
  - @sile
- [ADD] server サブコマンドの JSON-RPC で RTMP outbound endpoint を作成できるようにする
  - JSON-RPC に `createRtmpOutboundEndpoint` メソッドを追加する
  - `outputUrl` と `inputAudioTrackId` / `inputVideoTrackId` を指定して RTMP サーバーとして配信できる
  - `processorId` 未指定時は `rtmpOutboundEndpoint` を既定値として利用する
  - @sile
- [ADD] server サブコマンドの JSON-RPC で RTMP publisher を作成できるようにする
  - JSON-RPC に `createRtmpPublisher` メソッドを追加する
  - `outputUrl` と `inputVideoTrackId` / `inputAudioTrackId` を指定して RTMP 配信できる
  - `processorId` 未指定時は `rtmpPublisher` を既定値として利用する
  - @sile
- [ADD] 依存ライブラリに shiguredo_rtmp を追加する
  - @sile
- [CHANGE] orfail crate を依存から削除する
  - これにより、エラー発生時に標準エラー出力に表示されるメッセージの細部のフォーマットに非互換な変更が入ることになる
  - @sile
- [CHANGE] indicatif の依存を削除して自前のプログレスバー実装に置き換える
  - @sile
- [CHANGE] 実験的な `pipeline` サブコマンドを削除する
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

- [ADD] 内部用に VideoDeviceSource 構造体を追加する
  - Video Device からの読み込みと I420 への変換を行い、映像トラックに出力する
  - @sile
- [ADD] 内部用に VideoRealtimeMixer 構造体を追加する
  - リアルタイム用途に特化した映像合成を行うための構造体
  - @sile
- [ADD] 内部用に Mp4FileSource 構造体を追加する
  - MP4 ファイルからの読み込みとデコードをセットで行うための構造体
  - @sile
- [ADD] e2e テスト用の GitHub Actions `e2e-test.yml` を追加する
  - @voluntas
- [ADD] Hisui Python バインディングテスト用の GitHub Actions `pytest.yml` を追加する
  - @voluntas
- [ADD] python/tests に Hisui Python バインディングのテストコードを追加する
  - @voluntas
- [CHANGE] 実験的に機能として undocumented で実装していたプラグイン機能を削除する
  - 内部的な PoC 目的の機能だったが、不要となったので削除する
  - @sile
- [CHANGE] shiguredo_libyuv の CMake 呼び出しを cmake crate に置き換える
  - @voluntas
- [CHANGE] shiguredo_svt_av1 の CMake 呼び出しを cmake crate に置き換える
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
