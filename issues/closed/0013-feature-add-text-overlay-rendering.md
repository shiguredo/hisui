# 合成映像へのテキストオーバーレイ描画に対応する

- Priority: Medium
- Created: 2026-06-03
- Completed: 2026-06-23
- Model: Opus 4.8
- Branch: feature/add-text-overlay-rendering
- Polished: 2026-06-22

## 目的

合成映像にテキストをオーバーレイ描画できる汎用基盤を提供する。応用例: ラベル・タイムスタンプ・任意テキストの表示、別 issue 0012 (candle / Whisper 文字起こし) の結果を字幕として映像に重ねて表示する、等。本 issue は描画プリミティブと obsws 経由の操作 API までを範囲とし、字幕特有のスタイル一式 (縁取り・背景帯)・時刻スケジューリング・特定入力への追従は扱わない。

## 優先度根拠

テキスト描画は複数応用の前提となる汎用基盤で、単体で動作確認・マージが可能。即時の業務影響はないため Medium。

## 現状

### 映像合成

- リアルタイム合成 (`src/mixer/video.rs` の `VideoRealtimeMixer`) は I420 上で `compose_frame` (自由関数、`src/mixer/video.rs:914`) と `RealtimeI420Canvas::draw_frame_clipped` (`src/mixer/video.rs:1028` 周辺) により I420A レイヤを z-order でブレンドして I420 を出力する。`InputTrack` のフィールド型は `x: isize` / `y: isize` (`src/mixer/video.rs:136-137`) と `z: i32` (`src/mixer/video.rs:138`)。obsws 層の `ObswsVideoMixerInputTrack` は `x: i64` / `y: i64` / `z: i32` (`src/obsws/output_plan.rs:22-24`)。
- 録画合成 (`src/sora/recording_video_mixer.rs`) はリアルタイム合成とは別実装系 (`Canvas` / `mix_region` / `draw_frame`、アルファ合成なし)。本 issue のスコープ外。
- 色空間変換・リサイズは `shiguredo_libyuv` を使用する。RGBA 系 → I420 + アルファ変換として `argb_to_i420_alpha` (`shiguredo_libyuv-2026.1.0/src/convert.rs:5216`、ARGB バイト順) が存在する。
- canvas は hisui 全体で 1 つだけ、起動時固定 (`src/subcommand_server.rs:80-87` の `--canvas-width` / `--canvas-height`、`src/obsws/state.rs:56-62` の `ObswsSessionState::new` 経由で以降不変、実行中の resize API は無い)。obsws レスポンスの `canvasName` は OBS 互換のため `"Main"` 固定 (`src/obsws/response/general.rs:288-291`)。複数 canvas の概念はない。
- output 出力 (`mp4_output` / `hls_output` / `mpeg_dash_output` 等) は `VideoRealtimeMixer` の出力 track (`program:mixed_video`) を購読する。`VideoRealtimeMixer` 内でテキストオーバーレイレイヤと合成された出力がそのまま永続化される。
- テキスト描画機能は **本ブランチ内の過去コミットで「独立 `TextOverlayProcessor` を MediaPipeline 上に常駐 spawn し、`z = i32::MAX` 予約値で `VideoRealtimeMixer` の最終 InputTrack として接続」する旧設計で実装済み** (`src/mixer/text_overlay.rs` 1316 行、`src/obsws/coordinator/text_overlay.rs` 973 行、`src/obsws/output_plan.rs` の `text_overlay_track_id` 引数、`src/obsws/server.rs:314-335` の常駐 spawn、`src/obsws/coordinator.rs:878-887` の `rebuild_program_output` での text_overlay_track 解決、`docs/internals/mixer.md:145-151` の予約値節、4 つの個別 md (`HisuiCreateTextOverlay.md` 他)、`docs/obsws/PROTOCOL_STATUS.md:1064-1086`、`pbt/tests/prop_text_overlay.rs`、`src/obsws/session/tests.rs` のテキストオーバーレイテストヘルパー)。本 issue では設計を **mixer 内部レイヤ統合** に作り変えるため、これらは全て撤去 or 書き換え対象 (§3「旧実装の撤去対象」と完了条件チェックリストに集約)。

### MediaPipeline / Processor

- MediaPipeline の processor 概念が確立済み (`src/media_pipeline.rs` の `register_processor` / `spawn_processor` / `ProcessorHandle` / `subscribe_track` / `publish_track`)。
- `MediaFrame` / `RawVideoFrame` は内部に `Arc<VideoFrame>` を保持しており (`src/media.rs:8-11` / `src/video.rs:61-62`)、processor 間のフレーム受け渡しは Arc クローンのみでフレーム本体のメモリコピーは発生しない。
- **publish only (subscribe なし) の processor 参照実装**: `src/obsws/source/color_source.rs:20-75` / `src/obsws/source/png_file.rs:25-70`。`publish_track` のみ呼び、`notify_ready()` の直後に `wait_subscribers_ready().await?` で最初の subscriber 接続を待ち、その後 `tokio::time::sleep_until(start + frame_index * frame_period)` ループでセルフタイミングし、`MAX_NOACKED_COUNT = 100` の ack/syn による back-pressure を取る。

### obsws 独自拡張

