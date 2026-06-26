# VideoDecoder 系とその利用箇所を Sender 出力に統一する

- Priority: Medium
- Created: 2026-06-26
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-video-decoder-sender-interface
- Polished: 2026-06-26
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0057 で確定した採用案 C (全エンコーダー / デコーダーを `tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` 出力に統一) を **VideoDecoder 系と全利用箇所に適用する** 先行 PR。

最終形: 全使用側が「`let (decoder, decoded_rx) = VideoDecoder::new(options, stats);` → `tokio::spawn(decoder.run(input_rx, output_tx, decoded_rx))`」の 1 パターンに統一。`drain_video_decoder_output` / `discard_video_decoder_output` ヘルパと `set_video_decoder` 後注入パターンは完全に廃止し、中途半端な「2 系統共存」状態を残さない。

詳細な動機・採用理由は closed/0057 §3 参照。

### スコープ拡大の前提

本 issue は decoder 内部の API 変更だけでは完結しない。外部利用箇所 (mp4/reader, rtsp, srt, rtmp, subcommand 系, obsws/source/file_mp4) も本 issue 内で完全に Sender 化する。これを段階分けせず 1 PR で揃える理由は中途半端な 2 系統共存の回避 (closed/0057 §3 採用案 C 整合)。

兄弟 issue 0067 (encoder 側) も同様のスコープ拡大が必要 (encoder 外部利用箇所も Sender 化)。本 issue 完了時の 0067 polish で対称拡大する。

## 優先度根拠

Medium。

- closed issue 0057 で採用案が確定済み。実装着手段階に入っているため Medium 維持
- 依存先: 0067 (encoder) は本 issue 完了後に同パターンで対称展開
- 採用案 C 再検討 (案 A への後退) トリガー: 各 inner の `&mut self` 借用境界が `async fn` 化で解けない / PR 行数が想定 (約 2000-2500 行) を 1.5 倍以上に超える / mp4 reader の `recreate_decoders` 再設計が困難で代替案が無い、のいずれかが判明した時点
- 判断者: Decision Owner (`@sile`)

## 現状

`src/decoder.rs` および `src/decoder/*.rs` の各 inner の構造は closed issue 0057 「現状」§の表を参照。本 issue 固有の論点は以下:

### 上位 VideoDecoder の構造

- `VideoDecoder.decoded: VecDeque<VideoFrame>` (`src/decoder.rs:335`) + `poll_output()` (`src/decoder.rs:422-430`) で同期 pull
- `VideoDecoderInner::Initial { options }` (`src/decoder.rs:537-553`) で最初の入力フレーム到着時の遷移は `decode()` 内 (`src/decoder.rs:677-681`) で実行
- `VideoDecoder::run()` (`src/decoder.rs:360-391`) は素朴 `loop { input_rx.recv().await; ... drain_video_decoder_output(...) }` 構造で **`tokio::select!` 不使用**

### 外部利用箇所 (本 issue で全部書き換え対象)

| 利用パターン | 対象ファイル | 用途 |
|--------------|--------------|------|
| `drain_video_decoder_output` pull 型 | `src/mp4/reader.rs:1236, 1286` (2 行) | mp4 read ループから decoder を pull |
| 同上 | `src/rtsp/subscriber.rs:662` | RTSP 受信から pull |
| 同上 | `src/srt/inbound_endpoint.rs:445` | SRT 受信から pull |
| 同上 | `src/rtmp/inbound_endpoint.rs:422` | RTMP 受信から pull |
| `discard_video_decoder_output` pull warm-up drain | `src/mp4/reader.rs:1388` | mp4 reader 内 decoder warm-up 中の出力破棄 |
| `VideoDecoder::new` 外部生成 | `src/subcommand_inspect.rs:215` | inspect 単発 decode |
| 同上 | `src/rtsp/subscriber.rs:64` | RTSP 用 |
| 同上 | `src/srt/inbound_endpoint.rs:169` | SRT 用 |
| 同上 | `src/rtmp/inbound_endpoint.rs:112` | RTMP 用 |
| 同上 | `src/sora/recording_subcommand_vmaf.rs:362, 480` (2 行) | vmaf 用 |
| 同上 | `src/sora/recording_subcommand_compose.rs:463` | compose 用 |
| 同上 | `src/obsws/source/file_mp4.rs:54` | obsws file mp4 source 用 |
| `VideoDecoder::new` 内部 self-recreate | `src/mp4/reader.rs:1369` (`recreate_decoders` `:1350` 内) | mp4 reader が `loop_playback` 再生のたびに decoder を作り直す |
| `set_video_decoder` 後注入 | `src/obsws/source/file_mp4.rs:61` (注入先 `Mp4FileReader::set_video_decoder` `src/mp4/reader.rs:318`) | mp4 reader への decoder 注入 |

