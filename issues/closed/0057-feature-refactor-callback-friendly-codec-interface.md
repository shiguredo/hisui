# エンコーダー / デコーダーのインターフェースを callback friendly に再設計する検討

- Priority: Medium
- Created: 2026-06-24
- Completed: 2026-06-26
- Model: Claude Opus 4.7
- Branch: feature/refactor-callback-friendly-codec-interface
- Prototype Branch:
- Prototype Commit:
- Polished: 2026-06-26
- Reporter: @sile
- Decision Owner: @sile

## 目的

`VideoEncoderInner` / `VideoDecoderInner` の内部出力経路 (中継キュー + `next_encoded_frame()` / `next_decoded_frame()` の pull) を見直し、下位ライブラリの非同期コールバックから上位パイプラインの次段へフレームを直接届けられる経路を設計確定する。

本 issue は **設計検討フェーズ** で、A / B / C / D-1 / D-2 / D-3 (もしくは新たな案) から **1 つを選び方針を本 issue に追記する**、または **どの案も採用基準を満たさないため「現状維持」を結論とする** ことをゴールとする。実装は別 issue で扱う。プロトタイプの取り扱いは「解決方法 §2」で規定する。

### 本 issue における「callback friendly」の定義

「下位ライブラリ (例: `shiguredo_nvcodec`) のエンコード/デコード完了コールバック内で得たフレームを、上位 `run()` ループ内の `Receiver` 1 段 (もしくは同等の 1 段) で受けて `TrackPublisher::send_media` に渡せる構造」を指す。ホップ数の上限を 1 段 (Sender → Receiver) とし、現状の「中継キュー + pull バッファ + run() の `drain_*_output`」の多段経路を廃止することを意味する。

### 用語

本 issue で使う用語の定義:

- **下位 ABI callback**: 下位ライブラリ (例: `shiguredo_nvcodec::FnEncodeHandler`) がエンコード完了時に呼び出す関数ハンドラ
- **上位 API callback**: hisui の `VideoEncoder` 等が外部から受け取る Sender / 関数ハンドラ (案 A/B/C で導入を検討するもの)
- **Sender** (無修飾): `tokio::sync::mpsc::Sender` / `UnboundedSender` を指す。`std::sync::mpsc::Sender` を指す場合は「`std::sync::mpsc::Sender`」と明示する
- **frame latency**: 「下位 ABI callback が呼ばれた瞬間 → `TrackPublisher::send_media` が呼ばれる瞬間」までの所要時間 (`tracing` の span で計測する)。`encode()` 呼出 → callback 到達は下位ライブラリ内部なので本 issue では計測対象外
- **Sink 抽象 (`futures::sink::Sink`) の不採用理由**: hisui は既に `tokio` でランタイムを揃えており、`futures` クレートの抽象を新規導入する正味の利点が薄いため棄却。再考トリガー: ランタイムを `tokio` 以外と相互運用する必要が出た時

### スコープ

本 issue は **Video エンコーダー / Video デコーダー** を主対象とする。Audio (`AudioEncoder` / `AudioDecoder`) は内部 API が同期 push 型 (`AudioEncoderInner::encode()` が `crate::Result<Vec<AudioFrame>>` を即時返す `src/encoder.rs:315-323`、`AudioDecoderInner::decode()` が `crate::Result<AudioFrame>` を即時返す `src/decoder.rs:230-238`) で、下位 ABI callback 経路が存在せず、本 issue の再設計動機 (非同期コールバック並列性回復) が成立しないため、**完全にスコープ外** とする。後続実装 issue の分割例にも Audio 系は含めない。

## 優先度根拠

Medium。

- 現時点で性能を実測した上での悪影響報告はなく、ユーザー報告由来の不具合でもない (定量根拠は本 issue のプロトタイプで取得する)。
- 一方で `NvcodecEncoder::encode()` は `flush()` 同期待ちを毎フレーム行わざるを得ない状態で、NVENC が本来持つ非同期パイプライン並列性を全く活かせていない (詳細は「現状」§参照)。これが Medium 維持の中核理由。
- 上流が今後も非同期コールバック寄りに進化する見込みがあり、追従コストが累積する。
- 設計検討フェーズで方向性を固めれば、後続の実装 issue を最小コストで進められる。

なお、採用案が「flush 撤廃を達成しない」場合 (本評価軸の 1 つに過ぎず必須ではない) は、本 issue の Medium 維持の中核理由 (NVENC 並列性回復) が形式的に失われる。その場合は「NVENC 並列性回復をどの後続経路で確保するか」または「優先度を Low に降格して別 issue で再起票する判断」を Decision Owner が本 issue 内に追記する。

## 現状

### 上位パイプラインは既に async push 型

`src/encoder.rs:615-661` の `VideoEncoder::run()` は `async fn` で `tokio::select!` + `tokio::sync::mpsc::unbounded_channel` を使用済み。出力は `output_tx: TrackPublisher` への `send_media()` で下流に push される (`drain_video_encoder_output` `src/encoder.rs:745-764`)。下流側は `subscribe_track` (`src/encoder.rs:621` / `src/decoder.rs:90`) で `Message` を `recv().await` する完全な push / channel 型 (`tokio::sync::broadcast` ベース)。

つまり、「上位パイプライン全体が同期 pull」ではない。同期 pull が残っているのは以下の多段経路だけ:

