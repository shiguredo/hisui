# NvcodecEncoder の flush() 撤廃と VideoEncoder レイヤーでの in-flight バックプレッシャ導入

- Priority: Medium
- Created: 2026-07-21
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-encoder-inflight-backpressure
- Polished: 2026-07-21
- Reporter: @sile
- Decision Owner: @sile

## 目的

`NvcodecEncoder::encode()` 内の `self.inner.flush()?;` (`src/encoder/nvcodec.rs:356`) が worker 完了を毎フレーム待つため NVENC 非同期パイプライン並列性を殺している。 本 issue は `flush()` を撤廃し、代わりに `VideoEncoder::run` の `tokio::select!` に in-flight カウンタベースのバックプレッシャを追加することで NVENC 並列性を回復させる。

closed/0080 の (α) 案 (`Pacer<T>` + `Condvar` の nvcodec レイヤーセルフペーシング) は tokio worker thread を block する設計欠陥により不採用となった (closed/0080 §解決方法 参照)。 本 issue はその後継として `VideoEncoder` レイヤーでの async task 内 usize カウンタによる bp を採用する。

**flush 撤廃と bp 導入は 1 目的 2 実装フェーズ**: `flush()` 単独撤廃だけだと NVENC 内部キューが `"encoder buffer is full"` エラーで溢れるため、両者は分割不能な 1 目的の 2 側面。

### closed/0067 unbounded 採用根拠の反転

`src/encoder.rs:520` の `tokio::sync::mpsc::unbounded_channel()` は closed/0067 §設計方針 で「Nvcodec の GPU 側先行投入数は現状の `flush()` 維持で制限される」として採用された。 本 issue で `flush()` を撤廃するとこの採用根拠が反転する。 対策:

- **VideoEncoder レイヤーの in_flight guard が GPU 側投入数を絞る** → callback 経由で `rx` に届く出力フレームも一定時間あたり最大 `IN_FLIGHT_LIMIT` 相当に収束する。 unbounded のまま overflow しない
- **上位 rx (unbounded) は本 issue で bounded 化しない** → closed/0066 §「unbounded channel 採用根拠」を踏襲する

## 優先度根拠

Medium。 closed/0080 と同じ NVENC 並列性回復を目的とし、実装コストは 0080 の (α) 案より小さい。

- closed/0080 §優先度根拠 の Medium 判定理由がそのまま適用される
- 実装 LOC 内訳:
  - `src/encoder/nvcodec.rs`: flush 撤廃 -1 行、flush 動機コメント削除 -4 行、順序保証コメント更新 ±3 行、callback Err 分岐で `pop_front` 追加 +2 行 → **計 ~10 LOC**
  - `src/encoder.rs`: `rx` の `Option` 化 +2 行、`VideoEncoder::new` の rx 初期化変更 +1 行、`poll_output` の `.as_mut().expect(...)` 経由化 +3 行、`run()` の select! 3 腕化 (input guard + output 腕新規 + inner take + explicit drop) ~35 行、drain ループ (`:789-803`) 削除 -15 行、`recv_video_encoder_rpc_message_or_pending` docstring 更新 ~3 行、`inner` / `rx` の drop 順制御コメント (`:485-491`) 更新 ~5 行、既存 test コメント (`:1236-1242`) の retro-fit ~3 行 → **計 ~40 LOC**
  - 実装本体合計 ~50 LOC
  - integration test 追加 ~50-100 LOC (別途)
- 依存: closed/0080 の分析資産を継承 (詳細は closed/0080 参照)

## 現状

`origin/develop` 時点の状態から作業する。 closed/0080 の実装コミット (`7c137068` / `705d11f0` / `7af05076`) は PR #318 が un-merged close されたため develop には取り込まれていない。