合計: `drain_*` 利用 4 ファイル / 5 call site、`discard_video_decoder_output` 1 箇所、`VideoDecoder::new` 利用 8 ファイル / 9 call site、`set_video_decoder` 1 箇所。

### mp4 reader の制御フロー (Sender 化で配慮必須)

`Mp4FileReader` は `subscribe_track` 経路を **使わず**、内部で `decoder.handle_input_sample(...)` に直接 push して `drain_video_decoder_output(decoder, &mut sender.sender)` で同期 drain する構造 (`src/mp4/reader.rs:1230-1290` 周辺)。output 側は mp4 reader 自身が `publish_track` で取得した `video_sender: Option<TrackSender>` (`src/mp4/reader.rs:1446` で定義) を保持し、decoder にも `&mut sender.sender` (内部の `TrackPublisher`) を渡している。

`TrackSender` ラッパーは `noacked_sent: u64` (`:1446` 周辺フィールド) を持ち、`prepare_send` (`:1462-1470`) で `MAX_NOACKED_COUNT = 100` (`:24`) を超えたら SYN ACK 待ちを行う **独自バックプレッシャ機構** が組み込まれている。Sender 化後はこの SYN/ACK 背圧を decoder task 側で維持する必要がある (詳細は §設計方針)。

`recreate_decoders` (`src/mp4/reader.rs:1350`) は `loop_playback` 再生のたびに decoder を作り直す。呼出元は **`:475, :494, :519, :645, :1346` の 5 箇所** (うち `:1346` は `reset_for_restart` 内、`:645` は `apply_seek` 内)。`flush_decoders` (`src/mp4/reader.rs:1274`) は `:339, :350` から呼ばれ、decoder の残出力を drain する。Sender 化後はこれらすべて `async fn` 化が必要。

`TrackPublisher` は **`Clone` 不可**。`Drop` 実装で subscriber を pipeline に返却するため、decoder task に move-only で渡す設計に確定する。

### 各 inner の固有挙動 (Sender 化で配慮必須)

- **Openh264Decoder** (`src/decoder/openh264.rs:32-37, 60-71`): `decode()` 内で keyframe 入力時に先に `self.finish()` を呼び、バッファ内残フレーム (旧 SPS/PPS 由来) を flush してから新 keyframe を decode する。1 回の `decode()` で **0〜2 フレーム送信** が発生する
- **VideoToolboxDecoder** (`reinitialize_if_need` `src/decoder/video_toolbox.rs:119-166`、ヘルパ `reinitialize_raw_codec_if_need` `:169-188`): SPS/PPS 変化や解像度変化時にデコーダー再初期化。`self.decoded.is_some()` (= 直前 frame が pull 未消費) なら `Err` を返す不変条件あり。`decoded: Option<VideoFrame>` (`src/decoder/video_toolbox.rs:12`) は 1 フレーム/decode 単発
- **NvcodecDecoder** (`src/decoder/nvcodec.rs:13-14` 型エイリアス / `:24-25` フィールド / `:29-56` `build_handler` / `:220-295` `handle_decoded_frames`): hisui コードが `FnDecodeHandler` を直接実装し、`decoded_queue: Arc<Mutex<VecDeque>>` + `input_queue: VecDeque<VideoFrame>` + `error_slot: Arc<Mutex<Option<Error>>>` で callback 結果を退避してから pull。`handle_decoded_frames` 内で `input_queue.pop_front()` して対応する `VideoFrame` を取得し、`shiguredo_libyuv::nv12_to_i420` で NV12→I420 変換して `VideoFrame::new_i420` を組み立てる