- hisui 独自拡張リクエストが多数存在し、命名規則 `Hisui<Verb><Noun>` が確立されている (`HisuiCreateOutput` / `HisuiGetWebRtcStats` 等、14 メソッド)。
- 個別 md は `## Request` (`requestId` を含む) + `## RequestData` + `## ResponseData` + `## エラー条件` + `## 制約` の 2 段構造 (例: `docs/server/hisui_requests/HisuiCreateOutput.md`)。
- 識別子はクライアント指定の一意名 (`outputName` / `subscriberId` 等)。
- リクエストハンドラのディスパッチは `src/obsws/message.rs:175` の `match request_type.as_str()` で、`HisuiCreate*` 系は `src/obsws/coordinator.rs:658-661` 等で処理。RequestBatch（op=8）は `src/obsws/coordinator.rs:376` の `handle_request_batch` が全リクエストを逐次 dispatch する設計で、明示的な許可リストは存在しない (= 通常 dispatch に乗れば自動的に RequestBatch 対応となる)。state file 永続化対象だけが `src/obsws/coordinator.rs` 末尾の `is_state_persisted_request` (line 1138 周辺) で別管理されている。
- エラーコード定数は `src/obsws/protocol.rs:44-58` の `REQUEST_STATUS_*`。本 issue で使うのは `MISSING_REQUEST_FIELD (300)` / `INVALID_REQUEST_FIELD (400)` / `RESOURCE_NOT_FOUND (601)` / `RESOURCE_ALREADY_EXISTS (602)` / `RESOURCE_ACTION_NOT_SUPPORTED (606)`。
- リクエストハンドラからの状態反映は `src/obsws/session/output.rs:131-137` の `update_program_mixers` 経由で `VideoRealtimeMixerUpdateConfigRequest` を送り、**`input_tracks` を全置換**する設計 (`src/mixer/video.rs:509-510`)。`output_plan.video_mixer_input_tracks` は `src/obsws/output_plan.rs:89-159` の `build_composed_output_plan` 内で `source_plans.iter().zip(active_scene_inputs.iter())` の `filter_map` クロージャ 1 つから組み立てられ、scene の input から都度再計算される。
- `build_composed_output_plan` の呼び出し元は 2 箇所: `src/obsws/server.rs:315` (初期化時) と `src/obsws/coordinator.rs:844` (`rebuild_program_output` シーン切替時)。両方とも旧設計の `text_overlay_track_id: Option<TrackId>` 引数を渡している。新設計で削除対象 (§3 末尾「旧実装の撤去対象」)。
- `VideoRealtimeMixer` の RPC は既存 enum `VideoRealtimeMixerRpcMessage` (`src/mixer/video.rs:222-230`) で `UpdateConfig` / `Finish` の 2 バリアントを持つ。`register_rpc_sender` の呼び出しは `src/mixer/video.rs:79-84`。`ProcessorHandle::register_rpc_sender` は同一 processor に対する 2 回目の登録を一律拒否する設計 (`src/media_pipeline.rs:535-545` の `RegisterProcessorRpcSenderError::AlreadyRegistered`)。本 issue の RPC 統合方針はこの制約に従う。

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
| premultiplied vs straight alpha | **premultiplied (Prgb32)**。hisui の `blend_component` (`src/mixer/video.rs:1136-1148`) は straight 前提のため、I420A 化前に **straight 復元** (`A > 0 ? (RGB * 255 + A / 2) / A : 0` を 4 チャネル各々に適用) を mixer のテキストレイヤモジュール内のヘルパーで行う |
| 採用する shiguredo_libyuv 関数 | `argb_to_i420_alpha` (`shiguredo_libyuv-2026.1.0/src/convert.rs:5216`、ARGB バイト順入力 = 上記 straight 復元後のバッファをそのまま渡せる) |
| `FontData` / `FontFace` / `Font` / `Image` / `Context` の `Send + Sync` | 明示 `impl` なし。内部型は `Vec<u8>` / `Arc<Vec<u8>>` / プリミティブのみで auto trait による Send + Sync 成立。**旧実装 (`src/mixer/text_overlay.rs`) で `Arc<raden::FontFace>` を `tokio::spawn` 経路に乗せて稼働実績あり** = 実証済み。新設計でも `VideoRealtimeMixer` を `MediaPipeline` 経由で `tokio::spawn` する経路で問題なく動く |
| Cranelift JIT 初回コンパイル遅延 | `PipelineRuntime` がパイプラインキャッシュを持ち、同一パラメータの関数は再コンパイルしない。**warm-up は `TextOverlayLayer::new` の中で 1 回実行する** (`PipelineRuntime::new()` 直後、`Image` を作って空文字列の `fill_text` を 1 回呼ぶ)。mixer の `wait_subscribers_ready()` より前に終わるため、出力 cadence への影響なし。実遅延は実装時に計測 |
| glyph 不在時の挙動 | raden 内部の `src/api/context.rs:1153-1157` (= `raden` クレートの内部ソース、hisui のパスではない) で `glyph_id == 0` ならアドバンスのみ進めて描画スキップ (silent skip)。tofu は出ない。本 issue はこの既定挙動のまま透過 I420A レイヤに含める (エラーは返さない) |
| フォントロード API | `FontData::from_file(path: &str) -> Result<FontData, FontError>` → `FontFace::from_data(&font_data, index: u32) -> Result<FontFace, FontError>` (TTC 単体は `index = 0`) → `Font::from_face(&face, size: f64) -> Font` |
| 描画コンテキスト構築 | `PipelineRuntime::new()` (processor あたり 1 回) → `Context::new(&mut image, &mut runtime)` (描画呼び出しごと)。`Context::end()` で締める |