- Encoder: 下位 ABI callback → (nvcodec のみ) `encoded_queue: Arc<Mutex<VecDeque>>` → 各 inner の `output_queue: VecDeque<VideoFrame>` → `VideoEncoderInner::next_encoded_frame()` → `VideoEncoder.encoded` (`src/encoder.rs:435`) → `VideoEncoder::poll_output()` (`src/encoder.rs:734-742`)
- Decoder: 同様の構成で `decoded_queue` → `output_queue` → `next_decoded_frame()` → `decoded` (`src/decoder.rs:335`) → `poll_output()` (`src/decoder.rs:422-430`)

この多段経路を Sender 1 段 (もしくは内部キュー上限ベースのセルフペーシング) に短絡できるかが本 issue の論点。

### 既存の drain ヘルパとメトリクス計上の責務

`src/encoder.rs:267-286, 745-764` / `src/decoder.rs:493-512, 514-533` に `drain_audio_encoder_output` / `drain_video_encoder_output` / `drain_audio_decoder_output` / `drain_video_decoder_output` が存在し、「内部 pull バッファ → `TrackPublisher::send_media`」のラッパーを担う。

加えて `drain_encoded_frames` (`src/encoder.rs:714-722`) → `push_encoded_frame_with_metrics` (`src/encoder.rs:724-732`) では `total_output_video_frame_count_metric.inc()` / keyframe 判定が行われており、「inner から `VideoEncoder.encoded` に積む経路」がメトリクス計上と closed issue 0027 の「全フレームに sample_entry」不変条件 (`src/encoder.rs:729-730` のコメント) を担保している。callback 経路でこの責務を inner / Sender / drain いずれに置くかは設計の核心 (評価軸 6)。

### nvcodec の flush 強制同期化

`src/encoder/nvcodec.rs:248-256` で `encode()` 直後に `self.inner.flush()` を強制し、worker の完了を待ち合わせている (理由は同箇所のコメント「上位パイプラインは同期 pull 型で、上位側でペース制御しないと内部キューが溢れて encode() が "encoder buffer is full" で失敗するため」を参照)。この `flush()` 強制によって NVENC の非同期パイプライン並列性は実質的に潰されている。

### 各エンコーダーの現状出力モデル

| 実装 | 内部 API | 内部キュー | 備考 |
|------|----------|------------|------|
| `LibvpxEncoder` (`src/encoder/libvpx.rs`) | 同期 | `input_queue` + `output_queue` | `encode()` 内で `handle_encoded_frames()` を呼ぶ |
| `Openh264Encoder` (`src/encoder/openh264.rs`) | 同期 | `encoded: Option<VideoFrame>` 単発 | 出力 1 フレーム/encode |
| `SvtAv1Encoder` (`src/encoder/svt_av1.rs`) | 同期 | `input_queue` + `output_queue` | 同上 |
| `VideoToolboxEncoder` (`src/encoder/video_toolbox.rs`) | 非同期 | `input_queue` + `output_queue` | `shiguredo_video_toolbox` 内で下位 ABI callback → `std::sync::mpsc::Sender` (tokio ではない同期版) でチャネル化済み、上位は `next_frame()` で pull |
| `NvcodecEncoder` (`src/encoder/nvcodec.rs`) | 非同期 | `encoded_queue: Arc<Mutex<VecDeque>>` + `input_queue` + `output_queue` | hisui コードが下位 ABI callback ハンドラ (`FnEncodeHandler`) を直接実装する唯一の経路。上記の通り `flush()` で同期化 |

### 各デコーダーの現状出力モデル

| 実装 | 内部 API | 内部キュー | 備考 |
|------|----------|------------|------|
| `LibvpxDecoder` (`src/decoder/libvpx.rs`) | 同期 | `input_queue` + `output_queue` | |
| `Openh264Decoder` (`src/decoder/openh264.rs`) | 同期 | `input_queue` + `output_queue` | |
| `Dav1dDecoder` (`src/decoder/dav1d.rs`) | 同期 | `input_queue` + `output_queue` | |
| `VideoToolboxDecoder` (`src/decoder/video_toolbox.rs`) | 非同期 | `decoded: Option<VideoFrame>` 単発 | `shiguredo_video_toolbox` 内で `std::sync::mpsc::Sender` (tokio ではない同期版) チャネル化済み |
| `NvcodecDecoder` (`src/decoder/nvcodec.rs`) | 非同期 | `decoded_queue: Arc<Mutex<VecDeque>>` + `input_queue` + `output_queue` | hisui コードが `FnDecodeHandler` を直接実装 |

hisui コードが下位 ABI callback ハンドラを直接実装しているのは `shiguredo_nvcodec` のみ (`shiguredo_video_toolbox` は内部で callback を `std::sync::mpsc::Sender` 化し、上位には `next_frame()` の pull のみ露出)。

### 共有キュー方式の派生問題

`NvcodecEncoder` / `NvcodecDecoder` の callback 内エラーは `error_slot: Arc<Mutex<Option<Error>>>` に退避され、次回 `handle_*_frames()` 呼び出しで初めて伝搬する (`src/encoder/nvcodec.rs:271-278` / `src/decoder/nvcodec.rs:221-228`)。即時通知ができない。closed issue 0054 の fail-fast 方針 (sample_entry 未確定での出力を `Err` 化) とは別軸の懸念であり、本 issue の再設計で「callback 内 `Err` をどの経路で即時伝搬するか」は評価軸 4 に含める。

## 設計方針

設計検討フェーズなので、本 issue では以下の方向性を比較・評価して結論を出す。実装は別 issue で扱う。