### メトリクス計上の現状

- `total_input_video_frame_count_metric.inc()` (`src/decoder.rs:405`、フィールド宣言 `:333`、let 束縛 `:345`): 入力フレーム数
- `total_output_video_frame_count_metric.inc()` (`src/decoder.rs:415`、フィールド宣言 `:334`、let 束縛 `:346`): 出力フレーム数

decoder には sample_entry 不変条件 (closed/0027) や keyframe 判定の計上義務はない (encoder の責務)。decoder 出力フレームは `VideoFrame::new_i420` で `sample_entry: None` 設定なので、上位で不変条件チェック不要。

## 設計方針

closed issue 0057 §3 採用案 C「実装前提」を踏襲する。本 issue 固有の決定は以下:

### Sender の流路と所有関係 (確定版)

**2 種類の channel を分離する**:

- **外部 → VideoDecoder の入力**: 既存の `input_rx: MessageReceiver` を `run()` に引数で渡す
- **VideoDecoder 内部 inner → run() の橋渡し用 channel**: `VideoDecoder::new(options, stats)` の戻り値を `(decoder, decoded_rx)` のタプルにし、`decoded_rx` は外部が保持して `run()` 起動時に引数で戻す。`decoder` 構造体側は対の `tx` のみ保持。`Initial → 実 decoder` 遷移時に `self.tx.clone()` (素の Sender clone、Arc 不要) を inner に渡す
- **VideoDecoder → 外部の出力**: `run()` に `output_tx: TrackPublisher` を引数で渡す。`output_tx` は decoder task が move-only で独占所有

`run()` シグネチャ: `pub async fn run(self, input_rx: MessageReceiver, output_tx: TrackPublisher, decoded_rx: mpsc::Receiver<crate::Result<VideoFrame>>) -> crate::Result<()>`

### `run()` ループの構造: 入力 / 出力を 2 task に分離

同一 task 内 `tokio::select!` の `await` 中に他腕が停止する問題 (入力腕の `inner.decode().await` が内部 channel 満杯で `tx.send().await` 待ちになると Receiver 腕が drain できず deadlock) を避けるため、`run()` 内で **2 つの sub-task を `tokio::spawn` する** 構造に確定する。

`VideoDecoder` を擬似コードレベルで `let Self { ... } = self;` で分解し、各フィールドを 2 task に分配する (新型 `InnerRunner`/`OutputRunner` を別途定義する必要はない):

```rust
pub async fn run(
    self,
    input_rx: MessageReceiver,
    output_tx: TrackPublisher,
    decoded_rx: mpsc::Receiver<crate::Result<VideoFrame>>,
) -> crate::Result<()> {
    let Self {
        engine_metric,
        codec_metric,
        total_input_video_frame_count_metric,
        total_output_video_frame_count_metric,
        tx,
        inner,
        ..
    } = self;

    // 出力 task → 入力 task への shutdown 伝達 (下流 close 検知時)
    let cancel = tokio_util::sync::CancellationToken::new();

    let input_handle = tokio::spawn(run_input(
        inner,
        tx,
        input_rx,
        total_input_video_frame_count_metric,
        engine_metric,
        codec_metric,
        cancel.clone(),
    ));
    let output_handle = tokio::spawn(run_output(
        output_tx,
        decoded_rx,
        total_output_video_frame_count_metric,
        cancel,
    ));

    // 両 task の結果を回収する。JoinError は crate::Error に変換する
    let (input_result, output_result) = tokio::join!(input_handle, output_handle);
    input_result
        .map_err(|e| crate::Error::new(format!("decoder input task panicked: {e}")))??;
    output_result
        .map_err(|e| crate::Error::new(format!("decoder output task panicked: {e}")))??;
    Ok(())
}
```

依存追加: `tokio_util` workspace 依存を追加する (本 issue 内で `Cargo.toml` 編集対象に含める)。`tokio::sync::oneshot` での代替も可能だが、`CancellationToken` の方が複数所有者で共有しやすく実装が素直なため採用。