### 2. フォント

#### CLI 引数

- `--font-search-root <dir>`: フォント探索ルート (絶対パス必須)。サーバはこの配下のファイルのみ参照する。
- `--default-font <fontName>`: 省略時のデフォルトフォント名 (例: `PublicSans-Regular.ttf`)。

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
- テスト用フォントとして Public Sans Regular (SIL Open Font License 1.1、USWDS 配布版) を `testdata/fonts/PublicSans-Regular.ttf` に配置する。

### 3. 描画 API (内部)

#### VideoRealtimeMixer 内部レイヤとしての統合

- テキストオーバーレイ機能は `VideoRealtimeMixer` の **内部レイヤ** として組み込む。`MediaPipeline` 上で独立した processor (`TextOverlayProcessor`) を spawn する形は採らない。これは以下の理由による:
  - リアルタイム合成専用機能で、別 mixer や別経路からの再利用シナリオが現状ない。
  - 別 processor 構成は「`VideoRealtimeMixer` の最終 InputTrack として `z = i32::MAX` で接続」する必要があり、`InputTrack.z` の特殊値予約というクライアント API への漏れ出しが発生する。
  - 別 processor 構成は subscribe/publish の channel オーバーヘッドと中間バッファの追加コピーを伴う。
- 機能有効時は `VideoRealtimeMixer` 構造体に `text_overlay_config: Option<TextOverlayConfig>` フィールドを追加し、`VideoRealtimeMixer::run()` の冒頭で `TextOverlayLayer::new(canvas_width, canvas_height, frame_rate, config)?` を構築する。構築結果は `VideoRealtimeMixerRunner::text_overlay_layer: Option<TextOverlayLayer>` フィールドに保持する。canvas 単一・起動時固定のため、サーバ稼働中ずっと同じレイヤを使う (シーン切替で再生成しない)。
- `TextOverlayLayer::new` の中で warm-up (`PipelineRuntime::new()` → 空文字列の `fill_text` 1 回) を実行する。`wait_subscribers_ready().await?` より前に終わるため、初回フレーム出力 cadence への影響なし。
- 機能無効時は `text_overlay_layer = None`。obsws ハンドラ側で機能無効を検出して `RESOURCE_ACTION_NOT_SUPPORTED (606)` を返す (§4 参照)。
- `text_overlay_config` 伝播経路: `--font-search-root` / `--default-font` (`src/subcommand_server.rs`) → `ObswsSessionState::text_overlay_config` (現存、`src/obsws/state.rs:805`) → `start_mixer_processors` (新規追加引数 `text_overlay_config: Option<TextOverlayConfig>`) → `VideoRealtimeMixer.text_overlay_config` フィールド。`VideoRealtimeMixerUpdateConfigRequest` には `text_overlay_config` フィールドを **追加しない** (シーン切替で `update_config` が走っても `text_overlay_layer` は温存される設計)。
- lifetime: `TextOverlayLayer` (raden の `PipelineRuntime` / font cache / cached I420A バッファを内包) の Drop は `VideoRealtimeMixer` の Drop と同一タイミング。`Finish` RPC 受信時に明示的な後始末は不要。

#### compose_frame での合成

- `compose_frame` (`src/mixer/video.rs:914` の **自由関数**、`VideoRealtimeMixer::compose_frame` メソッドではない) のシグネチャに `text_overlay_layer: Option<&mut TextOverlayLayer>` 引数を追加する。呼び出し元 `handle_output_tick` (`src/mixer/video.rs:406-413`) が `self.text_overlay_layer.as_mut()` を渡す。
- `compose_frame` 内では `RealtimeI420Canvas` を構築して `draw_order` ループで一般 input track を描画した後、`text_overlay_layer` が `Some` かつ overlay が 1 つ以上ある場合に、テキストレイヤが生成した cached I420A バッファ (`Arc<VideoFrame>` で I420A 形式) を `RawVideoFrame::from_video_frame` 経由で `RealtimeI420Canvas::draw_frame_clipped(0, 0, &frame)` で合成する。「最上位レイヤ」 という性質は z 値ではなく、この **`draw_order` ループ後の追加合成段** で表現する。
- `InputTrack.z` (`src/mixer/video.rs:138`) と `ObswsVideoMixerInputTrack.z` (`src/obsws/output_plan.rs:24`) は **既に `i32` で揃っている** (本ブランチ内のコミット `47d78d15` で旧 64bit 前提を解消済み)。本 issue では現状の `i32` を維持し、`InputTrack.z` に予約値を持たせない (= クライアントは i32 全域を z として指定可能)。`x` / `y` は `InputTrack` 側 `isize` / `ObswsVideoMixerInputTrack` 側 `i64` のまま維持する (本 issue のスコープ外)。
- 複数テキストオーバーレイ間の z は `TextOverlayLayer` 内部でソートして 1 枚の I420A に重ね合わせる段で解決する。`VideoRealtimeMixer.input_tracks` には現れない。

