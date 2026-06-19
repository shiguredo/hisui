# 合成映像へのテキスト (字幕) 描画に対応する

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/add-text-overlay-rendering
- Polished:

## 目的

合成映像にテキストをオーバーレイ描画できる汎用基盤を提供する。応用例: ラベル・タイムスタンプ・任意テキストの表示、別 issue 0012 (candle / Whisper 文字起こし) の結果を字幕として映像に重ねて表示する、等。本 issue は描画プリミティブと obsws 経由の操作 API までを範囲とし、字幕特有のスタイル一式 (縁取り・背景帯) や時刻スケジューリング、特定入力への追従は本 issue では扱わない。

## 優先度根拠

- テキスト描画はラベル・タイムスタンプ・字幕など複数応用の前提となる汎用基盤で、単体で動作確認・マージが可能。
- ただし業務を止めている課題ではない。
- 以上から Medium。

## 現状

### 映像合成

- hisui の映像合成は I420 (YUV) 上で行う:
  - `src/video/canvas.rs` の `I420Canvas`。
  - `src/mixer/video.rs` の `VideoRealtimeMixer` が `compose_frame` / `blend_component` で I420A レイヤをブレンドして I420 を出力する。`InputTrack` の動的追加・削除は `UpdateConfig` RPC 経由で可能。
  - 録画合成側は `src/sora/recording_video_mixer.rs`。
  - 色空間変換・リサイズは shiguredo_libyuv を使用する。
- テキスト描画機能は無い。グリフをラスタライズして映像へ重ねる手段が存在しない。

### MediaPipeline / Processor

- `src/media_pipeline.rs` に MediaPipeline の processor 概念が確立されている (`register_processor` / `spawn_processor` / `ProcessorHandle` / `subscribe_track` / `publish_track` 等)。`VideoRealtimeMixer` も 1 processor として動く。
- `MediaFrame` / `RawVideoFrame` は内部に `Arc<VideoFrame>` を保持しており (`src/media.rs:9` / `src/video.rs:62`)、processor 間のフレーム受け渡しは Arc クローンのみでフレーム本体のメモリコピーは発生しない。

### obsws 独自拡張

- obsws には既に hisui 独自拡張リクエストが多数存在し、命名規則 `Hisui<Verb><Noun>` が確立されている (`HisuiCreateOutput` / `HisuiRemoveOutput` / `HisuiStartSoraSubscriber` / `HisuiGetWebRtcStats` 等)。
- 独自拡張のドキュメントは `docs/server/hisui_requests/<MethodName>.md` (個別ファイル) + `docs/server/hisui_requests/README.md` (一覧) + `docs/obsws/PROTOCOL_STATUS.md` (反映) の構造。
- 識別子はクライアント指定の一意名が一貫した慣わし (例: `outputName`, `subscriberId`)。

## 設計方針

### 1. 描画ライブラリ