### EOS / エラー終了の経路

入力 task / 出力 task の終了経路を以下 3 ケースで整理する:

**ケース A: 通常 EOS**
1. 入力 task が `Message::Eos` を受信
2. 入力 task が `inner.finish().await` で内部 channel に残フレームを全送出 (この finish 由来フレームも出力 task 側で `forward_decoded_frame` を経由するためメトリクスは漏れなく計上される)
3. 入力 task が `tx` を drop (= 内部 channel close シグナル)
4. 出力 task の `decoded_rx.recv()` が None を返す → 出力 task 側で `output_tx.send_eos()` を呼んで終了
5. `handle_input_message` 内では `Message::Eos` を **no-op** に変更し、上記シーケンスとの二重 finish を避ける

**ケース B: 下流 close (出力 task 側で検知)**
1. 出力 task の `output_tx.send_media()` が `false` を返す (pipeline closed)
2. 出力 task は `cancel.cancel()` で入力 task に shutdown 伝達 → `return Err(crate::Error::new("pipeline closed before decoder finished"))` で fail-fast
3. 入力 task は `cancel.cancelled()` を `tokio::select!` で監視しており、cancel 検知で抜ける
4. 出力 task はこのケースでは `send_eos` を呼ばない (下流が既に閉じているため)

**ケース C: inner エラー (入力 task 側で発生)**
1. 入力 task の `inner.decode().await` が `Err` を返す
2. 入力 task は `tx.send(Err(_)).await` で出力 task にエラー伝達 → 自身は `Err` を返して終了
3. 出力 task は `Err` 受信時に `output_tx.send_eos()` を呼んでから `return Err(_)` で fail-fast (closed/0054 整合)

### 各 inner の Sender 化形態 (全 variant 統一)

全 inner が **コンストラクタで `tx: tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` を受け取って内包** する形に統一する (Sender は内部 `Arc` なので素 clone で OK、`Arc<Sender>` ラップ不要)。`VideoDecoderInner::decode` の統一シグネチャは `async fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()>`。

- **同期 inner (Libvpx / Openh264 / Dav1d)**: 同期 decode → 各出力 frame を `self.tx.send(Ok(frame)).await?` で push
- **Openh264Decoder** の keyframe シーケンス: `if frame.keyframe { self.finish().await?; }` (旧 SPS/PPS 由来 0〜1 フレーム送信) → 新 frame decode + `tx.send(Ok(...)).await?` (0〜1 フレーム送信)
- **VideoToolboxDecoder**: `decoded: Option<VideoFrame>` 廃止。`reinitialize_if_need` の `decoded.is_some()` ガード条件は廃止し、代わりに `decode().await` 内で `tx.send().await` 完了を経た後に次の `decode().await` が始まるシーケンスを担保 (run_input task の sequential await で自然に成立)。bounded 容量 N の最小値検討 (N=1 で実害が無ければ採用、ただし `forward task` が drain 速ければ N=1 が最も同期的に近い)
- **NvcodecDecoder**: コンストラクタ `new_h264(.., tx)` で `tx` を `move` で `FnDecodeHandler` クロージャに内包。`build_handler` (現 `src/decoder/nvcodec.rs:29-56`) のシグネチャを `fn build_handler(tx: Sender<crate::Result<VideoFrame>>, input_queue: Arc<Mutex<VecDeque<VideoFrame>>>) -> FnDecodeHandler<...>` に変更。callback 内で `input_queue.pop_front()` → NV12→I420 変換 (`shiguredo_libyuv::nv12_to_i420`、`block_in_place` 範囲に含む) → `tx.blocking_send(Ok(frame))` の順で実行 (`tokio::task::block_in_place` 経由で tokio worker block を safe にする)。`input_queue` は `Arc<Mutex<VecDeque<VideoFrame>>>` 化して callback と `decode()` push 側で共有。callback が input より先に走るケース (`pop_front()` が None) は `shiguredo_nvcodec` 仕様上ありえないと暫定し、`expect("decoded frame produced without input frame")` で fail-fast