#### 内部 RPC

- **既存 `VideoRealtimeMixerRpcMessage` enum (`src/mixer/video.rs:222-230`) にバリアントを追加する形** で統合する。`ProcessorHandle::register_rpc_sender` は同一 processor に対する 2 回目の登録を一律拒否する仕様 (`src/media_pipeline.rs:535-545` の `RegisterProcessorRpcSenderError::AlreadyRegistered`) のため、別 sender を register する案は採れない。
- 追加するバリアント:
  - `TextOverlayAdd { request: TextOverlaySpec, reply_tx: oneshot::Sender<Result<(), TextOverlayError>> }`
  - `TextOverlayUpdate { name: String, patch: TextOverlayPatch, reply_tx: oneshot::Sender<Result<(), TextOverlayError>> }`
  - `TextOverlayRemove { name: String, reply_tx: oneshot::Sender<Result<(), TextOverlayError>> }`
  - `TextOverlayList { reply_tx: oneshot::Sender<Vec<TextOverlayState>> }`
- obsws ハンドラ側 (`src/obsws/coordinator/text_overlay.rs`) は `pipeline_handle.get_rpc_sender::<UnboundedSender<VideoRealtimeMixerRpcMessage>>(&ProcessorId::new("program:video_mixer"))` で sender を取得し、上記バリアントで送信する。
- `TextOverlayError` のバリアントと REQUEST_STATUS マッピングは §4 のエラー対応表に集約する。
- `TextOverlayPatch` は Update 用で全フィールド `Option<T>`、`None` = 省略 = 現状維持。JSON 上の `null` 受信は `INVALID_REQUEST_FIELD` として扱う (Create / Update 共通)。
- `TextOverlayState` は `HisuiCreateTextOverlay` の全属性 (`textOverlayName` / `text` / `x` / `y` / `fontSize` / `fontColor` / `fontName` / `z`) を保持し、`List` の戻り値および JSON 化される。
- 並列性: `VideoRealtimeMixerRunner::run` (`src/mixer/video.rs:353-374`) の `tokio::select!` メインループが既存 `rpc_rx` を単一 task で順次処理する設計のため、テキストオーバーレイ系バリアントも自動的に同一 task で順次処理される。同名 overlay の Add と Remove が同時送信された場合は受信順 (FIFO) で処理する。
- 描画ブロック注意: `compose_frame` は同期実行のため、描画中は次の RPC を待たせる。raden 描画の所要時間は文字数に比例するため、上限値 (`OVERLAY_LIMIT = 1024` × `text` 65536 バイト × 1024 行) で 1 フレーム描画コストが cadence (例: FPS=30 で 33ms) 内に収まることを実装時に計測する。超過した場合は描画 (raden + I420A 変換) を `tokio::task::spawn_blocking` でバックグラウンドに逃がす形へ切り替え、cached I420A の差し替えを次の `compose_frame` 呼び出し時に反映する設計に変更する。

#### 描画フロー

- raden の `Image (PixelFormat::Prgb32)` に全 overlay を z 順に重ね描き (premultiplied ARGB バッファ) → straight 復元ヘルパー (§1 調査結果参照) で straight ARGB へ戻す → `shiguredo_libyuv::argb_to_i420_alpha` で I420A に変換し、`TextOverlayLayer.cached_frame: Option<Arc<VideoFrame>>` に保持する。次の `compose_frame` 呼び出し時に `RawVideoFrame::from_video_frame(Arc::clone(&cached_frame))` で取り出して `RealtimeI420Canvas::draw_frame_clipped(0, 0, &frame)` で合成する。
- `TextOverlayLayer` のフィールド構成 (実装の指針):
  - `config: TextOverlayConfig` (canvas / font 設定、不変)
  - `overlays: BTreeMap<String, TextOverlaySpec>` (overlay 名 → 仕様、z は仕様内に含む)
  - `next_auto_z: i32` (z 省略時の auto z 解決、Create 時に `現在登録済みの z の最大値 + 1` を割り当てる。Update で z を変更しても再計算しない (= auto z 戻しはサポートしない))
  - `font_cache: HashMap<PathBuf, Arc<raden::FontFace>>` (フォント解決のキャッシュ)
  - `pipeline: raden::PipelineRuntime` (JIT キャッシュ含む、warm-up 済み)
  - `image: raden::Image` (描画用 Prgb32 バッファ、canvas サイズで 1 回確保して再利用)
  - `cached_frame: Option<Arc<VideoFrame>>` (I420A 形式、dirty=false の間はこれを再利用)
  - `dirty: bool` (次回描画要否)