### 共通の評価項目

評価表 (§2) では以下の 9 軸 + 補助 2 列 (合計 11 列) で各案を比較する:

1. **チャネル型**: `tokio::sync::mpsc::Sender` (bounded) / `UnboundedSender` / 内部キュー上限ベース (Sender 不使用) のいずれか
2. **バックプレッシャ戦略**: bounded 容量 N で `tx.send().await` / unbounded + drop / unbounded + 上限到達で `Err` / セルフペーシング (encoder 内部キュー長で待つ) のいずれか
3. **下位ライブラリへの tokio 露出範囲**: 上位 `run()` 層 (既に tokio 使用済み) のみか、`VideoEncoderInner` の各 variant まで広げるか
4. **エラー伝搬経路**: callback 内 `Err` を Sender 経由で送るか (`Result` 型を流す)、別チャネルか、`error_slot` 方式を維持するか (机上充足可能なため計測前に決められる)
5. **RPC (keyframe 要求) との両立 + 順序保証**: `VideoEncoderRpcMessage::RequestKeyframe` (`src/encoder.rs:372-375` / `request_upstream_video_keyframe` `src/encoder.rs:381-424`) は現状「RPC 受信 → `keyframe_request_pending = true` → 次の input フレーム到着時 `inner.request_keyframe()` 呼出」(`src/encoder.rs:694-699`)。入出力を分離する案では「RPC → 次入力 → keyframe 適用」順序保証が崩れないかを評価
6. **メトリクス計上責務の配置**: `total_output_video_frame_count_metric` / keyframe 判定 / sample_entry 不変条件 (`src/encoder.rs:724-732, 729-730`) の責務を inner / Sender / drain いずれに置くか。**各案ごとに 1 候補を本 issue で予め埋める**
7. **既存 `drain_*_output` ヘルパの扱い**: 消える / Sender 経路の入口に置き換わる / 残る
8. **既存テストへの影響**: `src/encoder/libvpx.rs` / `openh264.rs` / `svt_av1.rs` / `video_toolbox.rs` 末尾のテスト群と `src/encoder/test_helpers.rs` への書き換え要否と規模感 (件数は §1 で `wc -l` ベース)
9. **`shiguredo-rust` 規約整合**: 同スキルの「トレイトを作らない (どうしても必要なら許可取得)」「`#[non_exhaustive]` を使わない」等。これは **落とせない必要条件** (NG 案は採用不可)

補助列:

- **flush 撤廃達成度**: 達成 / 達成せず / 部分達成 / 別経路で確保 のいずれか。本 issue の中核動機なので独立列として残す
- **変更行数概算**: 本体実装 (encoder/decoder ファミリ) + テスト + 上位 (`run()`, drain) の合算で「数十行 / 数百行 / 千行超」の 3 段階で記載 (見積もり方法: §1 で `git diff --stat` 風に「変更見込みファイルとおおよその LOC」を列挙して合算)

評価軸の重み:

- (9) は **落とせない必要条件**。NG なら案ごと却下
- (1)〜(7) + flush 撤廃達成度 + 変更行数概算 は **重み付けスコアリング** (重み配分は Decision Owner が §3 で確定)
- (8) は **採用後の作業量見積もり** として参考扱い

### 案 A: 内部 inner と中継キューの間に Sender を挟む

`VideoEncoderInner` の各 variant に `tokio::sync::mpsc::Sender<VideoFrame>` を渡し、現在 `output_queue.push_back` している箇所から直接 push する。上位 `VideoEncoder.encoded: VecDeque` は廃止し、`run()` 側で `Receiver` を `tokio::select!` の入口に追加する。

- 長所: nvcodec の `flush()` 撤廃も bounded channel に切り替えれば自然
- 短所: 各 encoder の `next_encoded_frame()` API を Sender 注入型コンストラクタに書き換える必要があり、既存テスト群への波及が大きい
- 軸 6 候補: メトリクス計上を Sender 経由の型 (例: `(VideoFrame, EncoderStatsHandle)` タプル) で drain 側に集約

### 案 B: `VideoEncoder` を trait オブジェクト化して push / pull を両方許容

`with_callback(...)` のような副 API (= 上位 API callback の登録口) を追加し、上位が選択できるようにする。

- 長所: 段階的に移行可能
- 短所: 2 系統を維持するメンテコスト。**`shiguredo-rust` の「トレイトを作らないこと」規約と衝突** (規約 NG = 採用前に @voluntas / @sile への許可取得が必須。許可取得手続きは Decision Owner が別途実行する)。代替として「`VideoEncoderInner` enum に push 用 variant を増やす」案も検討余地あり。本案は flush 撤廃を必須としない
- 軸 6 候補: 既存の `drain_encoded_frames` 経路を残し、callback 経路では Sender 拡張型に sample_entry / metrics を載せる

### 案 C: 全エンコーダーで内部出力を `tokio::sync::mpsc::Sender` に揃える

同期エンコーダー (`LibvpxEncoder`, `Openh264Encoder`, `SvtAv1Encoder`) も `encode()` 内で得たフレームを Sender に push する形に揃える。上位の `run()` は Receiver を待つだけ。

- 長所: 全エンコーダー対称、`run()` ループのコードが単純化
- 短所: 同期エンコーダーは hisui 側で worker thread を起動する必要はないが、API が `tokio` を要求する点で「エンコーダー実装の純粋関数性」が失われる
- 軸 6 候補: 案 A と同じ (Sender 経由型に metrics handle を載せる)