- `src/encoder/nvcodec.rs:356` の `NvcodecEncoder::encode` 末尾に `self.inner.flush()?;` が残っている
- `src/encoder/nvcodec.rs:366` の `NvcodecEncoder::finish` 内の `self.inner.flush()?;` は EOS 保証のため維持対象
- `src/encoder/nvcodec.rs:352-355` に flush 動機コメント (「投入直後に flush() で 1 フレーム分の完了を待って同期動作させる」)
- `src/encoder/nvcodec.rs:334-335` に順序保証コメント (「flush() は callback 完了までブロックするため、push が先行することが担保される」)
- `src/encoder/nvcodec.rs` の callback Err 分岐は `input_queue.pop_front()` を呼ばず、`sink.emit_err(...)` のみ
- `src/encoder/pacer.rs` は存在しない
- `src/encoder.rs:787` の `VideoEncoder::run` の `Message::Syn(_)` arm は `{}` で暗黙 drop
- `src/encoder.rs:738-755` の `VideoEncoder::poll_output` は `try_recv` ベース
- `src/encoder.rs:780-819` の `VideoEncoder::run` は input 腕 + RPC 腕の **2 腕 `tokio::select!`** 構成 (drain は input 腕内の `poll_output` ループ)
- `src/encoder.rs:494` の `VideoEncoder::inner` は既に `Option<VideoEncoderInner>` (初期化遅延用)
- `src/encoder.rs:495` の `VideoEncoder::rx` は `EncoderOutputReceiver` (`Option` 化対象)
- `src/encoder.rs:485-491` に drop 順制御コメント (現契約: `inner` を `rx` より先に drop、callback で最後の 1 frame が rx に届いてから rx drop)
- `Cargo.toml:99` の `shiguredo_nvcodec = { version = "=2026.2.0", optional = true }`
- `src/sora/recording_encoder_nvcodec_params.rs:252` の `frame_interval_p: 1` ハードコード

## 設計方針

### bp 機構の位置付け

closed/0080 の (α) 案は「nvcodec レイヤーで bp」だったが、本 issue は「`VideoEncoder` レイヤーで bp」に置き換える。

- `NvcodecEncoder` は同期 API のまま、`self.inner.flush()?;` (encode 内) を撤廃するだけ
- `VideoEncoder::run` に `in_flight: usize` + `IN_FLIGHT_LIMIT` を追加し、`tokio::select!` の input 腕に `if !is_eos && in_flight < IN_FLIGHT_LIMIT` guard を付ける
- LIMIT 到達時は `input_rx.recv()` を呼ばない。 上流の Syn/Ack 経路で mixer / reader 側の自主ペーシングが自然に停止する (Syn を明示的に保持する追加ロジックは書かず、Syn が queue に留まる → Ack 復帰しない、を利用する)
- `Message::Syn(_)` は encoder レイヤーで即 drop する (現状の `{}` 相当を維持)
- **tokio worker block なし**: async task 内 usize カウンタで完結。 `tokio::select!` の input 腕 guard が close されるだけで tokio worker thread の同期 block は発生しない。 closed/0080 の (α) 案が抱えた「compose `--thread-count 1` で他 processor の進行を止める」問題は本 issue では発生しない。 shiguredo_nvcodec の callback は CUDA worker スレッドで発火し、`sink.emit_ok → tx.send()` (unbounded channel、block なし) で `rx` に流すため、tokio worker とは独立

### bp 伝播の機序 (経路別)

上流の `send_syn` 主体は経路によって異なる:

- **compose 経路** (`src/sora/recording_subcommand_compose.rs`):
  - Syn 発火主体は reader (`src/sora/recording_reader.rs:36-45, 274-283` で 100 frame ごとに `send_syn` → `ack.await`)
  - reader が発する Syn の subscriber は reader_output_track を subscribe する **decoder のみ**。 decoder は `src/decoder.rs:489` の `Message::Syn(_) => {}` で drop するため、reader の Ack は decoder 到達時点で復帰する
  - encoder が in_flight bp で input を止めても reader 側の bp としては直接には効かない。 その代わり mixer → encoder 間 (unbounded) にフレームが積まれる。 積載量は「encoder GPU throughput と mixer 合成 rate の差 × 時間」で決まり、単純な frame 数 cap は存在しない (GPU が十分速い定常状態では mixer 出力の消費 pace で自然に平衡する)