- `dirty` を `true` にする条件: Add / Update / Remove のいずれかを受信したとき (Update は patch が空でも dirty を立てる、簡潔性優先)。canvas サイズは起動時固定のため `dirty` 化トリガにならない。
- 描画タイミング: `compose_frame` の追加合成段に入る直前 (`draw_order` ループ後) に、`dirty = true` なら raden 描画 → straight 復元 → I420A 変換 → `cached_frame` 更新 → `dirty = false` を実行する。
- `overlays.is_empty()` (= 全 overlay が空) の場合は描画も追加合成も省略する。`cached_frame` が `Some` のままでも、合成段で `overlays.is_empty()` をチェックして早期 return する (cached_frame を残しておくのは、空 → 1 件以上 → 空 と切り替わるシナリオでも前回の bytes を再利用しないようにするため)。
- `text_overlay_layer` が `None` (機能無効) の場合は、`compose_frame` の追加合成段全体をスキップする。
- メモリ占有量 (1920x1080 canvas、機能有効時):
  - `image (Prgb32)`: 1920 × 1080 × 4 = **約 8.3 MB** (再利用、常時保有)
  - `cached_frame (I420A)`: Y plane 1920 × 1080 + U/V planes 各 (1920/2) × (1080/2) + A plane 1920 × 1080 = 約 **5.0 MB** (再利用、常時保有)
  - straight 復元用の中間バッファ (描画呼び出しのスタックで alloc → drop、約 8.3 MB の `Vec<u8>`、描画中のみ)
  - 合計常時保有 = 約 13.3 MB / mixer (= サーバあたり)。

#### モジュール構成 (実装の指針)

- `src/mixer/text_overlay.rs` (旧設計、1316 行) は **完全削除**。
- `src/mixer/video.rs` の中で `pub mod text_overlay;` を宣言し、`src/mixer/video/text_overlay.rs` を参照する (Rust 2018+ のパス規則。`mod.rs` は採用しない、hisui のスタイル踏襲)。
- 内部分割の指針: 1 ファイル 500 行程度を目安に。 `src/mixer/video/text_overlay.rs` をエントリポイントとし、その中で `pub mod layer;` `pub mod validate;` を宣言する形で `src/mixer/video/text_overlay/layer.rs` / `src/mixer/video/text_overlay/validate.rs` に分割する。
- 旧 → 新シンボル移行マッピング (`src/mixer/text_overlay.rs` の各シンボルの行き先):

| 旧シンボル (`src/mixer/text_overlay.rs`) | 新配置 | 備考 |
|---|---|---|
| `TextOverlayProcessor` 構造体 + `run` | (削除) | 独立 processor 不要 |
| `TEXT_OVERLAY_PROCESSOR_ID` / `TEXT_OVERLAY_TRACK_ID` 定数 | (削除) | publish しないため不要 |
| `MAX_NOACKED_COUNT` 定数 | (削除) | 独立 publish 経路の back-pressure 用なので不要 |
| `ProcessorState` 構造体 + 内部メソッド | `src/mixer/video/text_overlay/layer.rs::TextOverlayLayer` にリネーム + 移植 | `compose_frame` から呼ばれる構造に変える |
| `TextOverlayConfig` | `src/mixer/video/text_overlay.rs` | obsws / subcommand_server から参照 (公開 API、現状維持) |
| `TextOverlayError` enum + Display 実装 | `src/mixer/video/text_overlay.rs` | obsws から参照 (公開 API) |
| `TextOverlayRpcMessage` enum | (削除、`VideoRealtimeMixerRpcMessage` のバリアントに統合) | 上記「内部 RPC」節参照 |
| `TextOverlaySpec` / `TextOverlaySpecInput` / `TextOverlayPatch` / `TextOverlayState` | `src/mixer/video/text_overlay.rs` | obsws / pbt から参照 (公開 API) |
| `OVERLAY_LIMIT` / `TEXT_MAX_BYTES` / `TEXT_MAX_LINES` 定数 | `src/mixer/video/text_overlay.rs` | pbt から参照、 `pub` 維持 |
| 検証関数 (`validate_text` / `validate_font_size` / `validate_font_name_and_resolve_path` / `apply_patch`) | `src/mixer/video/text_overlay/validate.rs` | pbt から参照、 `pub(super)` または `pub(crate)` |
| `unpremultiply_argb` ヘルパー | `src/mixer/video/text_overlay/layer.rs` (private) | layer 内のみで使用 |
| 既存 `#[cfg(test)] mod tests` | テスト対象に応じて `src/mixer/video/text_overlay.rs` / `layer.rs` / `validate.rs` の各モジュールに分散 | |

#### 旧実装の撤去対象 (チェックリスト)

新設計への切替に伴い、本 issue の実装で以下の旧実装を **同じブランチ内で確実に削除・書き換え** する。完了条件のチェックリストでも再掲する。