### 案 D-1: nvcodec の内部キュー上限でセルフペーシング (上位 API 不変)

上位 API には手を入れず、`NvcodecEncoder` 内部で `encoded_queue.len()` を監視し、上限到達時は `encode()` 自身が短時間ブロック (`condvar` / spin 待ち) で待ち合わせる。`flush()` 強制は撤廃する。

- 長所: 影響範囲が `src/encoder/nvcodec.rs` のみに限定される。Premature Optimization 回避原則 (CLAUDE.md) と整合
- 短所: nvcodec 以外の callback 型が将来増えたとき、各実装ごとにセルフペーシングを再設計する必要 (D-2 長所の裏返し)
- 軸 6 候補: 既存の `drain_encoded_frames` 経路を維持

### 案 D-2: 上位 `VideoEncoder` に Sender を内蔵してバックプレッシャを `run()` に集約

`VideoEncoder` 内部に `tokio::sync::mpsc::Sender<VideoFrame>` を持ち、`drain_encoded_frames` (`src/encoder.rs:714-722`) で `tx.send(...).await` (bounded) によりバックプレッシャを発生させる。Receiver は `run()` (`src/encoder.rs:632-658`) の `tokio::select!` の腕に `encoded_rx.recv().await` として追加し、受信したフレームをそのまま `output_tx.send_media()` に渡す (`try_recv()` でビジーループ化させない)。`VideoEncoder::poll_output()` (`src/encoder.rs:734-742`) と `VideoEncoder.encoded: VecDeque` は廃止する。各エンコーダー inner 実装の API (`next_encoded_frame()`) は不変。

- 長所: nvcodec 以外の callback 型が増えても同じ枠で対応可能。各 inner 側のテスト群は無傷
- 短所: `drain_encoded_frames` / `handle_input_sample` (`src/encoder.rs:684-712`) / `handle_input_message` (`src/encoder.rs:676-682`) を **`async fn` 化** する必要がある。これにより `handle_input_sample` 内の `inner.encode()` 呼出経路 (`src/encoder.rs:693-711`) で `tx.send(...).await` を含むと、`run()` の `tokio::select!` (`src/encoder.rs:632-658`) で入力腕に滞留する間、RPC 受信腕 (`recv_video_encoder_rpc_message_or_pending` `src/encoder.rs:648-656`) の並行性が失われ、keyframe 要求の応答性が低下する懸念がある。`VideoEncoder` 内部構造と `run()` ループ構造の変更が必要 (案 A よりは小さいが「最小限」ではない)
- 軸 6 候補: `drain_encoded_frames` 内で `push_encoded_frame_with_metrics` を呼ぶ既存責務を維持

### 案 D-3: D-1 + D-2 を併用 (バックプレッシャ二重化)

`NvcodecEncoder` 内部でセルフペーシング (D-1) しつつ、上位 `VideoEncoder` でも Sender バックプレッシャ (D-2) を入れる。**Sender 流路は D-2 側 1 本のみ** (D-1 は `encode()` 内のセルフペーシングであり、フレームは引き続き `encoded_queue` → `next_encoded_frame()` 経由で D-2 の Sender に渡る)。

- 長所: バックプレッシャを GPU パイプライン上限 (D-1) と上位送信先飽和 (D-2) の双方で吸収でき、いずれか単独で塞ぎきれない領域を補完できる
- 短所: 実装行数が D-1 + D-2 の合算近く、バックプレッシャ責務が二重化する保守コスト
- 軸 6 候補: D-2 と同じ (D-1/D-2 双方が `drain_encoded_frames` 経路を維持するため一致)

### 棄却された案を保存する方針

採用案を決めるとき、棄却された案については以下を本 issue に追記する:

- 棄却理由 (どの評価軸で落ちたか、必要条件不適合か重み付けで負けたか)
- どんな条件が変われば再考するか (例: 「`shiguredo-rust` のトレイト規約が緩和されれば案 B 再考」「同期エンコーダー側にも `shiguredo_nvcodec` のような worker thread 型ライブラリへの切替が発生したら案 C 再考」)

## 完了条件

以下のいずれかが成立し、本 issue 本文に追記されている:

(a) 採用案の決定理由と、棄却された各案の棄却理由 + 再考トリガーが本 issue 内に追記されている

(b) すべての案が採用基準 (§2) を満たさないため「現状維持」を結論とする旨と、その判断根拠が追記されている (この場合「優先度根拠」の改訂も併せて行う)

(a) の場合は加えて以下を満たす:

- 採用案で `NvcodecEncoder` の `flush()` 強制を撤廃可能なバックプレッシャ機構が確定し、本 issue 内に明文化されている (達成しない案を採用する場合は「優先度根拠」末尾の方針に従って「NVENC 並列性回復の別経路」または「優先度降格」を追記)
- 実装に着手するための前提 (`Sender` の型と bounded/unbounded の選択、バックプレッシャ戦略、エラー伝搬経路、RPC keyframe 順序保証、メトリクス計上責務の配置、`shiguredo-rust` 規約上の許可要否) が明文化されている
- 後続実装 issue の **分割粒度案** が本 issue 本文に追記されている (後続 issue ファイルの作成は本 issue の作業に含めない)。素案: (a) `VideoEncoder` interface 変更と `LibvpxEncoder` 追従、(b) `Openh264Encoder` / `SvtAv1Encoder` 追従、(c) `NvcodecEncoder` の flush 撤廃 + nvcodec H.265/AV1 計測、(d) `VideoToolboxEncoder` 追従、(e) `VideoDecoder` 系追従。Audio 系はスコープ外で分割例に含めない
- 分割粒度案の各 issue は以下のチェックリスト項目を含むこと:
  - 影響ファイルの列挙
  - 推定 LOC (3 段階「数十行 / 数百行 / 千行超」)
  - 依存先 issue (記法例: `a → (b ∥ d) → c → e`)
  - 追加 / 改修対象テストの範囲
  - 後方互換影響 (内部 API のみか、外部公開 API か)