注: NvcodecDecoder の callback dispatch スレッド規約 (`cuvidParseVideoData` の同期 dispatch 仕様、フレーム順序保証) は実装着手前に `shiguredo_nvcodec` 側ソースを確認して本 issue に追記する (Polish フェーズで Decision Owner が実調査)。本 issue 起票時点では `block_in_place + blocking_send` 案 + 投入順保証前提で進める。

### バックプレッシャ戦略

- 内部 channel: `tokio::sync::mpsc::channel(N)` (bounded)。N=8 を暫定値とし、N=1 と N=8 の比較は別 issue で計測 (本 issue は N=8 暫定固定で完成扱い)
- N の最小制約: `inner.decode()` 1 回が一度に送信しうる最大フレーム数 + 余裕 (Openh264 で最大 2 → N >= 4)
- broadcast の lag drop: `MediaPipeline::subscribe_track` (`tokio::sync::broadcast` ベース) の lag drop で上位 receiver 遅延を吸収 (closed/0057 採用案 C 整合)

### 外部利用箇所の Sender 化パターン

全使用側が以下の共通パターンに揃う:

```rust
let (decoder, decoded_rx) = VideoDecoder::new(options, stats);
let join_handle = tokio::spawn(decoder.run(input_rx, output_tx, decoded_rx));
// 使用側は input 経路 (subscribe_track or 自前 mpsc) への送信と、output 経路からの取り出しのみ
// 終了時: input_rx を drop → decoder.run() ループ完走 → join_handle.await
```

#### mp4 reader (`Mp4FileReader`) の再設計

mp4 reader は subscribe_track 経路ではなく自前で decoder に push する構造のため、Sender pattern では以下に再設計:

1. mp4 reader 内部で `let (input_tx, input_rx) = mpsc::channel(M);` を生成し、decoder task に渡す
2. `let (decoder, decoded_rx) = VideoDecoder::new(options, stats);`
3. **`output_tx`/`TrackSender` の所有を decoder task に移す**: 現状の `video_sender: Option<TrackSender>` (`:1446`) は decoder task が独占所有する形に変える。具体的には、`TrackSender` ラッパーごと decoder task に move して、`TrackSender::prepare_send` (`:1462`) ベースの SYN/ACK 背圧 (`MAX_NOACKED_COUNT = 100`) を decoder task 内 (出力 task) で維持する。decoder の `run()` シグネチャを「`TrackSender` 受け取り版」と「`TrackPublisher` 受け取り版」に分けるか、`TrackSender` を生成済みのまま渡せるよう `VideoDecoder::new` の引数を拡張するかは実装段階で決定。設計の前提は **「SYN/ACK 背圧を回帰させない」**
4. `let join_handle = tokio::spawn(decoder.run(input_rx, output_tx_or_sender, decoded_rx));`
5. mp4 read ループで `input_tx.send(Message::Media(frame)).await?` でフレーム供給
6. `recreate_decoders` 時 (Restart 経路): 前 spawn の `input_tx` を drop → `join_handle.await?` で待つ → 上記 1〜4 で再 spawn。Restart 時は `inner.finish()` を呼ばずに drop することで残フレームを破棄する旧構造と等価な挙動を維持する (loop_playback 再生開始時に decode 残バッファは不要)
7. `flush_decoders` 時 (停止経路): `input_tx.send(Message::Eos).await?` → drop → `join_handle.await?` で finish 経由の残フレーム送出を待つ
8. `discard_video_decoder_output` (`src/mp4/reader.rs:1388`) は warm-up 中の出力破棄用ヘルパなので、Sender pattern では「decoder task を立ち上げ、出力 task の `decoded_rx` を warm-up 期間中は捨てる」処理に置き換えるか、warm-up を decoder task 内部の責務に変える。audio 側 `discard_decoder_output` (`:1382`) も同様に書き換える

`recreate_decoders` (`src/mp4/reader.rs:1350`)、`flush_decoders` (`:1274`)、`reset_for_restart` (`:1340`)、`apply_seek` (`:638`) はすべて `async fn` 化する必要があり、呼出元 (`:339, :350, :378, :383, :465, :475, :479, :484, :494, :498, :503, :519, :523, :528, :645` の 15 箇所、 すべて既に `async` コンテキスト内なので `.await` 付与で追従) も書き換える。

