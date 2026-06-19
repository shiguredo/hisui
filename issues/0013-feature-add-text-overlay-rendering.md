# 合成映像へのテキストオーバーレイ描画に対応する

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/add-text-overlay-rendering
- Polished: 2026-06-19

## 目的

合成映像にテキストをオーバーレイ描画できる汎用基盤を提供する。応用例: ラベル・タイムスタンプ・任意テキストの表示、別 issue 0012 (candle / Whisper 文字起こし) の結果を字幕として映像に重ねて表示する、等。本 issue は描画プリミティブと obsws 経由の操作 API までを範囲とし、字幕特有のスタイル一式 (縁取り・背景帯)・時刻スケジューリング・特定入力への追従は扱わない。

## 優先度根拠

テキスト描画は複数応用の前提となる汎用基盤で、単体で動作確認・マージが可能。即時の業務影響はないため Medium。

## 現状

### 映像合成

- リアルタイム合成 (`src/mixer/video.rs` の `VideoRealtimeMixer`) は I420 上で `compose_frame` / `blend_component` により I420A レイヤを z-order でブレンドして I420 を出力する。`InputTrack.z` の型は `isize` (`src/mixer/video.rs:138`)。
- 録画合成 (`src/sora/recording_video_mixer.rs`) はリアルタイム合成とは別実装系 (`Canvas` / `mix_region` / `draw_frame`、アルファ合成なし)。本 issue のスコープ外。
- 色空間変換・リサイズは `shiguredo_libyuv` を使用する。RGBA 系 → I420 + アルファ変換として `argb_to_i420_alpha` (`shiguredo_libyuv-2026.1.0/src/convert.rs:5216`、ARGB バイト順) などが存在する。hisui ではまだ未使用 (`src/obsws/source/png_file.rs:139` の手書き `rgba_like_to_i420a` で同等処理が回っている)。
- canvas は hisui 全体で 1 つだけ、起動時固定 (`src/subcommand_server.rs:80-87` の `--canvas-width` / `--canvas-height`、`src/obsws/state.rs:56-62` の `ObswsSessionState::new` 経由で以降不変、実行中の resize API は無い)。obsws レスポンスの `canvasName` は OBS 互換のため `"Main"` 固定 (`src/obsws/response/general.rs:288-291`)。複数 canvas の概念はない。
- output 出力 (`mp4_output` / `hls_output` / `mpeg_dash_output` 等) は `VideoRealtimeMixer` の出力 track (`program:mixed_video`) を購読する。`VideoRealtimeMixer` 内で TextOverlay レイヤと合成された出力がそのまま永続化される。
- テキスト描画機能は無い。

### MediaPipeline / Processor

- MediaPipeline の processor 概念が確立済み (`src/media_pipeline.rs` の `register_processor` / `spawn_processor` / `ProcessorHandle` / `subscribe_track` / `publish_track`)。
- `MediaFrame` / `RawVideoFrame` は内部に `Arc<VideoFrame>` を保持しており (`src/media.rs:8-11` / `src/video.rs:61-62`)、processor 間のフレーム受け渡しは Arc クローンのみでフレーム本体のメモリコピーは発生しない。
- **publish only (subscribe なし) の processor 参照実装**: `src/obsws/source/color_source.rs:20-75` / `src/obsws/source/png_file.rs:25-70`。`publish_track` のみ呼び、`notify_ready()` の直後に `wait_subscribers_ready().await?` で最初の subscriber 接続を待ち、その後 `tokio::time::sleep_until(start + frame_index * frame_period)` ループでセルフタイミングし、`MAX_NOACKED_COUNT = 100` の ack/syn による back-pressure を取る。

### obsws 独自拡張