- `shiguredo-rust` スキルに照らして、採用案の interface で「nvcodec encoder + recording_video_mixer の end-to-end」相当のテスト雛形を 1 件書き出して、モックを使わず実装できることを確認している (雛形コードを本 issue または別ドキュメントに添付)

判定権限: **Decision Owner (`@sile`)** が単独で確定可能。

## 解決方法

### 1. 現状調査

- 影響範囲を grep 起点で網羅:
  - `src/encoder.rs` / `src/decoder.rs` の `VideoEncoder` / `VideoDecoder` 実装、特に `run()` ループ (`src/encoder.rs:632-658`、`src/decoder.rs:371-388`)
  - `src/encoder/*.rs` / `src/decoder/*.rs` の各実装 (現状一覧は「現状」の表参照)
  - 既存ヘルパ (本 issue「現状」§の引用参照)
  - `MediaPipeline` / `ProcessorHandle` (`src/media_pipeline.rs`) の `spawn_processor` / `subscribe_track` / `publish_track`
  - encoder/decoder の出力 track を `subscribe_track` する箇所を `grep -rn 'subscribe_track' src/` で全列挙し、mixer / writer / scaler / subcommand 階層を網羅
  - RPC 経路: `request_upstream_video_keyframe` (`src/encoder.rs:381-424`) と `VideoEncoderRpcMessage` (`src/encoder.rs:372-375`)
- 各 encoder 実装の末尾テストの `wc -l` 規模感を本 issue に表形式で追記 (`src/encoder/libvpx.rs`, `openh264.rs`, `svt_av1.rs`, `video_toolbox.rs`, `nvcodec.rs`, `src/encoder/test_helpers.rs`)。decoder 側のテストはエンジン選択検証 (`src/decoder.rs:720-821`) に閉じておりリファクタとは独立なので評価対象から外す
- 各案ごとに「変更が見込まれるファイル一覧 + 推定 LOC」を本 issue に追記し、評価表の「変更行数概算」列を埋める
- **§1 完了の判定**: 上記 3 つの追記がすべて本 issue 本文に反映された時点

### 2. 設計案の検証

以下の評価表テンプレートで A / B / C / D-1 / D-2 / D-3 を比較する (セル粒度ガイド: 軸 (1)(3) は型名 / 軸 (2)(4) は戦略名 / 軸 (5)(6)(7) は自由記述 / 軸 (8) は `wc -l` 件数 / 軸 (9) は OK / NG / 達成度は 4 値 / 変更行数概算は 3 段階):

| 評価軸 | 案 A | 案 B | 案 C | 案 D-1 | 案 D-2 | 案 D-3 |
|--------|------|------|------|--------|--------|--------|
| (1) チャネル型 | | | | | | |
| (2) バックプレッシャ戦略 | | | | | | |
| (3) tokio 露出範囲 | | | | | | |
| (4) エラー伝搬経路 | | | | | | |
| (5) RPC 両立 + 順序保証 | | | | | | |
| (6) メトリクス計上責務 | | | | | | |
| (7) drain ヘルパ扱い | | | | | | |
| (8) 既存テスト影響 (規模) | | | | | | |
| (9) shiguredo-rust 規約整合 | | | | | | |
| flush 撤廃達成度 | | | | | | |
| 変更行数概算 | | | | | | |

必要に応じてプロトタイプを作成する。プロトタイプの運用は以下に従う:

- **本 issue ブランチには結論ドキュメント (本 issue 本文への追記) のみ残す**
- プロトタイプは別ブランチで作り、本 issue ブランチにはマージしない。ブランチ名と commit hash は本 issue 上部メタ欄の `Prototype Branch:` / `Prototype Commit:` に追記する
- プロトタイプの段階分け:
  - **(i) 全案共通 / flush 撤廃効果計測 (必須)**: `compose` サブコマンドで固定素材を `nvcodec` H.264 でエンコードし、現状 (`flush()` あり) と試作 (`flush()` 撤廃 + 仮ペーシング) の wall-clock 時間と p99 frame latency を計測。本 issue では H.264 単独で判定する (HEVC / AV1 は後続 (c) で再計測)
  - **(ii) 案 A/C 採用候補時のみ (任意)**: 同期エンコーダー (libvpx / openh264 / svt_av1) を Sender 化したときの追加オーバーヘッドを `compose` で計測 (1080p30 / 60 秒)
  - 工程順:
    1. §1 完了 (`(1-a)` テスト規模感の表、`(1-b)` 各案変更ファイル一覧 + LOC、`(1-c)` `subscribe_track` 全列挙の 3 つすべてが本文に反映)
    2. **Decision Owner が採用基準の暫定値 (15% / 5ms / -10%) を本 issue に確定記録** (計測前に確定して後付け正当化を防ぐ)
    3. 机上評価表 (一次案棄却)
    4. (i) 実施
    5. 採用基準判定の分岐:
       - 主計測 Yes (基準達成) → A/C が候補に残れば (ii) 実施 → §3
       - 主計測 No かつ部分改善あり → Decision Owner が判断 (採用 / 棄却 / (ii) 実施を指示) → §3 または (ii) 経由 §3
       - 主計測 No かつ改善なし → 全案棄却 = 完了条件 (b) で §3
       - サブ計測で 1 件でも -10% 以上の劣化があれば「保留」、Decision Owner が再評価