- shiguredo/raden (https://github.com/shiguredo/raden) を採用する。Cranelift JIT ベースの CPU-only な 2D ベクターグラフィックスライブラリで、`fill_text(x, y, &Font, text)` でテキストを描画できる。CPU only のため GPU の無い CI 環境とも相性が良い。
- リスク (要管理): raden は公式 README で「実験的プロジェクトであり、API や内部実装は予告なく大幅変更されうる」と明記されている。依存バージョンを厳密固定 (hisui 方針) し、API 変更時の追従コストを織り込む。

### 2. フォント

- フォント本体は同梱しない (リポジトリ・バイナリのいずれにも入れない)。
- 起動時に CLI 引数で探索ルートとデフォルトフォントを指定する:
  - `--font-search-root <dir>`: フォント探索ルート (絶対パス必須)。サーバはこの配下のファイルのみ参照する。`canonicalize` 後にルート配下チェックで path traversal を遮断する。
  - `--default-font <fontName>`: 省略時のデフォルトフォント名 (例: `Roboto-Regular.ttf`)。`<root>/<fontName>` が起動時に解決可能であることを起動時に検証する。
- どちらかが未指定の場合はテキストオーバーレイ機能無効 (`HisuiCreateTextOverlay` を呼ばれたらエラー応答)。
- obsws リクエストは `fontName` (拡張子付きファイル名) で参照する。絶対パスは受け付けない (path traversal および機密ファイル参照を防ぐため)。
- 複数フォントの動的管理 API (`HisuiCreateFont` 等)・フォールバックチェーン・CJK 標準フォント提供は本 issue では扱わない (必要になれば別 issue)。
- テスト用フォントとして Roboto Regular (Apache 2.0、約 170KB) を `testdata/fonts/Roboto-Regular.ttf` に配置する。

### 3. 描画 API (内部)

- `VideoRealtimeMixer` には描画機能を直接生やさず、新規 `TextOverlayProcessor` を MediaPipeline 上の 1 processor として実装する (`MediaPipeline::register_processor` / `spawn_processor` 系に乗せる)。
- `TextOverlayProcessor` は最終キャンバス解像度の透過 I420A レイヤを 1 本の track として publish する (raden で RGBA に描画 → shiguredo_libyuv で I420A 変換)。
- `VideoRealtimeMixer` はその track を z-order 最上位の `InputTrack` として合成する (`UpdateConfig` 経由で動的追加・削除)。
- 位置指定は最終キャンバス上の絶対座標 (px) のみサポートする。これによりサイズ・位置の一貫性は最終キャンバス基準で自然に確保される。
- 特定入力への追従や相対座標指定は本 issue では扱わない (必要になれば mixer のシーン情報 broadcast 機構を別途検討)。
- 内部 RPC: `AddText` / `UpdateText` / `RemoveText` / `ListTexts` を `(canvasName, textOverlayName)` 単位で操作する。
- 静的テキスト時は描画済み `Arc<VideoFrame>` をキャッシュし、毎フレームの publish は Arc クローンのみとする。

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

#### 識別子・スコープ

- `textOverlayName` (string) はクライアント指定の一意名。`(canvasName, textOverlayName)` で一意 (異なる canvas に同名 OK)。
- canvas 削除時に紐づくテキストオーバーレイは自動削除する。

#### `HisuiCreateTextOverlay` requestData

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `textOverlayName` | string | 必須 | canvas 内で一意 |
| `text` | string | 必須 | `\n` 改行可 |
| `x` / `y` | integer | 必須 | 最終キャンバス絶対座標 (左上原点、px) |
| `fontSize` | integer | 必須 | px |
| `fontColor` | string | - | `#RRGGBB` / `#RRGGBBAA`、default `#FFFFFFFF` |
| `fontName` | string | - | `--font-search-root` 配下のファイル名 (拡張子付き)、default は `--default-font` |
| `canvasName` | string | - | canvas が複数あるときは必須 |
| `z` | integer | - | overlay 間 z-order、省略時は宣言順 (= 後勝ち) |

エラー条件:

- 同名既存: `RESOURCE_ALREADY_EXISTS`
- `fontName` 解決失敗 (ファイルなし / ルート外): `INVALID_REQUEST_FIELD`
- 必須フィールド欠落: `MISSING_REQUEST_FIELD`
- `canvasName` 未指定だが canvas 複数: `MISSING_REQUEST_FIELD`
- 指定 `canvasName` が存在しない: `RESOURCE_NOT_FOUND`
- テキストオーバーレイ機能無効 (`--font-search-root` 等が未指定): 既存定数があれば揃える、なければ新規定義する

#### `HisuiUpdateTextOverlay` requestData

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `textOverlayName` | string | 必須 | 対象識別子 (変更不可) |
| `canvasName` | string | canvas 複数時必須 | 対象 canvas (変更不可) |
| `text` / `x` / `y` / `fontSize` / `fontColor` / `fontName` / `z` | - | - | 送ったフィールドのみ部分更新、省略は現状維持 |

エラー条件: `RESOURCE_NOT_FOUND` (overlay or canvas) / `MISSING_REQUEST_FIELD` / `INVALID_REQUEST_FIELD` (fontName 解決失敗)。

#### `HisuiRemoveTextOverlay` requestData

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `textOverlayName` | string | 必須 | |
| `canvasName` | string | canvas 複数時必須 | |

エラー条件: `RESOURCE_NOT_FOUND` / `MISSING_REQUEST_FIELD`。

#### `HisuiListTextOverlays` requestData

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `canvasName` | string | - | 指定時はその canvas のみ、省略時は全 canvas |

responseData: `textOverlays` 配列 (各要素は Create の全属性 + `canvasName`)。

#### 共通制約

- WebSocket / データチャネル両方で利用可能 (`HisuiCreateOutput` と同じ)。
- RequestBatch (op=8) 対応 (`HisuiCreateOutput` と同じ)。

#### 永続化・イベント・スケジューリング (= 含めないもの)

- **永続化対象に含めない** (`--state-file` に保存しない)。テキストオーバーレイは揮発的な性質 (字幕・ラベル等の用途と整合)。クライアントが再起動を跨いで保持したい場合は `PersistentData` (KV ストア) に保存して再投入する。
- **`TextOverlayCreated` / `TextOverlayRemoved` / `TextOverlayUpdated` 等のイベントは出さない**。現状 Hisui 系メソッド (`HisuiCreateOutput` 等) も独自イベントを出していないため整合的。将来 Hisui 系リソースのイベント通知を統一する別 issue で一括対応する。
- **表示スケジューリング (`startAt` / `endAt` 等) を持たない**。`HisuiCreateTextOverlay` = 即表示、`HisuiRemoveTextOverlay` = 即削除のみ。スケジューリングは応用層 (字幕応用 issue 等) が時刻を見て `HisuiCreateTextOverlay` / `HisuiRemoveTextOverlay` を呼び分ける責務とする。

### 5. ドキュメント

- `docs/server/hisui_requests/HisuiCreateTextOverlay.md` 等、4 メソッド分の個別 md を作成する (既存 `HisuiCreateOutput.md` 等のフォーマットを踏襲)。
- `docs/server/hisui_requests/README.md` に「## テキストオーバーレイ」節を追加し、4 メソッドを表記する。
- `docs/obsws/PROTOCOL_STATUS.md` の独自拡張節に反映する。

### 6. スコープ

- リアルタイム (obsws 経由) のみを対象にする。
- 録画合成 (`src/sora/recording_video_mixer.rs` / `src/sora/recording_subcommand_compose.rs`) は本 issue では扱わない (要件が出てから別途検討)。

## 完了条件

- 上記 4 メソッドが obsws (WebSocket / データチャネル両方) と RequestBatch から動作する。
- `--font-search-root` / `--default-font` の CLI 引数が動作し、絶対パス / ルート外パスは拒否される (path traversal 対策が機能している)。
- ドキュメント (個別 md 4 本 / `README.md` / `PROTOCOL_STATUS.md`) が整備されている。
- `testdata/fonts/Roboto-Regular.ttf` を使った integration test レベルの動作検証がある (モック / スタブを使わない、規約遵守)。
- 0012 等の他 issue に依存せず、本 issue 単独で動作確認・マージできる。
- CHANGES.md の `## develop` に該当エントリを追記する。

## 解決方法

- raden で RGBA へ描画 → shiguredo_libyuv で I420A へ変換 → `TextOverlayProcessor` が透過 I420A レイヤを publish → `VideoRealtimeMixer` が z-order 最上位の `InputTrack` として合成、という流れで実装する。
- 詳細スコープ (テスト粒度、エラー文言、`HisuiListTextOverlays` の戻り値構造の細部、機能無効時のエラーコード割り当て) は `/polish-issue` での磨き上げ時に確定する。