- `src/mixer/text_overlay.rs` (1316 行、ファイル全体) を削除し、上記マッピングに従い `src/mixer/video/text_overlay.rs` + サブモジュールに移植する。
- `src/mixer/video.rs` の `VideoRealtimeMixer` 構造体に `text_overlay_config: Option<TextOverlayConfig>` フィールドを追加し、`VideoRealtimeMixerRunner` に `text_overlay_layer: Option<TextOverlayLayer>` フィールドを追加する。`run()` 冒頭で `TextOverlayLayer::new()` を呼ぶ。`compose_frame` 自由関数の引数に `text_overlay_layer: Option<&mut TextOverlayLayer>` を追加する。`VideoRealtimeMixerRpcMessage` enum に 4 バリアント (`TextOverlayAdd` / `TextOverlayUpdate` / `TextOverlayRemove` / `TextOverlayList`) を追加し、`VideoRealtimeMixerRunner::handle_rpc_message` で処理する。
- `src/obsws/output_plan.rs` の `build_composed_output_plan` から `text_overlay_track_id: Option<TrackId>` 引数を削除し、関数末尾の「`if let Some(track_id) = text_overlay_track_id { ... push z = i32::MAX ... }`」ブロックを削除する。テスト `build_composed_output_plan_skips_dormant_inputs` の引数 `None,` も削除する。
- `src/obsws/server.rs:314-348` の `TextOverlayProcessor` spawn ブロックと `text_overlay_track` 解決ロジックを削除する。代わりに、 `start_mixer_processors` (`src/obsws/session/output.rs` ほか) に `text_overlay_config: Option<TextOverlayConfig>` を渡す経路を追加する。
- `src/obsws/coordinator.rs:878-887` (`rebuild_program_output`) の `text_overlay_track` 計算と `build_composed_output_plan` への引数渡しを削除する。
- `src/obsws/coordinator/text_overlay.rs` のハンドラ 4 本を新設計に書き換える。具体的には: (a) `text_overlay_sender()` ヘルパーを「`get_rpc_sender::<UnboundedSender<VideoRealtimeMixerRpcMessage>>(&ProcessorId::new("program:video_mixer"))`」に変更、(b) `parse_optional_z` の `i32::MAX` 拒否バリデーション (`src/obsws/coordinator/text_overlay.rs:707-718` 周辺) を削除、(c) 各ハンドラが新バリアント (`VideoRealtimeMixerRpcMessage::TextOverlayAdd` 等) で送信するよう書き換え。
- `docs/internals/mixer.md:145-151` の旧 `### InputTrack.z の予約値` 節を **削除** し、新設計の「`VideoRealtimeMixer` のテキストオーバーレイレイヤ」節に差し替える (§5 参照)。
- `docs/server/hisui_requests/HisuiCreateTextOverlay.md` / `HisuiUpdateTextOverlay.md` / `HisuiRemoveTextOverlay.md` / `HisuiListTextOverlays.md` の旧設計記述 (`i32::MAX` 予約値・auto z 戻し非サポート等) を新設計に合わせて書き換える (§5 参照)。
- `docs/obsws/PROTOCOL_STATUS.md:1064-1086` (テキストオーバーレイ独自拡張節) の「最終 InputTrack として z = `i32::MAX` で合成」 記述を新設計の「`VideoRealtimeMixer` 内部のテキストオーバーレイレイヤとして最上位合成」 に書き換える (§5 参照)。
- `pbt/tests/prop_text_overlay.rs` の import を旧パスから `hisui::mixer::video::text_overlay::*` に書き換える (テスト本体はそのまま再利用可能)。
- `src/obsws/session/tests.rs` のテキストオーバーレイ関連ヘルパー (`create_initialized_coordinator_with_text_overlay` 等、line 3950 以降) を新設計に書き換える: `TextOverlayProcessor` spawn を呼ばず、`start_mixer_processors` 相当に `text_overlay_config` を渡す形にする。`parse_optional_z_rejects_reserved_i32_max` テスト (旧バリデーション) を削除する。

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
| `textOverlayName` | string | 必須 | サーバ全体で一意。空文字は `INVALID_REQUEST_FIELD` |
| `text` | string | 必須 | `\n` 改行可、最大 `TEXT_MAX_BYTES = 65536` バイト・最大 `TEXT_MAX_LINES = 1024` 行。空文字 (`""`) は `INVALID_REQUEST_FIELD` (`MISSING_REQUEST_FIELD` ではない) |
| `x` | integer (i64 範囲) | 必須 | キャンバス絶対座標 X (左上原点、px)。負値・キャンバス外は許容 (テキストレイヤ内でクリップ)。i64 範囲外は `INVALID_REQUEST_FIELD` |
| `y` | integer (i64 範囲) | 必須 | キャンバス絶対座標 Y (左上原点、px)。同上 |
| `fontSize` | integer | 必須 | px。`1` 以上 `canvas_height` 以下。範囲外は `INVALID_REQUEST_FIELD` |
| `fontColor` | string | - | 正規表現 `^#[0-9A-Fa-f]{6}([0-9A-Fa-f]{2})?$`、default `#FFFFFFFF` |
| `fontName` | string | - | `--font-search-root` 配下のファイル名 (拡張子付き)、default は `--default-font` |
| `z` | integer (i32 範囲) | - | overlay 間 z-order、i32 全域指定可能 (`i32::MAX` 含む、旧設計の予約値は廃止)。省略時は **自動割り当て** = `現在登録済みの z の最大値 + 1` (= 後勝ち)。i32 範囲外は `INVALID_REQUEST_FIELD` |

ResponseData: なし。

**共通**: 全てのフィールドで JSON `null` を受信した場合は `INVALID_REQUEST_FIELD (400)` を返す (省略 = キー自体が存在しない、と `null` を区別する)。型不一致 (例: 数値が必要な箇所で文字列) も同コードを返す。

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
| `LimitExceeded` | `RESOURCE_ACTION_NOT_SUPPORTED (606)` | Create | `OVERLAY_LIMIT` 超過 |
| `Disabled` | `RESOURCE_ACTION_NOT_SUPPORTED (606)` | 全 | テキストオーバーレイ機能無効 |