- 計測条件 (再現性のため明示):
  - 主計測: 1080p30 / 60 秒 / GOP 30。サブ計測: 4K30 / 60 秒、1080p60 / 60 秒、GOP 120
  - GPU 型番・NVIDIA driver / CUDA バージョン・OS・hisui ビルド feature (`--features nvcodec`) を本 issue に記録
  - 計測時は並走する NVENC セッションが無いことを確認
  - 計測素材は事前に固定の素材を 1 つ選び、本 issue に出所と保存先を追記する。保存先運用: 素材が **100 MB 未満なら `Prototype Branch:` 直下に commit 可**、**100 MB 以上なら git に含めず社内 S3 等の URL を本 issue に記録** (git 履歴肥大を防ぐため)
  - 各案 5 run + ウォームアップ 1 run、`平均 ± 標準偏差` を記録、信頼区間 95%
- 採用基準 (定量しきい値、**暫定値、計測前 = §1 完了直後・机上評価着手前に Decision Owner が本 issue に確定記録**):
  - 主計測で wall-clock 短縮 ≥ 15% かつ p99 frame latency 改善 ≥ 5ms を「flush 撤廃の効果あり」の暫定基準とする (15% / 5ms は本 issue 起票時点の経験則暫定値、Decision Owner 確定時に置き換え可)
  - サブ計測閾値 -10% の暫定根拠: 主計測 +15% 改善幅の 2/3 以内 (= 10%) を悪化吸収幅の上限とする経験則
  - 主計測未達時の判断は上記「工程順」の分岐に従う (閾値未達でも全案棄却 = 完了条件 (b) の選択肢が常に残る)
- 計測結果は以下のテンプレートで本 issue に追記する:

| 案 | 解像度 | GOP | 現状 wall-clock | 試作 wall-clock | 短縮率 % | 現状 p99 latency | 試作 p99 latency | Δp99 (ms) | メモ |
|----|--------|-----|------------------|-------------------|----------|-------------------|---------------------|-----------|------|

### 3. 決定

- 採用案: **案 C (全エンコーダー Sender 出力に統一)**
- 決定日: 2026-06-26
- 決定者: `@sile`

#### 採用理由 (定性判断)

ユーザー判断軸「互換性・修正量を気にしない、VideoEncoder 自体を非同期寄りにする、今後の HW エンコーダーが非同期化していくトレンドを前提」のもとで、コードベース全体の単純性 + 将来の HW 非同期化への素直さで C が他案を上回る:

1. **`VideoEncoderInner` enum の dispatch が `encode()` だけに集約**: `next_encoded_frame()` 系の dispatch (`src/encoder.rs:862-872`) が消える
2. **inner 構造が 1 系統に揃う**: 同期 inner も非同期 inner も「Sender push 型」に統一。「inner ごとに違うパターン」を読み解く認知負荷が消える
3. **上位 aggregation コードが消える**: `drain_encoded_frames` + `push_encoded_frame_with_metrics` 相当の集約処理が `run()` の Receiver 1 ループに集約される
4. **テストパターン統一**: 全 inner テストが「`tokio::sync::mpsc::channel(N)` を作って Sender 渡して Receiver で受ける」1 パターンに集約
5. **callback friendly 定義 (ホップ数上限 1) を真に満たす**: 案 A は途中段階で、C はその徹底版

#### 採用基準 (定量しきい値) の判定スキップ

§2 の計測 ((i)/(ii)) は本 issue ではスキップする。理由:

- 本判断は「将来の HW 非同期化への素直さ」「コードベース全体の単純性」という **設計品質に基づく定性判断** であり、定量しきい値で決めるべき性質ではない
- flush 撤廃自体は採用案 C で技術的に達成可能 (bounded `tokio::sync::mpsc::channel` でバックプレッシャを発生させ、`NvcodecEncoder` の callback ハンドラ内で `tx.blocking_send` / `tx.try_send` 経路で出力 → `flush()` 強制を撤廃できる)
- 実機性能計測は後続実装 issue (c) で `NvcodecEncoder` の flush 撤廃時に実施し、想定通り並列性が回復しているかを確認する

#### 棄却された案

| 案 | 棄却理由 | 再考トリガー |
|----|----------|---------------|
| A | C の中間段階。同期 inner で `output_queue + next_encoded_frame()` の旧構造が残り「2 系統」が解消されないため、コードベース全体の単純性で C に劣る | 同期 inner (`LibvpxEncoder` 等) で `Sender::send().await` 経路が想定以上に重い / 借用境界が解けないことが (a) 実装段階で判明した場合、A に後退する |
| B | `shiguredo-rust` 規約「トレイトを作らない」の必要条件 (評価軸 9) NG。許可取得しても push/pull 2 系統の維持コストが残る | 規約が緩和され、かつ push/pull 両系統を併存させたい外部互換要求が出た場合 |
| D-1 | nvcodec のみで完結する局所修正で、今後の HW 非同期化トレンドに対する将来コストが残る | nvcodec の修正だけで急いで済ませたい運用要求が単発で出た場合 |
| D-2 | inner レベルの構造が古いまま (`next_encoded_frame()` を残す)、callback friendly 定義 (ホップ 1) を満たさない。`handle_input_sample` の async 化で RPC 並行性ロスの懸念 | C 採用後に (a) の実装が困難と判明し、上位だけ async 化する妥協案として落ち場が必要な場合 |
| D-3 | D-1 + D-2 併用でバックプレッシャ責務が二重化、保守コスト増 | D-2 単独 (or C 単独) で塞ぎきれない GPU パイプライン上限要因が後続計測で判明した場合 |