- hisui 独自拡張リクエストが多数存在し、命名規則 `Hisui<Verb><Noun>` が確立されている (`HisuiCreateOutput` / `HisuiGetWebRtcStats` 等、14 メソッド)。
- 個別 md は `## Request` (`requestId` を含む) + `## RequestData` + `## ResponseData` + `## エラー条件` + `## 制約` の 2 段構造 (例: `docs/server/hisui_requests/HisuiCreateOutput.md`)。
- 識別子はクライアント指定の一意名 (`outputName` / `subscriberId` 等)。
- リクエストハンドラのディスパッチは `src/obsws/message.rs:175` の `match request_type.as_str()` で、`HisuiCreate*` 系は `src/obsws/coordinator.rs:658-661` 等で処理。RequestBatch（op=8）対応可否は `src/obsws/coordinator.rs:1131-1132` 近傍の許可リストで管理されている。
- エラーコード定数は `src/obsws/protocol.rs:44-58` の `REQUEST_STATUS_*`。本 issue で使うのは `MISSING_REQUEST_FIELD (300)` / `INVALID_REQUEST_FIELD (400)` / `RESOURCE_NOT_FOUND (601)` / `RESOURCE_ALREADY_EXISTS (602)` / `RESOURCE_ACTION_NOT_SUPPORTED (606)`。
- リクエストハンドラからの状態反映は `src/obsws/session/output.rs:131-137` の `update_program_mixers` 経由で `VideoRealtimeMixerUpdateConfigRequest` を送り、**`input_tracks` を全置換**する設計 (`src/mixer/video.rs:509-510`)。`output_plan.video_mixer_input_tracks` は `src/obsws/output_plan.rs:89-159` の `build_composed_output_plan` 内で `source_plans.iter().zip(active_scene_inputs.iter())` の `filter_map` クロージャ 1 つから組み立てられ、scene の input から都度再計算される (`ObswsVideoMixerInputTrack.z` の型は `i64`、`output_plan.rs:24`)。
- `build_composed_output_plan` の呼び出し元は 2 箇所: `src/obsws/server.rs:315` (初期化時) と `src/obsws/coordinator.rs:844` (`rebuild_program_output` シーン切替時)。

## 設計方針

### 1. 描画ライブラリ