#### 共通制約

- WebSocket / データチャネル両方で利用可能 (`HisuiCreateOutput` と同じ)。
- RequestBatch（op=8）対応: `src/obsws/coordinator.rs:658` 周辺の `dispatch_request` の match 分岐に 4 メソッドを追加すれば、 `handle_request_batch` (line 376) が自動的に dispatch するため、追加 allowlist は不要。

### 5. ドキュメント

旧設計時に既に作成済みのドキュメントを **新設計に合わせて書き換える**。新規追加は `docs/server/hisui_requests/README.md` への節追加のみ。

- `docs/server/hisui_requests/HisuiCreateTextOverlay.md` / `HisuiUpdateTextOverlay.md` / `HisuiRemoveTextOverlay.md` / `HisuiListTextOverlays.md` (既存 4 本、旧設計準拠) を新設計に合わせて書き換える:
  - `z` フィールドの「`i32::MIN..=i32::MAX - 1`」「`i32::MAX` は内部予約値のため指定不可」「Auto z への再戻しはサポートしない」 等の **旧設計記述を全削除** し、「`z` は i32 全域指定可、省略時は `現在登録済みの z の最大値 + 1` (auto z)」 に統一する。
  - List メソッドの ResponseData の「`HisuiCreateTextOverlay` で z 省略時に自動割り当てされた値も含む」は新設計でも維持 (auto z 解決後の値を返す)。
  - 共通エラーケース (null / 型不一致 / 空文字) の記述を §4 共通の方針に揃える。
  - 構造 (`## Request` / `## RequestData` / `## ResponseData` / `## エラー条件` / `## 制約`) は `HisuiCreateOutput.md` を踏襲。
- `docs/server/hisui_requests/README.md` に「## テキストオーバーレイ」節を追加する (新規)。節冒頭に既存節 (例: 「Output 管理」) と同様の前提条件行「WebSocket / データチャネル両対応。RequestBatch（op=8）に対応。」を入れる。
- `docs/obsws/PROTOCOL_STATUS.md:1064-1086` (テキストオーバーレイ独自拡張節、旧設計) の「最終 InputTrack として z = `i32::MAX` で合成され」 記述を **削除** し、「`VideoRealtimeMixer` 内部のテキストオーバーレイレイヤとして最上位合成され」 に書き換える。
- `docs/obsws/STATE_FILE.md` の永続化対象列挙 (line 7-8) にテキストオーバーレイが永続化対象外である旨を明記する (例: 「テキストオーバーレイは揮発的で `--state-file` には保存されません (再起動でクリアされます)」)。
- `docs/internals/mixer.md:145-151` (旧 `### InputTrack.z の予約値` 節、旧設計準拠) を **削除** し、同箇所に「`VideoRealtimeMixer` のテキストオーバーレイレイヤ」節を新設する。新節では以下を明記:
  - (a) mixer 内部レイヤとして `VideoRealtimeMixerRunner.text_overlay_layer` フィールドに保持されていること。
  - (b) `compose_frame` の追加合成段 (一般 InputTrack の `draw_order` ループ後) で最上位合成される順序の規約。
  - (c) `InputTrack.z` には予約値を持たないこと (= クライアントは i32 全域を z として指定可能)。
  - (d) テキストオーバーレイ間の z は `TextOverlayLayer` 内部でソートして 1 枚の I420A に重ね合わせる段で解決し、 `VideoRealtimeMixer.input_tracks` には現れないこと。
- `docs/internals/architecture_overview.md` / `docs/internals/processor_id.md`: 触らない (`TextOverlayProcessor` は新設計で存在しないため、 `processor_id.md` への記載追加は不要。 `architecture_overview.md` も mixer 内部の話なので変更不要)。

### 6. スコープ

- リアルタイム合成 (obsws 経由) のみを対象とする。`VideoRealtimeMixer` の出力 `program:mixed_video` を購読する `mp4_output` / `hls_output` / `mpeg_dash_output` 等を通せば、テキスト描画済みの動画ファイルが結果として得られる (字幕応用 0012 の主用途もこの経路で実現される)。
- 録画合成 (`src/sora/recording_video_mixer.rs` / `src/sora/recording_subcommand_compose.rs`) は本 issue では一切触らない。

#### 将来案メモ (本 issue では対応しない)

- `MediaFrame` enum (`src/media.rs:8`) に `Text` バリアントを生やす案。 文字起こし等で「映像/音声トラックを解析しつつ途中でテキストフレームを挿入する」 ような pipeline を MediaPipeline 上で自然に表現できる可能性がある。 本 issue は video mixer の内部レイヤとして組み込む案で確定済みだが、 0012 (ML 推論) や 0014 (ML 結果出力) と組み合わせる将来の設計で再検討する余地がある。

## 完了条件