`Mp4FileReader::set_video_decoder` (`src/mp4/reader.rs:318`) は廃止し、mp4 reader が options を受け取って自前で decoder を spawn する。`obsws/source/file_mp4.rs:54-61` の `VideoDecoder::new` + `set_video_decoder` 経路は消失する。

#### RTMP / RTSP / SRT inbound endpoint

現状 `subscriber.rs` 構造体内に `decoder: Option<VideoDecoder>` を持って同期 pull していたが、Sender pattern では以下に書き換える:

- 構造体内に `decoder_join_handle: Option<tokio::task::JoinHandle<crate::Result<()>>>` と `decoder_input_tx: Option<mpsc::Sender<Message>>` を持つ
- decoder 起動時に `(decoder, decoded_rx) = VideoDecoder::new(...)` + `tokio::spawn(decoder.run(input_rx, output_tx, decoded_rx))` で task 化
- 受信したフレームは `decoder_input_tx.send(...).await` で投入
- 終了時に `decoder_input_tx` を drop → `join_handle.await?` で待つ

### `shiguredo-rust` 規約整合

- トレイト追加なし (`VideoDecoderInner` enum を維持)
- `#[non_exhaustive]` 不使用
- モック / スタブ不使用 (テストは実 decoder + tokio channel)
- 規約上の許可取得は不要

## 完了条件

- 「解決方法」§の step 1〜9 すべてが実装され、以下の旧構造がコードベースから完全に消えている:
  - `VideoDecoder.decoded: VecDeque<VideoFrame>`
  - `VideoDecoder::poll_output()`
  - `VideoDecoderInner::next_decoded_frame()` 系 dispatch
  - `drain_video_decoder_output` / `discard_video_decoder_output` / `discard_decoder_output` (audio) ヘルパ
  - `NvcodecDecoder::error_slot` および `decoded_queue` の中継キュー
  - `VideoToolboxDecoder::decoded: Option<VideoFrame>` フィールド
  - `Mp4FileReader::set_video_decoder` (`src/mp4/reader.rs:318`)
- `total_input_video_frame_count_metric.inc()` (現状 `:405` の 1 箇所) は入力 task 内に集約され、`total_output_video_frame_count_metric.inc()` (現状 `:415` の 1 箇所) も出力 task 内 1 箇所に共通ヘルパ (例: `forward_decoded_frame()`) で集約されている
- 「現状」§ 外部利用箇所表のすべての行が Sender 経路 (設計方針 §「外部利用箇所の Sender 化パターン」) に置き換わっている
- mp4 reader の `recreate_decoders` / `flush_decoders` / `reset_for_restart` / `apply_seek` がすべて async fn 化され、呼出元 15 箇所が `.await` 付与で追従している
- mp4 reader の `TrackSender` (`:1446`) の SYN/ACK バックプレッシャ (`MAX_NOACKED_COUNT = 100`) が decoder task 側で維持されている (回帰なし)
- `cuvidParseVideoData` callback dispatch スレッド規約の調査結果と、NvcodecDecoder の採用方式 (`blocking_send` / `try_send` / 他)、および callback 順序保証 (`pop_front()` None が起きないことの根拠) が本 issue に追記されている
- end-to-end テストが `src/decoder/nvcodec.rs` に追加され、以下の最小ケースを検証する:
  - (a) `NvcodecDecoder` で callback 内 `Err` が次回 decode 呼出を待たず Receiver に届くこと
  - (b) `Openh264Decoder` で keyframe 入力時に `finish()` 経由の旧 frame と新 keyframe 由来 frame が両方 Sender に送信され、順序が保たれること
  - (c) `VideoToolboxDecoder` で再初期化条件下 (SPS/PPS 変化など) で Sender 送信完了 → 再初期化 → 次フレーム送信の順序が保たれること
- 既存 `src/decoder.rs:720-821` のエンジン選択テストは inner variant pattern match を `#[tokio::test]` + Sender 形式で維持
- `Cargo.toml` に `tokio_util` workspace 依存が追加されている
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