- **realtime 経路** (`src/mixer/video.rs`):
  - Syn 発火主体は realtime mixer (`src/mixer/video.rs:126, 423-429` で `MAX_NOACKED_COUNT = 100` (`:17`) ごとに `output_tx.send_syn()` → `waiting_ack.await`)
  - encoder が in_flight bp で `input_rx.recv()` を止めると、Syn は encoder 側の unbounded queue に到達済みだが consume されない → Syn 内包の `mpsc::Sender<()>` clone が保持され、mixer の `waiting_ack.await` (`:425`) が block されて mixer の合成 pace が停止 (リアルタイム性への悪影響)
  - 本 issue のスコープでは block を許容し、issues/0086 で `tokio::time::timeout` によるフレームスキップを追加する
- **vmaf 経路** (`src/sora/recording_subcommand_vmaf.rs`): compose 経路と同型

### 実装ディテール

`VideoEncoder::run` を以下の 3 腕 `tokio::select!` に書き換える。 数値・変数名・エラーメッセージ等の細部は実装時に確定する:

```rust
const IN_FLIGHT_LIMIT: usize = 3;

pub async fn run(
    mut self,
    handle: ProcessorHandle,
    input_track_id: TrackId,
    output_track_id: TrackId,
) -> Result<()> {
    // (subscribe_track / publish_track / register_rpc_sender / notify_ready 等の
    //  既存の run() 冒頭の前処理は不変)

    // drop 順制御:
    // - 現契約 (src/encoder.rs:485-491): `inner` を `rx` より先に drop する
    // - `inner` は既に `Option<VideoEncoderInner>`。本 issue で `rx` も `Option` 化して
    //   run() 冒頭で take() → local に move する (split-borrow 回避)
    // - Rust の drop 順は「引数より後に宣言された local が先に drop」なので、
    //   local として rx を持つと return 時に rx が先に drop される
    //   → callback で最後の 1 frame が emit されるとき rx は消滅済み → panic
    // - したがって return 直前に `drop(inner)` を明示呼び出して inner を強制的に先に drop する
    let mut inner = self.inner.take().expect("BUG: inner must be Some at run() entry");
    let mut rx = self.rx.take().expect("BUG: rx must be Some at run() entry");
    let mut in_flight: usize = 0;
    let mut is_eos = false;

    loop {
        tokio::select! {
            // input 腕: EOS 未受信 かつ in_flight < LIMIT のときのみ enable (bp guard)
            message = input_rx.recv(), if !is_eos && in_flight < IN_FLIGHT_LIMIT => {
                match message {
                    Message::Media(sample) => {
                        // 実装時は既存 handle_input_sample のロジックを local `inner` 経由に置き換える
                        // (self.inner の代わりに local inner を使う)
                        // ... inner.encode(...) ...
                        in_flight += 1;
                    }
                    Message::Eos => {
                        // ... inner.finish() で全 in-flight を drain ...
                        is_eos = true;
                        // 同期 encoder (libvpx / openh264) では EOS 時点で in_flight=0 が典型。
                        // ここで早期終了しないと、次 iter で input 腕 guard で無効化、
                        // output 腕は rx 空で pending、RPC 腕も pending の 3 腕 pending で deadlock する
                        if in_flight == 0 {
                            drop(inner);
                            output_tx.send_eos();
                            return Ok(());
                        }
                    }
                    Message::Syn(_) => {}  // encoder レイヤーで drop → 上流 Ack 復帰
                }
            }
            // output 腕: 常時 enable、in-flight を drain
            result = rx.recv() => {
                let frame = result
                    .expect("encoder output channel disconnected unexpectedly (sink dropped before rx)")?;
                in_flight -= 1;
                if !output_tx.send_media(MediaFrame::video(frame)) {
                    drop(inner);
                    output_tx.send_eos();
                    return Ok(());
                }
                if is_eos && in_flight == 0 {
                    drop(inner);
                    output_tx.send_eos();
                    return Ok(());
                }
            }
            // RPC 腕: 既存 helper 経由で維持 (helper docstring は 3 腕構造前提に更新)
            rpc_message = recv_video_encoder_rpc_message_or_pending(
                rpc_rx_enabled.then_some(&mut rpc_rx)
            ) => {
                // (既存処理)
            }
        }
    }
}
```