- 4 メソッドが obsws (WebSocket / データチャネル両方) と RequestBatch（op=8）から動作する。
- `--font-search-root` / `--default-font` の CLI 引数が動作する (両方未指定で機能無効・正常起動、片方のみで起動失敗、両方指定で起動時検証成功なら `VideoRealtimeMixer::run()` 冒頭で `TextOverlayLayer::new()` が成功、検証失敗で起動失敗。リクエスト時の `fontName` で `..` 含む / シンボリックリンク経由でルート外を指すパスは拒否)。片方のみ未指定の起動失敗時のエラーメッセージは英語で記述する (CLAUDE.md「ログメッセージは全て英語」に従う、例: `--font-search-root and --default-font must be specified together`)。
- **旧実装の撤去** (§3 末尾「旧実装の撤去対象 (チェックリスト)」に列挙した全項目が完了している):
  - `src/mixer/text_overlay.rs` (1316 行) が削除され、`src/mixer/video/text_overlay.rs` + サブモジュールに移植されている
  - `src/mixer/video.rs` の `VideoRealtimeMixer` / `VideoRealtimeMixerRunner` / `VideoRealtimeMixerRpcMessage` / `compose_frame` が新設計に追従している
  - `src/obsws/output_plan.rs` から `text_overlay_track_id` 引数と関連ロジックが削除されている
  - `src/obsws/server.rs:314-348` の `TextOverlayProcessor` spawn ブロックが削除され、 `start_mixer_processors` に `text_overlay_config` を渡す経路に置き換わっている
  - `src/obsws/coordinator.rs:878-887` の `text_overlay_track` 計算が削除されている
  - `src/obsws/coordinator/text_overlay.rs` のハンドラ 4 本が `VideoRealtimeMixerRpcMessage` の新バリアントに送信する形に書き換わり、 `parse_optional_z` の `i32::MAX` 拒否バリデーションが削除されている
  - `pbt/tests/prop_text_overlay.rs` の import が `hisui::mixer::video::text_overlay::*` に書き換わっている
  - `src/obsws/session/tests.rs` のテキストオーバーレイヘルパーが新設計に書き換わり、 `parse_optional_z_rejects_reserved_i32_max` テストが削除されている
- ドキュメント整備:
  - `docs/server/hisui_requests/` 個別 md 4 本: 既存ファイル (旧設計準拠) を新設計に書き換える (§5 参照)
  - `docs/server/hisui_requests/README.md` への節追加 (新規、前提条件行込み)
  - `docs/obsws/PROTOCOL_STATUS.md:1064-1086` の既存節を新設計に書き換える
  - `docs/obsws/STATE_FILE.md` 永続化対象外追記
  - `docs/internals/mixer.md:145-151` の旧予約値節を削除し、 新設計の「`VideoRealtimeMixer` のテキストオーバーレイレイヤ」 節に差し替える (§5 参照)
- テスト (`testdata/fonts/PublicSans-Regular.ttf` は本ブランチに配置済み、新設計でもそのまま利用):
  - `src/obsws/session/tests.rs` のテキストオーバーレイヘルパー (line 3950 以降) を新設計に書き換えた上で、4 メソッド往復 + 全エラーケース (`AlreadyExists` / `NotFound` / `InvalidFontName` / `FontResolveFailed` / `InvalidColor` / `InvalidFontSize` / `InvalidText` / `LimitExceeded` / `Disabled` / `MISSING_REQUEST_FIELD`) の検証が動く。
  - `src/mixer/video/text_overlay/layer.rs` (`TextOverlayLayer`) の単体テストで、 (a) 黒一色 I420 input track 1 つとテキストオーバーレイ 1 つを mixer に与え、 1 フレーム取得して、 (b) テキスト bbox 内 (`x..x + fontSize × 文字数 × 1.5`, `y..y + fontSize × 行数 × 1.5` の矩形目安) で Y プレーン値が黒 (16) でない画素が存在することを assert する。 (c) raden → I420A 変換後の A プレーン非ゼロ領域が指定 x/y 近傍に収まっていることも検証する。
  - `pbt/tests/prop_text_overlay.rs` を新パス (`hisui::mixer::video::text_overlay::*`) で動く形に書き換えた上で、 x/y/fontSize の境界値 PBT を維持する (`text=""` 拒否 / `x = i64::MIN / i64::MAX 近傍` / `fontSize = 1` / `fontSize = canvas_height` / 改行のみ / Unicode 制御文字 / フォント不在文字 / `text` 長境界 / 行数境界 / `z = i32::MAX` 含む i32 全域 OK)。
- shiguredo-issues 規約: コード本体・docstring・テストコメント・`CHANGES.md` ・ docs に issue 番号 (`0013`) や issue ファイル名 (`0013-feature-add-text-overlay-rendering.md`) への参照が残っていないこと (`grep -rn '0013' src/ docs/ CHANGES.md pbt/ testdata/` で 0 件、commit メッセージは対象外)。
- 0012 等の他 issue に依存せず、本 issue 単独で動作確認・マージできる。
- **CHANGES.md** は本ブランチで既に追記済みの `[ADD]` エントリ (`## develop` 内、`obsws 経由でリアルタイム合成映像にテキストオーバーレイを描画できるようにする`) を **新設計に合わせて維持または微修正** する。新規追記ではない。現行エントリは既に「`VideoRealtimeMixer` の最上位レイヤとして合成する」「揮発的 (`--state-file` には保存されず、再起動でクリアされる)」 と新設計と整合する文言を含んでいるため、大幅な書き換えは不要。担当者欄 (`@<github-id>`) は実 GitHub ID で記載する (本リポジトリの一貫した担当者は `@sile`)。