#### 実装前提 (採用案 C)

- **Sender 型**: `tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` (bounded、初期容量 N=8 を推奨、計測 / 実装段階で調整可)
- **バックプレッシャ戦略**: bounded 容量 N で `tx.send(...).await` (溢れたら待つ)。非同期 inner の callback 内では `tx.blocking_send(...)` (callback が tokio runtime 外なら) または `tx.try_send(...)` + 容量超過時の上位ペーシング待ち。同期 inner は `encode()` を `async fn` 化して `tx.send(...).await` を直接呼ぶ
- **エラー伝搬経路**: `Result<VideoFrame, crate::Error>` を Sender に流す形に統一。`error_slot: Arc<Mutex<Option<Error>>>` (`src/encoder/nvcodec.rs` / `src/decoder/nvcodec.rs`) は廃止し、callback 内 `Err` は即時に `tx.send(Err(_))` で通知する
- **RPC (keyframe 要求) との両立 + 順序保証**: 現状の「RPC 受信 → `keyframe_request_pending = true` → 次の input フレーム到着時に inner に伝える」(`src/encoder.rs:694-699`) 経路を維持。inner が async 化しても受け方は不変
- **メトリクス計上責務の配置**: `run()` 内の Receiver 受信ループに集約。現状 `push_encoded_frame_with_metrics` (`src/encoder.rs:724-732`) で行っている `total_output_video_frame_count_metric.inc()` / keyframe 判定 / sample_entry 不変条件 (closed/0027) は Receiver 受信側に移植
- **既存 `drain_*_output` ヘルパの扱い**: `drain_video_encoder_output` (`src/encoder.rs:745-764`) と `drain_encoded_frames` は廃止。`run()` の `tokio::select!` の腕に `encoded_rx.recv().await` を追加し、受信フレームを直接 `output_tx.send_media()` に渡す
- **`shiguredo-rust` 規約**: トレイト追加なし (`VideoEncoderInner` enum は維持)、`#[non_exhaustive]` 不使用、規約上の許可取得は不要

#### end-to-end テスト雛形 (モック禁止規約整合の確認)

採用案 C のもとで「nvcodec encoder + recording_video_mixer の end-to-end 相当」のテストは以下の構造で書ける (モック不使用 + 実エンコーダ + tokio channel):

```rust
#[tokio::test]
async fn test_nvcodec_encoder_to_receiver_e2e() {
    let stats = crate::stats::Stats::new();
    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<crate::Result<VideoFrame>>(8);

    // 1. C 形式: NvcodecEncoder を Sender を受け取って生成する
    let mut encoder = NvcodecEncoder::new_h264(&options, tx).expect("encoder");

    // 2. 実フレームを投入 (生成 I420 でも実カメラフレームでも可)
    let raw_frame = generate_test_raw_video_frame(1920, 1080);
    encoder.encode(raw_frame).await.expect("encode");
    encoder.finish().await.expect("finish");

    // 3. Receiver でエンコード結果を受信
    let frame = rx.recv().await.expect("receive").expect("ok frame");

    // 4. closed/0027 / closed/0054 由来の不変条件を確認
    assert_eq!(frame.format, VideoFormat::H264);
    assert!(frame.sample_entry.is_some());
}
```

`recording_video_mixer` 連携テストも、`rx` 側を mixer の入力 track に bridge する形で同じ枠で書ける。モック / スタブは不要 (`shiguredo-rust` 規約 OK)。

#### 後続実装 issue の分割

実装着手段階で encoder 側 / decoder 側の 2 分割に絞った。decoder が encoder より単純 (RPC keyframe 経路なし / `flush()` 強制同期化なし / sample_entry 不変条件なし / メトリクス計上が `total_output_video_frame_count_metric` のみ) なので、より単純な decoder で C 形式 interface パターンを先行確立し、encoder で複雑な要件 (RPC / flush 撤廃 / メトリクス重 / `error_slot` 廃止) を解く流れにする。

依存順序:

- decoder 系列: **`0066 → {0068 / 0071 / 0072} → 0073`** (2026-07-06 完了)
- encoder 系列: **`0066 → 0067 → {open/0079 / encoder wrap 削除 rename / encoder 未使用 API 削除}`** および `0067 → open/0080` (perf は refactor 系列と並列)