設計上の要点:

- **`inner` の Option 化と明示 drop**: `VideoEncoder::inner` は既に `Option<VideoEncoderInner>` (初期化遅延用)。 本 issue では `run()` 冒頭で `self.inner.take()` して local `inner` に move し、return 直前に `drop(inner)` を明示呼び出しする
- **`rx` の Option 化**: 現行 `rx: EncoderOutputReceiver` を `rx: Option<EncoderOutputReceiver>` に変更。 `VideoEncoder::new` で `rx: Some(rx)` にラップ、`poll_output` (integration test 用) では `self.rx.as_mut().expect("...").try_recv()` に変更
- **同期経路 (libvpx / openh264)**: `handle_input_sample` 内で `emit_ok` が同期発火するため、in_flight は 0〜1 の間を往復する。 IN_FLIGHT_LIMIT に到達しないため guard は事実上 no-op (性能影響なし)
- **AudioEncoder への影響なし**: `AudioEncoder::run` (`src/encoder.rs:183-211`) は fdk-aac / opus / audio_toolbox すべて同期エンコーダで callback 完結型は存在しない。 本 issue の変更対象外
- **エラー経路の in_flight**: output 腕で `frame` が `Err` の場合、`in_flight -= 1` の前に `?` で return する形にする (実装時に `let frame = result.expect(...)?;` の順で書けば自動的にそうなる)

### NvcodecEncoder 側の変更

- `src/encoder/nvcodec.rs:356` の `encode()` 末尾の `self.inner.flush()?;` を撤廃
- `src/encoder/nvcodec.rs:352-355` の flush 動機コメントを削除
- `src/encoder/nvcodec.rs:334-335` の順序保証コメントを更新 (flush 前提 → Mutex + VecDeque + shiguredo_nvcodec worker FIFO):

    ```
    // 順序保証: callback で pop する前に必ず push_back する。
    // Mutex 排他 + VecDeque FIFO + shiguredo_nvcodec 内部 worker の FIFO 処理により、
    // 「push_back → encode → callback pop」の因果順序が担保される。
    ```

- `src/encoder/nvcodec.rs:366` の `finish()` の `self.inner.flush()?;` は EOS 保証のため **維持**
  - EOS 時 1 回のみ発火。 毎フレーム発火する encode() 内 flush() (closed/0080 で問題視) とは根本的に異なる
  - 発火時間見積もり: 最大 `IN_FLIGHT_LIMIT = 3` frame 分の GPU emit 待ち (1080p30 の NVENC で数 ms 〜 数十 ms オーダー)
  - `--thread-count 1` の compose 経路では writer / progress_bar が同期 block されるが、EOS 時 1 回のみのため実運用影響は限定的
- `input_queue` の型は現行 `Arc<Mutex<VecDeque<VideoFrame>>>` (metadata FIFO 用途) をそのまま維持 (Condvar / Pacer は導入しない)
- callback の Err 分岐で `input_queue.pop_front()` を追加する (現状: Err 分岐は pop していない)。 これは in-flight とは無関係で、`input_queue` のメタデータ FIFO 順序保証 (Ok 分岐が `pop_front` する契約) と対称にするため

### IN_FLIGHT_LIMIT の根拠

`IN_FLIGHT_LIMIT = 3`。 数学的上限と `frame_interval_p: 1` ハードコードの背景は closed/0080 §現状 §shiguredo_nvcodec 2026.2.0 の実測 を参照。

- **本 issue の `in_flight` の意味論**: `inner.encode()` 呼出済かつ `sink.emit_ok` 未受領のフレーム数。 shiguredo_nvcodec 内部 pending キューの pending 数とは **保守的上界** の関係 (`in_flight ≥ internal_pending`) が成立する
  - `inner.encode()` 呼出 → worker の Job::Encode 処理 → GPU 完了 → drain 経由 emit_ok → `rx.recv()` の順で時間ズレがあり、in_flight は常に internal_pending 以上
  - `in_flight < 3` の間しか投入しない → `internal_pending ≤ in_flight < 3 < 4 = n_encoder_buffer` で `"encoder buffer is full"` エラーを回避