- shiguredo/raden (https://github.com/shiguredo/raden) v2026.1.1 を採用する (crates.io 公開済み、Apache-2.0)。Cranelift JIT ベースの CPU only な 2D ベクターグラフィックスライブラリで、GPU の無い CI 環境とも相性が良い。
- `Cargo.toml` に `raden = "=2026.1.1"` で追加する (hisui の依存は他のクレートと同様パッチまで含めた `=` 固定が慣行、`Cargo.toml` 冒頭 NOTE 参照)。
- raden の連鎖依存 (`cranelift-*` 系・フォント解析系) によりビルド時間・バイナリサイズが増加する。実装着手時に CI 時間影響と clippy / fmt 等の既存 CI への影響を計測し、許容範囲外なら別途検討する。

#### raden 調査結果 (確定済み)

raden v2026.1.1 のソースを直接確認して以下を確定済み。本表の項目は §2 / §3 の設計判断の根拠となる。

| 項目 | 確定値 |
|---|---|
| 採用バージョン | `=2026.1.1` (crates.io 公開済み、Apache-2.0) |
| 出力ピクセルフォーマット | `PixelFormat::Prgb32` = 32-bit premultiplied ARGB (`0xAARRGGBB`、リトルエンディアン環境ではバイト列 `B G R A`)。`Image::new(w, h, PixelFormat::Prgb32)` で生成、`data() -> &[u8]` で参照 |
| premultiplied vs straight alpha | **premultiplied (Prgb32)**。hisui の `blend_component` (`src/mixer/video.rs:1136-1148`) は straight 前提のため、I420A 化前に **straight 復元** (`A > 0 ? (RGB * 255 + A / 2) / A : 0` を 4 チャネル各々に適用) を `TextOverlayProcessor` 内のヘルパーで行う |
| 採用する shiguredo_libyuv 関数 | `argb_to_i420_alpha` (`shiguredo_libyuv-2026.1.0/src/convert.rs:5216`、ARGB バイト順入力 = 上記 straight 復元後のバッファをそのまま渡せる) |
| `FontData` / `FontFace` / `Font` / `Image` / `Context` の `Send + Sync` | 明示 `impl` なし。内部型は `Vec<u8>` / `Arc<Vec<u8>>` / プリミティブのみで auto trait による Send + Sync 成立見込み。実装時に `cargo check` で確認する。万一 `Send` 不可なら `tokio::task::LocalSet` 配下に変更する |
| Cranelift JIT 初回コンパイル遅延 | `PipelineRuntime` がパイプラインキャッシュを持ち、同一パラメータの関数は再コンパイルしない。`TextOverlayProcessor` 起動直後に空文字列の `fill_text` を 1 回実行して JIT をウォームアップする。実遅延は実装時に計測 |
| glyph 不在時の挙動 | `src/api/context.rs:1153-1157` で `glyph_id == 0` ならアドバンスのみ進めて描画スキップ (silent skip)。tofu は出ない。本 issue はこの既定挙動のまま透過 I420A レイヤに含める (エラーは返さない) |
| フォントロード API | `FontData::from_file(path: &str) -> Result<FontData, FontError>` → `FontFace::from_data(&font_data, index: u32) -> Result<FontFace, FontError>` (TTC 単体は `index = 0`) → `Font::from_face(&face, size: f64) -> Font` |
| 描画コンテキスト構築 | `PipelineRuntime::new()` (processor あたり 1 回) → `Context::new(&mut image, &mut runtime)` (描画呼び出しごと)。`Context::end()` で締める |

### 2. フォント

#### CLI 引数

- `--font-search-root <dir>`: フォント探索ルート (絶対パス必須)。サーバはこの配下のファイルのみ参照する。
- `--default-font <fontName>`: 省略時のデフォルトフォント名 (例: `Roboto-Regular.ttf`)。

`src/subcommand_server.rs:80-87` 近傍 (canvas 引数の隣) に `noargs::opt("font-search-root")` / `noargs::opt("default-font")` で追加し、`ObswsSessionState::new` (`src/obsws/state.rs:56-62`) と `start_obsws_server` (`src/obsws/server.rs`) に伝播させる。

#### 起動時検証 (CLI パース直後)

- `--font-search-root` 指定時: `canonicalize` してルートパスを確定。失敗時 (存在しない / 権限なし) は **hisui プロセスを起動失敗 (abort)** させる (server 起動引数の不整合のため警告継続より安全)。
- `--default-font` 指定時: `<root>/<fontName>` を `canonicalize` してルート配下に収まるか検証し、raden で `FontData::from_file` → `FontFace::from_data` まで成功することを確認 (`Font::from_face` はサイズ依存なので起動時には行わない)。失敗時は同じく起動失敗。
- 両方未指定: **テキストオーバーレイ機能無効** として hisui は正常起動する。`HisuiCreateTextOverlay` / `HisuiUpdateTextOverlay` / `HisuiRemoveTextOverlay` / `HisuiListTextOverlays` 呼び出し時は `RESOURCE_ACTION_NOT_SUPPORTED (606)` を返す (エラー文言で「`--font-search-root` / `--default-font` が未指定」を伝える)。
- 片方のみ指定: 起動失敗 (CLI 引数の組として整合性を取る)。

#### 安全策 (path traversal 対策)

- `fontName` には `/` / `\` / `..` / NULL バイト (`\0`) を含めない (含まれていたら `INVALID_REQUEST_FIELD`)。
- 解決後のパスは `canonicalize` してから root の `canonicalize` 結果配下に収まるか検証する (収まらなければ `INVALID_REQUEST_FIELD`)。
- シンボリックリンクは `canonicalize` で root 外に出るため自動的に弾かれる。
- 上記は hisui のサポート OS (macOS / Linux) で `std::fs::canonicalize` が実体パスへ解決する挙動を前提とする。Windows サポートは本 issue では考慮しない。

#### その他

- CJK / フォントに含まれない文字: raden の既定挙動 (silent skip、glyph_id=0 はアドバンスのみ進めて描画スキップ。tofu は出ない) のまま透過 I420A レイヤに含める。代替フォント探索・複数フォント登録の動的管理 API・フォールバックチェーン・CJK 標準フォント提供は本 issue では扱わない。
- テスト用フォントとして Roboto Regular (Apache 2.0、Google Fonts 配布版) を `testdata/fonts/Roboto-Regular.ttf` に配置する。

### 3. 描画 API (内部)

#### Processor の起動・存在期間

§1 の raden 調査結果により、`Send + Sync` は auto trait による成立見込み、JIT 初回遅延は warm-up で吸収可能と確認済み。以下の常駐 spawn 戦略で進める (実装時に `cargo check` で `Send` 成立を最終確認)。

- 新規 `TextOverlayProcessor` を MediaPipeline 上の 1 processor として実装する。canvas 単一・起動時固定のため、**server 起動時に常駐 spawn し、サーバ稼働中ずっと生かす** (`src/subcommand_server.rs` の MediaPipeline 構築直後、テキストオーバーレイ機能有効時のみ)。シーン切替で再生成しない。
- ProcessorId は `program:text_overlay_processor` 固定、出力 track_id は `program:text_overlay` 固定。canvas サイズ・frame_rate は CLI 引数から受け取り起動時固定 (実行中の追従不要)。
- run ループは `src/mixer/video.rs:353-374` の `VideoRealtimeMixerRunner::run` を input event_rx なしで簡略化した形 (`rpc_rx` と `tokio::time::sleep_until(next_output_instant)` の 2 系統 select)。
- 起動シーケンス: `notify_ready()` を呼んだ後、**`wait_subscribers_ready().await?` は呼ばない**。`wait_subscribers_ready` (`src/media_pipeline.rs:1120` 周辺) は「初期 processor 集合の `notify_ready` 完了」を待つ API であり、TextOverlayProcessor は初期 processor 集合に含めず、後発で接続する subscriber (output 系) を前提とする非定常 publisher として動かすため、待機 API を呼ぶ意味がない。`color_source.rs:40-41` の定番 2 行セットからは外れる扱いとなる旨をコード上のコメントで明記する。

#### TextOverlay InputTrack の VideoRealtimeMixer への注入

- `VideoRealtimeMixerUpdateConfigRequest` は input_tracks を全置換するため、シーン切替で TextOverlay track が落ちる。これを防ぐため:
  - `src/obsws/output_plan.rs` の `build_composed_output_plan` のシグネチャに `text_overlay_track_id: Option<TrackId>` を追加する。`Some(track_id)` なら関数内で `video_mixer_input_tracks` を構築した後 (`.collect()` 後) に `push` (または `chain`) で **末尾 (`z = i64::MAX`) に TextOverlay InputTrack を追加** する。`None` (機能無効時) なら何もしない。
  - 呼び出し元 4 箇所 (本体 2 箇所: `src/obsws/server.rs:315` 初期化時 / `src/obsws/coordinator.rs:844` `rebuild_program_output`、テスト 2 箇所: `src/obsws/output_plan.rs:226` の既存ユニットテスト `build_composed_output_plan_skips_dormant_inputs` / `src/obsws/session/tests.rs:143` の共通ヘルパー `create_initialized_coordinator_handle_with_pipeline_and_record_dir`) を改修する。本体 2 箇所はテキストオーバーレイ機能有効時のみ `Some(...)` を渡す。テスト 2 箇所は `None` 固定で既存挙動を保つ。
- `i64::MAX` (`ObswsVideoMixerInputTrack.z: i64` の型に合わせる) は **テキストオーバーレイ専用予約値** とし、一般 input track が指定することを禁止する。内部の `InputTrack.z: isize` (`src/mixer/video.rs:138`) に渡すときも同等の最上位値となる (hisui のサポートターゲットは 64bit のため `isize::MAX == i64::MAX`)。
- 複数テキストオーバーレイ間の z は `TextOverlayProcessor` 内部でソートして 1 枚の I420A に重ね合わせる段で解決する (`VideoRealtimeMixer` 側の z 配列には影響しない)。InputTrack は常に 1 つだけで、`OVERLAY_LIMIT = 64` は processor 内部の overlay マップの上限であり mixer 側の InputTrack 数とは無関係。

#### 内部 RPC

- `TextOverlayProcessor` は `register_rpc_sender` パターン (`src/mixer/video.rs:222-457` 参考) で `TextOverlayRpcMessage` enum (バリアント: `Add` / `Update` / `Remove` / `List`) を受け、各バリアントは `oneshot::Sender` で reply する。
- reply 型は `Result<T, TextOverlayError>` (`T` は Add/Update/Remove で `()`、List で `Vec<TextOverlayState>`)。`TextOverlayError` のバリアントと REQUEST_STATUS マッピングは §4 のエラー対応表に集約する。
- `TextOverlayPatch` は Update 用で全フィールド `Option<T>`、`None` = 省略 = 現状維持。JSON 上の `null` 受信は `INVALID_REQUEST_FIELD` として扱う。
- `TextOverlayState` は `HisuiCreateTextOverlay` の全属性 (`textOverlayName` / `text` / `x` / `y` / `fontSize` / `fontColor` / `fontName` / `z`) を保持し、`List` の戻り値および JSON 化される。
- 並列性: processor 内ループは単一 task で `rpc_rx` を順次処理する。同名 overlay の Add と Remove が同時送信された場合は受信順 (FIFO) で処理する。

#### 描画フロー

- raden の `Image (PixelFormat::Prgb32)` に全 overlay を z 順に重ね描き (premultiplied ARGB バッファ) → straight 復元ヘルパー (§1 調査結果参照) で straight ARGB へ戻す → `shiguredo_libyuv::argb_to_i420_alpha` で I420A に変換 → `Arc<VideoFrame>` を生成して `cached_frame` に保持。`dirty = false` の間は毎フレーム Arc クローンを `publish_track` する。
- `dirty` を `true` にする条件: `text` / `x` / `y` / `fontSize` / `fontColor` / `fontName` / `z` のいずれかが変わった (Add / Update / Remove のいずれか)。canvas サイズは起動時固定のため `dirty` 化トリガにならない。
- 全 overlay が空 (`overlays.is_empty()`) の場合は publish しない (mixer 側で `pending_frames` 空の InputTrack は何も合成しないため透明な状態となる)。

各種上限値・位置/サイズ制約 (`OVERLAY_LIMIT = 64`、`text` 4096 バイト・64 行、`fontSize` 範囲等) は §4 RequestData 表に集約する。raden の描画コストは文字数に比例するため、巨大入力で processor がブロックして他 overlay 操作が詰まるのを防ぐ意図。なお `cached_frame` 保持で processor あたり I420A 1 フレーム分 (1920x1080 で約 3 MB) のメモリを常時占有する。

### 4. 外部 API (obsws)

#### 命名

hisui obsws 既存の独自拡張命名規則 `Hisui<Verb><Noun>` に従う。

#### メソッド一覧

| メソッド | 説明 |
|---|---|
| `HisuiCreateTextOverlay` | テキストオーバーレイ作成 = 即表示 |
| `HisuiUpdateTextOverlay` | 既存オーバーレイの属性を部分更新 |
| `HisuiRemoveTextOverlay` | オーバーレイ削除 |
| `HisuiListTextOverlays` | 一覧取得 |

#### 共通: Request

全メソッド共通: `requestId` (string、必須) のみ。

#### 識別子

`textOverlayName` (string、サーバ全体で一意) はクライアント指定。hisui は単一 canvas のため canvas スコープは導入しない。

#### `HisuiCreateTextOverlay` RequestData

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `textOverlayName` | string | 必須 | サーバ全体で一意 |
| `text` | string | 必須 | `\n` 改行可、最大 4096 バイト、最大 64 行 |
| `x` | integer | 必須 | キャンバス絶対座標 X (左上原点、px)。負値・キャンバス外は許容 (raden 側でクリップ) |
| `y` | integer | 必須 | キャンバス絶対座標 Y (左上原点、px)。同上 |
| `fontSize` | integer | 必須 | px。`1` 以上 `canvas_height` 以下 |
| `fontColor` | string | - | 正規表現 `^#[0-9A-Fa-f]{6}([0-9A-Fa-f]{2})?$`、default `#FFFFFFFF` |
| `fontName` | string | - | `--font-search-root` 配下のファイル名 (拡張子付き)、default は `--default-font` |
| `z` | integer | - | overlay 間 z-order、省略時は宣言順 (= 後勝ち) |

ResponseData: なし。

#### `HisuiUpdateTextOverlay` RequestData

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `textOverlayName` | string | 必須 | 対象識別子 (変更不可) |
| `text` | string | - | 省略時は現状維持 (以下同じ) |
| `x` | integer | - | |
| `y` | integer | - | |
| `fontSize` | integer | - | |
| `fontColor` | string | - | |
| `fontName` | string | - | |
| `z` | integer | - | |

ResponseData: なし。

#### `HisuiRemoveTextOverlay` RequestData

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `textOverlayName` | string | 必須 | |

ResponseData: なし。

#### `HisuiListTextOverlays` RequestData

なし。

ResponseData: `textOverlays` 配列。各要素は以下:

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `textOverlayName` | string | 必須 | |
| `text` | string | 必須 | |
| `x` | integer | 必須 | |
| `y` | integer | 必須 | |
| `fontSize` | integer | 必須 | |
| `fontColor` | string | 必須 | 解決済みの値 (デフォルト適用後) |
| `fontName` | string | 必須 | 解決済みの値 (デフォルト適用後) |
| `z` | integer | 必須 | 解決済みの値 (宣言順から確定された値) |

#### エラー条件 (全メソッド共通対応表)

| `TextOverlayError` | REQUEST_STATUS | 適用メソッド | 条件 |
|---|---|---|---|
| `AlreadyExists` | `RESOURCE_ALREADY_EXISTS (602)` | Create | 同名既存 |
| `NotFound` | `RESOURCE_NOT_FOUND (601)` | Update / Remove | 対象 overlay が存在しない |
| (フィールド欠落) | `MISSING_REQUEST_FIELD (300)` | 全 | 必須フィールドなし |
| `InvalidFontName` | `INVALID_REQUEST_FIELD (400)` | Create / Update | `fontName` が `/` `\` `..` `\0` を含む |
| `FontResolveFailed` | `INVALID_REQUEST_FIELD (400)` | Create / Update | `fontName` 解決失敗 (ファイルなし / ルート外 / シンボリックリンクでルート外 / フォント破損) |
| `InvalidColor` | `INVALID_REQUEST_FIELD (400)` | Create / Update | `fontColor` 形式違反、または JSON `null` 受信 |
| `InvalidFontSize` | `INVALID_REQUEST_FIELD (400)` | Create / Update | `fontSize` 範囲外 |
| `InvalidText` | `INVALID_REQUEST_FIELD (400)` | Create / Update | `text` がバイト数 / 行数上限超過 |
| `RenderFailed` | `INVALID_REQUEST_FIELD (400)` | Create / Update | raden 描画失敗 (詳細はエラー文言) |
| `LimitExceeded` | `RESOURCE_ACTION_NOT_SUPPORTED (606)` | Create | `OVERLAY_LIMIT` 超過 |
| `Disabled` | `RESOURCE_ACTION_NOT_SUPPORTED (606)` | 全 | テキストオーバーレイ機能無効 |

#### 共通制約

- WebSocket / データチャネル両方で利用可能 (`HisuiCreateOutput` と同じ)。
- RequestBatch（op=8）対応 (`src/obsws/coordinator.rs:1131-1132` 近傍の許可リストに 4 メソッドを追加する)。
- obsws レスポンスの `comment` フィールドは英語で記述する (CLAUDE.md「ログメッセージは全て英語」と同方針)。

### 5. ドキュメント

- `docs/server/hisui_requests/HisuiCreateTextOverlay.md` / `HisuiUpdateTextOverlay.md` / `HisuiRemoveTextOverlay.md` / `HisuiListTextOverlays.md` を新規作成する。構造 (`## Request` / `## RequestData` / `## ResponseData` / `## エラー条件` / `## 制約`) は `HisuiCreateOutput.md` を踏襲する。
- `docs/server/hisui_requests/README.md` に「## テキストオーバーレイ」節を追加する。節冒頭に既存節 (例: 「Output 管理」) と同様の前提条件行「WebSocket / データチャネル両対応。RequestBatch（op=8）に対応。」を入れる。
- `docs/obsws/PROTOCOL_STATUS.md` の独自拡張節に反映する。
- `docs/obsws/STATE_FILE.md` の永続化対象列挙 (line 7-8) にテキストオーバーレイが永続化対象外である旨を明記する。
- `docs/internals/mixer.md` (実在を確認済み) に「`InputTrack.z` の最大値 (`i64::MAX` 相当) はテキストオーバーレイレイヤ用の予約値、一般 input track では使用しない」を追記する。
- `README.md` に「対応プラットフォームは 64bit (`isize::MAX == i64::MAX` 前提)」を明記する (既存に明示がないため本 issue で確定する。`docs/internals/mixer.md` の `z` 予約値説明はこの記述を参照する形で書く)。

(closed 0040 で議論された `docs/internals/processor_conventions` 系のドキュメントは存在しない結論で close されているため、本 issue で追記する先はない。)

### 6. スコープ

- リアルタイム合成 (obsws 経由) のみを対象とする。`VideoRealtimeMixer` の出力 `program:mixed_video` を購読する `mp4_output` / `hls_output` / `mpeg_dash_output` 等を通せば、テキスト描画済みの動画ファイルが結果として得られる (字幕応用 0012 の主用途もこの経路で実現される)。
- 録画合成 (`src/sora/recording_video_mixer.rs` / `src/sora/recording_subcommand_compose.rs`) は本 issue では一切触らない。

## 完了条件

- 4 メソッドが obsws (WebSocket / データチャネル両方) と RequestBatch（op=8）から動作する。
- `--font-search-root` / `--default-font` の CLI 引数が動作する (両方未指定で機能無効・正常起動、片方のみで起動失敗、両方指定で起動時検証成功なら常駐 `TextOverlayProcessor` を spawn、検証失敗で起動失敗。リクエスト時の `fontName` で `..` 含む / シンボリックリンク経由でルート外を指すパスは拒否)。
- ドキュメント整備:
  - `docs/server/hisui_requests/` 個別 md 4 本
  - `docs/server/hisui_requests/README.md` への節追加 (前提条件行込み)
  - `docs/obsws/PROTOCOL_STATUS.md` 反映
  - `docs/obsws/STATE_FILE.md` 永続化対象外追記
  - `docs/internals/mixer.md` への `z` 予約値追記
  - `README.md` に「対応プラットフォームは 64bit」を追記
- テスト (`testdata/fonts/Roboto-Regular.ttf` を本 issue のコミットに含める):
  - `src/obsws/session/tests.rs` 相当箇所に 4 メソッド往復 + 全エラーケース (`AlreadyExists` / `NotFound` / `InvalidFontName` / `FontResolveFailed` / `InvalidColor` / `InvalidFontSize` / `InvalidText` / `LimitExceeded` / `Disabled` / `MISSING_REQUEST_FIELD`) の検証を追加する。
  - `TextOverlayProcessor` の単体テストで raden → I420A 変換後の A プレーン非ゼロ領域が指定 x/y 近傍に収まっていることを検証する。
  - `pbt/` に x/y/fontSize の境界値 PBT を追加する (`text=""` / `x = i64 境界` / `fontSize = 1` / `fontSize = canvas_height` / 改行のみ / Unicode 制御文字 / フォント不在文字 / `text` 長境界 / 行数境界)。
- 0012 等の他 issue に依存せず、本 issue 単独で動作確認・マージできる。
- CHANGES.md の `## develop` に `[ADD]` エントリを追記する。エントリ例 (担当者欄はコミット担当者の実 GitHub ID に置き換える。複数担当者の場合は `  - @a` / `  - @b` のように行を分ける):

```
- [ADD] obsws 経由でリアルタイム合成映像にテキストオーバーレイを描画できるようにする
  - 起動時 CLI 引数 `--font-search-root` / `--default-font` でフォント探索ルートとデフォルトフォントを指定する
  - `HisuiCreateTextOverlay` / `HisuiUpdateTextOverlay` / `HisuiRemoveTextOverlay` / `HisuiListTextOverlays` の 4 メソッドを obsws 経由で利用できる
  - @<github-id>
```