| ID | 範囲 | 推定 LOC | 依存先 | 後方互換影響 |
|----|------|----------|---------|---------------|
| closed/0066 (`feature/refactor-add-async-video-decoder`) | `AsyncVideoDecoder` 新規追加 + 既存 `VideoDecoder` の wrap 化 + 全 inner (Libvpx/Openh264/Dav1d/VideoToolbox/Nvcodec) の Sender 化 (`OutputSink` 経由)、既存外部 API 維持 | 千行前後 | なし | 内部 API のみ |
| closed/0068 (`feature/refactor-migrate-video-decoder-users-to-async`) | subcommand_inspect / sora の processor 経路を `AsyncVideoDecoder` に移行 + `AsyncVideoDecoder::run` 追加 | +147/-9 | 0066 | 内部 API のみ |
| closed/0071 (`feature/refactor-mp4-reader-async-video-decoder`) | mp4 reader の video decoder 経路を decoder task (spawn pattern) 化 + `set_video_decoder` / `discard_video_decoder_output` 削除 | +324/-107 | 0066 | 内部 API のみ |
| closed/0072 (`feature/refactor-inbound-endpoint-async-video-decoder`) | RTMP / RTSP / SRT inbound endpoint の video decoder 経路を spawn pattern 化 | +483/-150 | 0066 | 内部 API のみ |
| closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`) | 同期 wrap `VideoDecoder` 削除 + `AsyncVideoDecoder` を `VideoDecoder` にリネーム | +109/-294 | 0068 / 0071 / 0072 | 内部 API のみ |
| closed/0067 (`feature/refactor-add-async-video-encoder`) | `AsyncVideoEncoder` 新規追加 + 既存 `VideoEncoder` の wrap 化 + 全 inner (Libvpx/Openh264/SvtAv1/VideoToolbox/Nvcodec) の Sender 化 (`OutputSink` 経由)、`error_slot` 廃止、メトリクス計上の `OutputSink` ペアリング化、既存外部 API 維持 | +1025/-401 | 0066 | 内部 API のみ |
| open/0079 (`feature/refactor-migrate-video-encoder-users-to-async`) | encoder 使用側 4 hit を `AsyncVideoEncoder` に移行 + `AsyncVideoEncoder::run` 追加 | +75/-13 | 0067 | 内部 API のみ |
| (未起票) encoder wrap 削除 + rename refactor issue | 同期 wrap `VideoEncoder` 削除 + `AsyncVideoEncoder` を `VideoEncoder` にリネーム + `_sync` / `_async` サフィックス整理 | 未推定 | open/0079 | 内部 API のみ |
| (未起票) encoder 未使用 API 削除 refactor issue | 使用側移行完了後の dead code 削除 + `EncoderOutputReceiver` 可視性整理 | 未推定 | encoder wrap 削除 + rename issue | 内部 API のみ |
| open/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`) | NVENC 非同期パイプライン並列性回復 (flush() 撤廃 + bp 機構)、 wall-clock 短縮 15% / p99 改善 5ms 等の実機計測を完了条件に据える | 未推定 | 0067 | 内部 API のみ (perf カテゴリ) |

備考:

- Audio 系 (AudioEncoder / AudioDecoder) は本 issue スコープ外なので分割に含めない (再設計動機が成立しないため現状維持)
- 0066 を先に置く理由: 単純な題材で C 形式の interface が成立可能かを実装可否検証する。困難なら採用案 C を再検討 (案 A への後退) する弾力性ポイント
- 0067 は 0066 完了後に着手し、0066 で確定した Sender 型 (unbounded) と派生方針 (δ) を踏襲する
- 各 inner ごとに分割しない理由: `VideoEncoderInner` / `VideoDecoderInner` enum dispatch は全 variant 揃って初めて C 形式になるため、途中段階で adapter を挟むのは捨てコードになる (Premature Optimization)。1 PR 内で全 variant をまとめて書き換える方がコードベース全体の単純性に貢献する
- **方針 (δ) について**: 0066 polish 後の Decision Owner 判断で「2 系統共存を意図的に許容し 0068 / 0071 / 0072 / 0073 で最終解消する派生」を採用。 0067 polish (2026-07-07) で同派生方針を encoder 系列にも展開 (0067 + encoder 使用側移行 / wrap 削除 rename / 未使用 API 削除の 4 段階、 flush 撤廃 perf は独立)。 0066 + 0068 で採用案 C の長所の大半を分担達成し、残る (v)「ホップ数上限 1」は 0071 / 0072 / 0073 のクリーンアップ (drain 経路・wrap 型の除去) で最終達成 (詳細は closed/0066 §設計方針)。 encoder 系列も同じ達成パターンを踏襲する

## CHANGES.md について

本 issue は設計検討のみで実装を伴わないため、`CHANGES.md` への記載は不要。後続実装 issue 側で個別に判断する。

## 関連

- closed/0027 (`feature/refactor-video-sample-entry-all-frames`): 映像エンコーダは全出力フレームに sample_entry を載せる不変条件 (`src/encoder.rs:729-730` のコメントが参照点)。本 issue で interface を変えても維持する
- closed/0030 (`feature/refactor-encoded-frame-sample-entry-invariant`): エンコード済みフレームの sample_entry 必須化
- closed/0051 (`feature/refactor-remove-writer-sample-entry-fallback`): writer 入口の不変条件 (圧縮フレームの sample_entry は必ず `Some`)。callback 経路でも違反しない設計が必要
- closed/0054 (`feature/refactor-encoder-defer-output-until-sample-entry-ready`): エンコーダーで sample_entry 未確定時の出力を `Err` 化する fail-fast 整備。callback 経路でも fail-fast を維持できる Err 伝搬設計とする
- open/0046 (`feature/refactor-clarify-processor-validation-boundary`): processor 構造体の validation 責務分担。本 issue で API を変更する場合、`VideoEncoderOptions` / `VideoDecoderOptions` の validation を `new()` 内に閉じ込めるか外出しするかを併せて結論を出す。0046 側に「本 issue の API 決定に追従する余地あり」と将来追記すべきか、本 issue 採用案決定時に Decision Owner が判断する