- N=2 vs N=3 の実機計測比較で最終確定 (§実機計測 参照)
- 「LIMIT を上げるとリアルタイム性が下がる」のでリアルタイム性優先の観点で小さい値を維持する。 値を大きくする場合は svt_av1 の `look_ahead_distance` / video_toolbox の delay パラメータとの整合も要検討 (関連 issues/0087 参照)

### 不採用案 (本 issue 特有、closed/0080 の (β) / (γ) とは別内容)

closed/0080 の不採用案 (β) NVENC 側 `max_frames_in_flight` / (γ) 上位 bounded channel + `blocking_send` は closed/0080 §設計方針 §bp 機構の選定 参照。 混同を避けるため、本 issue の不採用案は A / B とする。

#### 案 A: Syn を writer に forward して end-to-end bp

encoder レイヤーで drop せず writer まで forward する end-to-end 方式は理論的には理想的 (writer 遅延も上流に伝わる) だが、正しく実装するには「Syn 受信時点で in-flight の全 Media が emit されて writer に転送されるまで Syn を保持」する必要がある (Syn だけ先に届くと writer の recv 順が壊れる)。 実装複雑度が現時点の bp 要件に対して過剰。 writer 遅延時の bp が要件になった場合に別 issue で再検討する。

#### 案 B: writer 側の実装変更 (write 完了時に Syn を drop)

writer が Syn を write 完了時に drop する形にすると、writer の内部バッファリング効果が消えて Syn 保持期間が過剰になる。 writer は現状の即 drop (`src/mp4/writer.rs:1008, 1035` の default arm で drop) で問題ない (writer の `input_rx.recv()` を呼ぶ頻度で pace が伝わる)。

### スコープ外

- **writer 遅延時の bp**: `encoder → writer` の unbounded channel (`MessageReceiver` は `tokio::sync::mpsc::UnboundedReceiver`) にフレームが溜まる可能性は本 issue の設計では取れない。 既存の hisui でも取れていない挙動。 writer 遅延が実観測された場合、上記案 A または writer 側 self-pacing を別 issue で検討する
- **decoder 側の in-flight bp**: 現状 nvcodec の decoder は bp なしで成立している (reader の Syn/Ack ペーシングと shiguredo_nvcodec::Decoder の内部設計)。 本 issue の変更で decoder 側の挙動は不変。 将来 `"buffer full"` 相当エラーが観測された場合は同型 (`VideoDecoder::run` に select! guard by in_flight) を別 issue で検討する (pending issue は observation-driven のため現時点では作らない)
- **compose + svt_av1 の warm-up deadlock リスク**: SVT-AV1 の `look_ahead_distance` デフォルト (native default ~33) の状態で本 issue の bp guard 発動時、theoretically には deadlock 可能。 実測で観測されたら別 issue で対処。 対処候補:
  - (I) shiguredo_svt_av1 に「継続可能な flush (flush_pending)」API 追加 + hisui 側で Syn 受信時に flush 依頼 (bp guard の設計変更込み)
  - (II) compose 経路でも `look_ahead_distance` を強制上書き
  - (III) `VideoEncoderInner` に `needs_backpressure()` を追加して guard を分岐 (encoder 種別で切り替え)
- **compose + video_toolbox の同型リスク** (macOS 環境で観測時に対応)
- **`NvcodecEncoder::encode` 内の同期 `self.inner.encode()` の同期コスト** (数 ms オーダー、GPU 転送含む) は現状の設計を維持
- **realtime 経路の低遅延化のための encoder パラメータ強制上書き**: issues/0087 で扱う

### shiguredo-rust 規約整合

- モック / スタブ不使用 (CLAUDE.md 準拠)
- 新規 trait / マクロ / re-export / `#[non_exhaustive]` 追加なし
- `.expect("MESSAGE")` で fail-fast (現状パターン踏襲)
- テストは PBT / 単体テスト / integration test の役割分担に従う (§完了条件 §テスト 参照)

## 完了条件

### コード変更