設計方針 §で確定した方針に従って以下の step を実装する。各 step の詳細は設計方針 §の対応箇所を参照。

### 1. inner の Sender 化

設計方針 §「各 inner の Sender 化形態」に従う。全 inner のコンストラクタを `new(..., tx: tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>) -> crate::Result<Self>` に統一、`VideoDecoderInner::decode` を `async fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()>` に統一する。

### 2. VideoDecoder 構造体の書き換え

- `VideoDecoder.decoded: VecDeque<VideoFrame>` を廃止し、代わりに `tx: tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` を保持
- `VideoDecoder::new(options, stats)` の戻り値を `(VideoDecoder, mpsc::Receiver<...>)` のタプルにし、内部 channel の `rx` を返す
- `VideoDecoder::poll_output()` (`src/decoder.rs:422-430`) と `VideoDecoderInner::next_decoded_frame()` (`src/decoder.rs:706-717`) 系 dispatch を廃止

### 3. VideoDecoder::run() の 2 task 分離構造

設計方針 §「`run()` ループの構造」に従う。`run(self, input_rx, output_tx, decoded_rx)` 内で 2 sub-task (`run_input` / `run_output`) を `tokio::spawn` して `tokio::join!` で待ち合わせる。`tokio_util::sync::CancellationToken` で出力 task → 入力 task の shutdown 伝達を行う。EOS / エラー終了は §「EOS / エラー終了の経路」のケース A/B/C に従う。

### 4. drain_video_decoder_output / discard_*_output 廃止と外部利用箇所の書き換え

「現状」§ 外部利用箇所表の `drain_video_decoder_output` 5 call site、`discard_video_decoder_output` 1 call site、`discard_decoder_output` (audio) 1 call site、`VideoDecoder::new` 9 call site を、設計方針 §「外部利用箇所の Sender 化パターン」に従って書き換える。mp4 reader は §5 で扱う。

### 5. mp4 reader (`Mp4FileReader`) の再設計

設計方針 §「mp4 reader (`Mp4FileReader`) の再設計」の 1〜8 シーケンスを実装。呼出元 grep 結果 (`:339, :350, :378, :383, :465, :475, :479, :484, :494, :498, :503, :519, :523, :528, :645` 等) を `.await` 付与で追従。`Mp4FileReader::set_video_decoder` (`src/mp4/reader.rs:318`) と `obsws/source/file_mp4.rs:54, 61` の `VideoDecoder::new` + `set_video_decoder` 呼出を削除。`TrackSender` の SYN/ACK 背圧維持の具体形態 (decoder task に丸ごと move か、`MAX_NOACKED_COUNT` を内部 channel N に置換するか) を実装段階で確定。

### 6. NvcodecDecoder の error_slot 廃止と input_queue 共有化

設計方針 §「各 inner の Sender 化形態 NvcodecDecoder」に従う。削除対象:

- `error_slot: Arc<Mutex<Option<Error>>>` 型エイリアス (`src/decoder/nvcodec.rs:14`) とフィールド (`:25`)
- `decoded_queue: Arc<Mutex<VecDeque>>` の中継キューフィールドと型エイリアス (`:13, :24`)
- `handle_decoded_frames()` (`src/decoder/nvcodec.rs:220-295`) の error_slot 取り出しロジックと中継キュー pop ロジック

変更対象:

- `input_queue: VecDeque<VideoFrame>` を `Arc<Mutex<VecDeque<VideoFrame>>>` に変更
- `build_handler` (`src/decoder/nvcodec.rs:29-56`) のシグネチャを `fn build_handler(tx: Sender<crate::Result<VideoFrame>>, input_queue: Arc<Mutex<VecDeque<VideoFrame>>>) -> FnDecodeHandler<...>` に変更し、callback 内で input pop → NV12→I420 変換 → `tx.blocking_send(...)` を直接呼ぶ

### 7. VideoToolboxDecoder の reinitialize_if_need 再設計

設計方針 §「各 inner の Sender 化形態 VideoToolboxDecoder」に従う。削除対象: `decoded: Option<VideoFrame>` フィールド (`src/decoder/video_toolbox.rs:12`) と `reinitialize_if_need` 内ガード (`:131-134, :147-151, :180-183`)。