- `src/encoder/nvcodec.rs::encode` の `self.inner.flush()?;` が撤廃されている
- `src/encoder/nvcodec.rs::finish` の `self.inner.flush()?;` は維持されている
- `src/encoder/nvcodec.rs:352-355` の flush 動機コメントが削除されている
- `src/encoder/nvcodec.rs:334-335` の順序保証コメントが flush 撤廃後の担保 (Mutex + VecDeque + worker FIFO) に更新されている
- callback の Err 分岐で `input_queue.pop_front()` が実装されている
- `src/encoder.rs::VideoEncoder::rx` フィールドが `Option<EncoderOutputReceiver>` に変更されている
- `src/encoder.rs::VideoEncoder::new` の `rx` 初期化が `Some(rx)` に変更されている
- `src/encoder.rs::VideoEncoder::poll_output` 内で `self.rx.as_mut().expect("...").try_recv()` に変更されている
- `src/encoder.rs::VideoEncoder::run` の `tokio::select!` に `in_flight: usize` カウンタ + `is_eos: bool` フラグ + input 腕 guard (`if !is_eos && in_flight < IN_FLIGHT_LIMIT`) + output 腕 (`rx.recv().await`) + 既存 RPC 腕の 3 腕構造が実装されている
- `run()` 冒頭で `self.inner.take()` と `self.rx.take()` で local に move し、各 return 直前に `drop(inner)` を明示呼び出しする実装になっている
- `Message::Eos` arm 内で `in_flight == 0` の場合の早期終了 (`drop(inner)` → `send_eos` → `return Ok(())`) が実装されている
- `Message::Syn(_)` は encoder レイヤーで drop (現状の `{}` 相当を維持)
- `src/encoder.rs:485-491` の drop 順制御 docstring が `inner` / `rx` の Option 化と明示 drop 契約を反映して更新されている
- `src/encoder.rs:822-834` の `recv_video_encoder_rpc_message_or_pending` docstring が 3 腕構造前提に更新されている
- `src/encoder.rs:1236-1242` の既存 test `output_sink_emit_ok_panics_when_receiver_dropped` のコメントが新設計 (明示 drop 契約) の下で妥当か再確認され、必要なら更新されている
- `NvcodecEncoder::new_h264` / `new_h265` / `new_av1` / `encode` / `finish` の pub シグネチャは不変

### closed/0057 §3 分割表の更新

closed/0083 / 0084 と同じ precedent に従い、本 issue の実装 PR に以下を含める:

- `issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md:364` (表本体の 0080 行): `open/0080` を `closed/0080` に更新し、直後に 0085 の 5 セル行を追加
- L351 (encoder 系列依存順序) は既に無 prefix 統一済み (commit `4a0dc4ee`) のため、`0067 → 0080 → 0085` 相当の追記のみ検討

### grep 検証

- `rg 'self\.inner\.flush' src/encoder/nvcodec.rs` の hit が **1 件** かつ `pub fn finish` 内
- `rg '投入直後に flush' src/encoder/nvcodec.rs` の hit が **0 件** (撤廃対象コメント残骸検出)
- `rg 'IN_FLIGHT_LIMIT' src/encoder.rs` で const 宣言 1 件 + guard 参照 1 件が検出できる
- `rg 'in_flight' src/encoder.rs` で `+= 1` と `-= 1` がペアで検出できる

### テスト

本 issue の integration test は `tests/encoder_tests.rs` (既存の VideoEncoder 系) に追加する。 shiguredo-rust テスト規約 (PBT / Fuzzing / 単体テストの役割分担) に照らして、in-flight bp の状態遷移は状態空間が狭く単体テスト向き。 PBT / Fuzzing は本 issue のスコープ外。

**検証観点**:

- (a) libvpx VP8 (同期) で既存 test (`tests/encoder_tests.rs::video_encoder_run_processes_i420_via_pipeline`) が回帰しないこと。 特に guard が事実上 no-op で従来通り動くこと、EOS 時 `in_flight = 0` の早期終了が正しく発火することを確認
- (b) EOS 受信後に `in_flight = 0` かつ output rx が空になったら `send_eos` して `run` が Ok で return すること
- (c) `Message::Syn` 到達で encoder レイヤーで drop され、上流の `Ack.await` が復帰することを検証。 `tokio::time::timeout(Duration::from_secs(5), ack).await` で timeout 付きに包む (無限 hang 防止、モック / スタブ不使用と両立)
- (d) 実 nvcodec を使う (`#[cfg(feature = "nvcodec")]` guard の) integration test で bp 発火経路を検証。 CI 化困難のため CUDA 環境で Decision Owner が別途実施

### 実機計測

**担当**: Decision Owner (@sile) が別 GPU マシン (Ubuntu + NVIDIA GPU) で実施。

**計測条件**:

- GPU 型番、NVIDIA driver / CUDA バージョン、OS を本 issue 完了時に追記
- 1080p30 / 60 秒の compose を H.264 で実行
- hisui ビルド feature: `--features nvcodec`
- 各案 5 run + ウォームアップ 1 run、平均 ± 標準偏差
- N=2 と N=3 の比較 (wall-clock と p99 latency のトレードオフを確認)
- `--thread-count 1` (デフォルト) と `--thread-count 4` の両方で計測 (本 issue の設計では tokio worker block が発生しないため、`--thread-count 1` でも他 processor が block されないことを検証する)

**達成基準** (暫定、実装着手前に Decision Owner が確定):

- 現状 (flush あり) と本 issue の wall-clock 時間を比較して **wall-clock 短縮 15% 以上**
- 同計測で **p99 frame latency 改善 5ms 以上**
- `"encoder buffer is full"` エラーが発生しないこと
- `--thread-count 1` で writer / progress_bar が block されないこと (mixer は bp で block されるが意図した挙動)

**未達時の close 経路** (closed/0080 と同型):

- close せず、実測数値を残懸念として本 issue に追記
- Decision Owner が (a) 上限 `IN_FLIGHT_LIMIT` の再調整、(b) 別 bp 機構の再選定、(c) priority 降格、(d) CHANGES.md 草案の破棄 のいずれかを判断

### cargo

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo check --workspace --features nvcodec`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`
- **`cargo test --features nvcodec`** (CUDA 環境で Decision Owner が別途実施、CI 現状未対応)

すべて通る。

## 解決方法

polish 完了後に追記する。

## CHANGES.md について

実装 PR 開設時点で `## develop` セクションに以下の草案を追加し、実機計測完了後に X の数値を埋める。 未達の場合は §完了条件 §未達時の close 経路 に従う (草案の破棄含む):

```
- [UPDATE] nvcodec エンコーダーの非同期パイプライン並列性を回復させて 1080p30 の合成 wall-clock 時間を X% 短縮する
  - NvcodecEncoder::encode() の flush() 強制同期化を撤廃し、VideoEncoder レイヤーで in-flight カウンタによるバックプレッシャ機構を導入する
  - Nvcodec が使えない環境 (macOS / CUDA なし Linux) では効果ゼロ
  - @sile
```

## 関連

- issues/0086 (`feature/add-realtime-video-mixer-skip-on-encoder-backpressure`): 本 issue の後続。 realtime mixer 側で encoder 詰まり時のフレームスキップを追加する。 本 issue の in-flight bp が入って初めて mixer が「詰まり」を認識できる依存関係を持つ
- issues/0087 (`feature/add-realtime-encoder-param-override`): 本 issue の関連。 リアルタイム動作時に全 encoder に共通の低遅延パラメータを強制上書きする機構を追加する。 本 issue の bp guard で顕在化する encoder 内部 warm-up 遅延を、encoder パラメータの強制上書き (svt_av1 の `look_ahead_distance` / video_toolbox の delay パラメータ等) で解消する
- closed/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`、 2026-07-21 closed、close commit `9892ec2a`、 PR #318 は un-merged close): 直接の前身。 (α) 案が tokio worker block 問題で不採用となり本 issue に方針変更した経緯を持つ
- その他の関連 issue (closed/0057 §3、closed/0067、closed/0079、closed/0083、closed/0084) は closed/0080 §関連 を参照