### 8. Initial に sender を含めない設計

- `VideoDecoderInner::Initial { options }` (`src/decoder.rs:537-553`) は現状の構造を維持
- `VideoDecoder` 構造体側に `tx: tokio::sync::mpsc::Sender<...>` を保持し、`initialize_decoder` (`src/decoder.rs:555-668`) で実 decoder 生成時に `self.tx.clone()` を渡す
- Initial 状態のまま EOS 到達時は `self.tx` を drop して終了

### 9. テスト書き換え

- 既存 `src/decoder.rs:720-821` のエンジン選択テスト (`vp9_without_size_skips_video_toolbox` 系) は `#[tokio::test]` に置換し、`(decoder, _rx) = VideoDecoder::new(...)` 形式で `inner` の variant pattern match を維持する最小改変
- 各 decoder 末尾テスト (`src/decoder/openh264.rs:161-247` の `build_annexb_input` テスト等) は本 issue の API 変更影響範囲のみ書き換え
- 新規 end-to-end テストを `src/decoder/nvcodec.rs` に追加 (完了条件の (a)(b)(c) を検証)
- テスト fixture は既存 `crate::video::h264::tests::SPS_320X240_ANNEXB` 系 (`src/video/h264.rs:911`、`pub(crate)` 可視性) を流用する
- end-to-end テスト雛形で参照する `build_test_h264_keyframe()` は本 issue 内で `src/video/h264.rs` の `pub(crate) mod tests` に新規追加する (既存 `SPS_320X240_ANNEXB` を流用して最小 H.264 keyframe バイト列 `[SPS, PPS, IDR slice]` を組み立てる)。新規追加もテスト書き換え範囲に含む

end-to-end テスト雛形:

```rust
#[tokio::test]
#[cfg(feature = "nvcodec")]
async fn test_nvcodec_decoder_to_receiver_e2e() {
    // CUDA 非搭載 CI では skip する
    if !shiguredo_nvcodec::is_cuda_library_available() {
        return;
    }

    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<crate::Result<VideoFrame>>(8);
    let input_queue =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));

    let decode_params = DecodeConfig::default();
    let mut decoder =
        NvcodecDecoder::new_h264(&decode_params, tx, input_queue.clone()).expect("decoder");

    // 実 H.264 fixture を投入
    let input_frame = crate::video::h264::tests::build_test_h264_keyframe();
    decoder.decode(&input_frame).await.expect("decode");
    decoder.finish().await.expect("finish");

    // Receiver でデコード結果を受信
    let frame = rx.recv().await.expect("receive").expect("ok frame");

    // 不変条件確認 (decoder 出力は raw I420)
    assert_eq!(frame.format, VideoFormat::I420);
    assert!(frame.sample_entry.is_none());
}
```

モック / スタブは不要 (`shiguredo-rust` 規約 OK)。

## CHANGES.md について

内部リファクタにつき記載不要。`VideoDecoder` 系は library として外部公開していないため、API 変更の後方互換影響は obsws coordinator / mixer / writer / subcommand 階層等の crate 内利用箇所のみ。

## 関連

- closed/0057 (`feature/refactor-callback-friendly-codec-interface`): 設計検討の親 issue。本 issue は §3 採用案 C の decoder 部分実装。本 issue でスコープを「decoder 内部 + 外部利用箇所」に拡大したため、closed/0057 §3 分割表も対称的に更新 (本 issue 完了時に追記、または 0067 polish 時に併せて更新)
- closed/0054 (`feature/refactor-encoder-defer-output-until-sample-entry-ready`): encoder で sample_entry 未確定時の出力を `Err` 化する fail-fast 整備。decoder 側は sample_entry 不変条件の対象外だが、本 issue でも採用案 C に従い `Result<VideoFrame>` 流路を採用
- open/0067 (`feature/refactor-video-encoder-sender-interface`): 後続 PR。本 issue で確立した C 形式 interface を encoder に展開する。依存順序: `0066 → 0067`。本 issue 完了時に 0067 polish で「encoder + 外部利用箇所」へスコープ対称拡大する
