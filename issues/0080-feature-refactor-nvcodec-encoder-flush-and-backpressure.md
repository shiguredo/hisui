# NvcodecEncoder の flush() 強制同期化を撤廃してバックプレッシャ機構を導入する

- Priority: Medium
- Created: 2026-07-07
- Completed: 2026-07-21
- Model: Opus 4.7
- Branch: feature/refactor-nvcodec-encoder-flush-and-backpressure
- Polished: 2026-07-10
- Reporter: @sile
- Decision Owner: @sile

## 目的

`NvcodecEncoder::encode()` 内の `self.inner.flush()?;` (`src/encoder/nvcodec.rs:356`) が worker 完了を待つため毎フレーム強制呼出しており、 NVENC が本来持つ非同期パイプライン並列性を全く活かせていない。 本 issue はこの `flush()` 強制同期化を撤廃し、 内部キュー上限 (Condvar ベース) のセルフペーシング機構 (bp 機構) を導入することで NVENC 非同期パイプライン並列性を回復させる。

closed/0057 §3 採用案 C の中核動機 (NVENC 並列性回復) の直接的な達成 issue。 closed/0067 (Sender 化 + `error_slot` 廃止) 完了で下地 (Sender 経由の Err 伝搬 + メトリクスペアリング + callback から `sink.emit_ok` への直接 push) が整い、 closed/0083 (wrap 削除 + `AsyncVideoEncoder` → `VideoEncoder` rename) と closed/0084 (未使用 API 削除 + `EncoderOutputSender` の `pub(crate)` 化) で型面もクリーンな状態に到達。 本 issue はその上に bp 機構を差し込む。

### closed/0067 unbounded 採用根拠の反転

`src/encoder.rs:519` の `tokio::sync::mpsc::unbounded_channel()` は closed/0067 §設計方針 §Sender の流路 §L104 で「Nvcodec の GPU 側先行投入数は現状の `flush()` 維持で制限されるため unbounded による無制限投入の懸念は生じない (flush() 撤廃時の bp 機構は別 perf issue で扱う)」として採用された。 本 issue で flush() を撤廃するとこの採用根拠が反転する。 対策方針:

- **inner の bp 機構 (α 案) が GPU 側投入数を絞る** → 上位 rx への流入を間接的にペースする。 定常時の rx 上限は N (bp 上限) 相当に収束する
- **上位 rx (unbounded) は本 issue で bounded 化しない** → closed/0066 §「unbounded channel 採用根拠」(callback blocking_send は deadlock パスを持つ / bp は下流 `TrackPublisher` の lag drop で発生) を踏襲する

## 優先度根拠

Medium。

- closed/0057 §3 §決定で採用案 C が確定した時点で Medium 維持の中核理由 (NVENC 並列性回復) として位置付けられていた
- 依存先 closed/0067 (Sender 化)、 closed/0079 (使用側移行)、 closed/0083 (wrap 削除 + rename)、 closed/0084 (未使用 API 削除) はいずれも完了済みで即着手可能
- Nvcodec が使えない環境 (macOS / CUDA なし Linux) では効果ゼロだが、 実運用では GPU 環境が主流のため影響は大きい
- 実装 LOC 見積もり: bp 用の薄い型 (`Pacer`) 新規追加 ~50 LOC + nvcodec.rs の 4 箇所修正 ~30 LOC + 単体テスト ~100 LOC で計 ~200 LOC 前後 (Medium 判断の再現材料)

## 現状

closed/0067 / 0083 / 0084 完了後の `src/encoder/nvcodec.rs` (全 451 行) と `src/encoder.rs` を基準とする。 行番号は着手時に `rg 'flush\|input_queue' src/encoder/nvcodec.rs src/encoder.rs` で再特定する (以下は 2026-07-10 時点の実測位置)。

### `NvcodecEncoder::encode()` の flush() 強制同期化

`src/encoder/nvcodec.rs:295-357` の encode() は、 push_back → `self.inner.encode()` → `self.inner.flush()?;` (`:356`) → `Ok(())` の順で実行する。 L352-355 のコメントが理由を説明する:

```
// shiguredo_nvcodec のエンコーダーは内部の worker スレッドで非同期にエンコードし、
// encode() は即時 return する。 上位パイプラインは同期 pull 型で、 上位側でペース制御
// しないと内部キューが溢れて encode() が "encoder buffer is full" で失敗するため、
// 投入直後に flush() で 1 フレーム分の完了を待って同期動作させる。
```

現状は上位のペース制御が存在しないため `flush()` で 1 フレーム分の完了を強制的に待って同期動作させている。 これにより NVENC 内部の非同期 worker パイプラインが 1 フレームごとに同期障壁で止まり、 GPU 側の投入並列度が事実上 1 になっている。

### `NvcodecEncoder::finish()` の flush()

`src/encoder/nvcodec.rs:364-368`:

```rust
pub fn finish(&mut self) -> crate::Result<()> {
    // flush で in-flight 完了を待ち合わせる
    self.inner.flush()?;
    Ok(())
}
```

`finish()` 内の `flush()` (`:366`) は EOS 経路で in-flight の残物を確実に flush するために必須。 本 issue では **維持する** (bp 機構と直交する EOS 保証)。

### 0067 / 0083 / 0084 完了後の callback → sink.emit_ok 経路

- 0067 で `NvcodecEncoder::build_handler` (`src/encoder/nvcodec.rs:59-135`) は callback スレッドで直接 `sink.emit_ok(VideoFrame)` に流す形に変更済み。 中継バッファ `encoded_queue` とエラー退避スロット `error_slot` は廃止済み
- 0083 で wrap 型 `VideoEncoder` (同期) は削除、 `AsyncVideoEncoder` は `VideoEncoder` にリネーム済み (`src/encoder.rs:477` の `pub struct VideoEncoder`)
- 0084 で未使用 API `next_encoded_frame` は削除済み、 `EncoderOutputSender` は `pub(crate)` 化済み
- 上位 `VideoEncoder` は unbounded `tokio::sync::mpsc::UnboundedReceiver` で受ける (`src/encoder.rs:519` で `unbounded_channel()` 生成)

### callback の Err 分岐は pop_front / notify_one を発火しない (bp 実装時に反転必要)

`src/encoder/nvcodec.rs:65-135` の `build_handler` を実測すると、 **Ok 分岐は `input_queue.pop_front()` を呼ぶが、 Err 分岐 (L132-134) では `pop_front` も `notify_one` も呼ばない**。 現状は encode() が毎回 `flush()` で 1 フレーム完了を待つため in-flight は最大 1 で未顕在化するが、 本 issue の (α) 案では `input_queue.len()` を in-flight として使用するため、 Err 分岐で pop も notify もされないと encode() の Condvar wait が永久ループする経路が生まれる。 **本 issue で Err 分岐にも `pop_front + notify_one` を追加する契約に反転させる** (§設計方針 §(α) 案の実装ディテール 参照)。

### 入力キュー順序保証コメント

`src/encoder/nvcodec.rs:334-335`:

```
// 順序保証: callback で pop する前に必ず push_back する。
// flush() は callback 完了までブロックするため、 push が先行することが担保される。
```

このコメントは flush() 前提。 本 issue で flush 撤廃後は「`input_queue` の `Mutex` 排他 + VecDeque FIFO + shiguredo_nvcodec worker の pending キュー FIFO」で担保する形に切り替わる (実質的な順序保証は不変)。 コメント更新を本 issue 完了条件に含める。

### shiguredo_nvcodec 2026.2.0 の実測

`Cargo.toml:99` で `shiguredo_nvcodec = "=2026.2.0"` (2026-07-10 時点)。 crate 内実装の実測 (追加の GPU 実機検証は不要):

- `EncoderConfig` に `max_frames_in_flight` 相当の pub パラメータは **存在しない** (`encode.rs:204-253`)
- `n_encoder_buffer` は内部計算 `frame_interval_p + 3` で決まる (`encode.rs:515`)
- 内部 pending キュー上限超過時は `"encoder buffer is full"` エラーを返す (`encode.rs:1572`)
- 内部 worker は FIFO 順で処理 (`encode.rs:1554`)
- `impl Drop for Encoder` は `Job::Terminate` 送信 → worker join → `send_eos` → `send_pending_drain_requests` → `wait_all_drains` を実行 (`encode.rs:1347-1364` / `:1639-1657`)。 **Drop 時の in-flight 全 drain は保証されている**

hisui 側での `frame_interval_p` は `src/sora/recording_encoder_nvcodec_params.rs:252` で **`frame_interval_p: 1` にハードコード** されている (nvcodec_h264 / h265 / av1 全経路共通)。 従って実運用の `n_encoder_buffer = 1 + 3 = 4`。 (α) 案の bp 上限 N はこの値未満に設定する必要がある。

## 設計方針

closed/0057 §3 §決定 §採用理由 §採用基準スキップ (L291-292) が「flush 撤廃自体は採用案 C で技術的に達成可能 (bounded `tokio::sync::mpsc::channel` で bp を発生させ、 callback ハンドラ内で `tx.blocking_send` / `tx.try_send` 経路で出力 → `flush()` 強制を撤廃できる)」と bp 経路を想定していた。 本 polish で下記の実測結果に基づき (α) を採用に確定する。

### bp 機構の選定 (本 polish で確定)

closed/0057 §3 §2 で議論された案から本 issue で確定させる:

| 本 issue | closed/0057 | 判定 | 理由 |
|---|---|---|---|
| (α) 内部キュー上限セルフペーシング | 案 D-1 | 採用 | 影響範囲が `src/encoder/nvcodec.rs` に閉じる。 shiguredo_nvcodec 依存に手を入れない。 unbounded channel の採用根拠と両立 |
| (β) NVENC 側 `max_frames_in_flight` | (無し、 新規追加) | 棄却 | shiguredo_nvcodec 2026.2.0 の `EncoderConfig` に該当 pub パラメータ不存在 (§現状 §shiguredo_nvcodec 2026.2.0 の実測 参照) |
| (γ) 上位 bounded channel + `blocking_send` | 案 D-2 | 棄却 | callback は CUDA worker スレッド上で発火するため、 `tx.blocking_send` を採用すると tokio worker (同一 runtime 上の他 processor と共用) を block する deadlock パスを持ち込む (closed/0066 / 0067 §unbounded 採用根拠と正面衝突) |

closed/0057 §3 §決定 §D-1 棄却理由 (「nvcodec のみで完結する局所修正で、 今後の HW 非同期化トレンドに対する将来コストが残る」) は interface 層 (案 C: 全 inner Sender 統一) の議論であり、 案 C 採用後の bp 実装層で D-1 相当を局所化するのは interface と bp の責務分離として適切。

### (α) 案の実装ディテール

**待ち合わせ機構の選択 (`std::sync::Condvar`)**:

- 呼出元 `VideoEncoder::handle_input_sample` (`src/encoder.rs:705`) は `pub fn` で **同期 fn**。 encode() 経路も全て sync
- `tokio::sync::Notify` (async 版) は不適で、 `std::sync::Condvar` が正解
- 現状 `flush()` も shiguredo_nvcodec 内部で同期 block するため、 同期 block 自体の位置付けは変わらない
- 現状 `src/encoder/nvcodec.rs:1-2` の use は `Arc, Mutex, OnceLock` のみ。 実装時に `use std::sync::Condvar;` と `use std::time::Duration;` を追加する

**`input_queue` 型変更と `Pacer` 型の位置付け**:

- 現状 `type SharedInputQueue = Arc<Mutex<VecDeque<VideoFrame>>>` (`src/encoder/nvcodec.rs:15`)
- 変更後は薄い new-type `struct Pacer<T> { queue: Mutex<VecDeque<T>>, cv: Condvar, limit: usize }` を feature 非依存な場所 (例: `src/encoder/pacer.rs`) に切り出し、 `type SharedInputQueue = Arc<Pacer<VideoFrame>>` として nvcodec は Pacer を保持する。 Pacer が単体テスト対象を兼ねる (§完了条件 §テスト 参照)
- 影響箇所は 4 箇所: (i) `pacer.rs` 新規 + `SharedInputQueue` 型変更、 (ii) `build_encoder` (`:150-176`) の `Arc::new(Mutex::new(VecDeque::new()))` を `Arc::new(Pacer::new(N))` に、 (iii) `build_handler` (`:59-135`) 内の `input_queue.lock()` を `input_queue.pop()` 相当の Pacer API 経由に、 (iv) `encode()` (`:295-357`) 内の push_back / wait を `input_queue.push_wait()` 相当の Pacer API 経由に

**encode() 側 wait ループ (擬似コード)**:

```rust
// Pacer 内部の擬似実装 (encode() 呼出側は input_queue.push_wait(frame, N) 相当を呼ぶだけ)
let mut guard = self.queue.lock().expect("nvcodec input queue lock poisoned");
// spurious wakeup 対策で while (if だと誤起床で条件破りうる)
while guard.len() >= self.limit {
    let (new_guard, _timeout_result) = self.cv
        .wait_timeout(guard, Duration::from_millis(100))
        .expect("nvcodec input queue condvar poisoned");
    guard = new_guard;
}
guard.push_back(frame);
drop(guard);
// caller は Mutex ホールド外で self.inner.encode(...) を呼ぶ
```

**callback 側 pop + notify のタイミング** (Mutex ホールドスコープ規定):

- Ok 分岐: `{ let mut q = queue.lock(); q.pop_front(); }` (block scope で lock を pop_front 直後に解放) → `sink.emit_ok(...)` → `cv.notify_one()`
- Err 分岐 (本 issue で追加): 同じく `{ let mut q = queue.lock(); q.pop_front(); }` → `sink.emit_err(...)` → `cv.notify_one()`
- **`pop_front` のみ lock 内、 `emit_ok` / `emit_err` / `notify_one` は lock 解放後** に実行する。 `emit_ok` の中で `tx.send` 等が発生するため、 lock 中に emit すると encode() 側の再 push が Mutex 待ちで空転する性能事故を招く。 現状 `src/encoder/nvcodec.rs:70-75` の Ok 分岐が既にこの pattern (`{ let mut queue = ... ; queue.pop_front() }`) を使っているので、 それを踏襲する

**上限 N の初期値と選定**:

- `n_encoder_buffer = frame_interval_p + 3 = 4` (現状 hisui は `frame_interval_p: 1` 固定)
- 上限 N は `n_encoder_buffer - 1 = 3` を数学的上限に、 実機計測で N = 2 / 3 を比較して確定 (N > n_encoder_buffer - 1 は buffer full を招くため意味を持たない)
- N=2 と N=3 の比較動機: N=3 は GPU 側パイプライン最大活用 (wall-clock 短縮最優先)、 N=2 は memory footprint 削減 + wait 発生時の起床頻度が上がり平均滞留時間が短くなる可能性 (p99 latency 最優先)。 実機計測で wall-clock と p99 のトレードオフを見て確定
- 将来 `frame_interval_p` を config 化する場合は N = `frame_interval_p + 2` の式で追従

**wait タイムアウト D**:

- 100ms 程度の short timeout で periodic に条件再評価 (spurious wakeup 対策の `while` ループとは独立の safety net)
- **inner drop 検出は本 polish 時点で反実仮想と判定**: encode() 呼出は単一 caller thread の同期呼出 (`handle_input_sample` 経由)、 Rust ownership rules 上、 encode() リターン前に `NvcodecEncoder::Drop` は発火不可能。 shiguredo_nvcodec 内部の `Encoder::Drop` は Terminate → worker.join で全 drain 保証 (§現状 §shiguredo_nvcodec 2026.2.0 の実測 参照)。 timeout 再評価は safety net として残すが、 明示的な drop 検出は不要

**Poison 時の方針**:

- `Mutex` の poison / `Condvar` の poison は既存の nvcodec.rs L72-74 と同様に `.expect("nvcodec input queue lock/condvar poisoned")` で panic させる (既存方針継承)
- bp N > 1 で callback スレッドが lock を保持する経路は増えるため、 callback panic → 待機側 encode() が poison expect で panic → tokio worker で unwind → runtime abort、 の流路が想定される
- 「callback panic は encoder プロセス全体を止める前提でよい」ことを Decision Owner が確認する。 変更が必要なら本 issue でスコープを追加する

### encode() からの flush() 撤廃

`src/encoder/nvcodec.rs:356` の `self.inner.flush()?;` を撤廃する。 encode 後は投入直後に return し worker 完了を待たない。 callback → `sink.emit_ok` の経路は 0067 で確立済み、 完了フレームは非同期に上位 `VideoEncoder.rx` に届く。 撤廃直前 (push_back の直前) に上記 (α) の in-flight wait を配置する。

### finish() の flush() 維持

`src/encoder/nvcodec.rs:366` の `self.inner.flush()?;` は EOS 時の残物完全 flush のために必須で維持する。 本 issue の変更対象外。

### 入力キュー順序保証コメントの更新

`src/encoder/nvcodec.rs:334-335` のコメントを以下に更新する (flush 前提 → Mutex + VecDeque + shiguredo_nvcodec worker FIFO で担保):

```
// 順序保証: callback で pop する前に必ず push_back する。
// Mutex 排他 + VecDeque FIFO + shiguredo_nvcodec 内部 worker の FIFO 処理により、
// 「push_back → encode → callback pop」の因果順序が担保される。
```

### VideoEncoder wrapper 側への波及検証

- **drop 順制御 (`src/encoder.rs:485-491`)**: 現状の宣言順 (`inner` → `rx`) は本 issue で不変。 in-flight 拡大 (最大 1 → 最大 N) で drop 時の callback 発火数が増えるが、 順序制約 (`rx` が `inner` より長く生存) は不変で問題なし
- **`shiguredo_nvcodec::Encoder::Drop` の drain 保証**: crate source 実測で Terminate → worker join → send_eos → wait_all_drains の順で drain 保証されている (§現状 §shiguredo_nvcodec 2026.2.0 の実測 参照)。 `NvcodecEncoder::Drop` の追加実装は不要
- **メトリクスペアリング (`src/encoder.rs:470-475` docstring)**: 現状の drain 契約 (VideoEncoder を drop する前にエンコード結果を drain し切る) を実装完了時に再確認。 検証対象の使用側は 3 経路 — (i) `src/sora/recording_subcommand_compose.rs` の `encoder.run(...).await`、 (ii) `src/sora/recording_subcommand_vmaf.rs` の同型、 (iii) `create_video_processor(_with_params)` (`src/encoder.rs`)。 `VideoEncoder::run` は EOS → poll_output ループの `Finished` でクリーン終了する構造のため drain 担保あり (`src/subcommand_list_codecs.rs` は `get_engines()` static fn のみで drain 経路なし、 対象外)
- **tokio runtime worker 同時 block ピーク**: compose 経路は 1 pipeline 1 encoder (`recording_subcommand_compose.rs:565-592` で `spawn_processor_task` に閉じ、 grid の複数セルは `VideoMixer` が合成 → 1 個の `VideoEncoder` に流れる)。 vmaf も同型で 1 pipeline 1 encoder。 従って **単 pipeline 単一 encoder の block は問題化しない**。 複数 encoder が同一 runtime に載る実例は (i) hls / dash で解像度別出力用に `create_video_processor` を複数呼ぶケース、 (ii) 多 pipeline 同時実行時の合計 encoder 数、 の 2 パターン。 これらの環境で bp α の `Condvar::wait_timeout(100ms)` block ピークが tokio worker 数を上回ると latency が悪化する可能性があるため、 実機計測で確認する
- **RPC keyframe 応答性**: `force_keyframe_next` (`src/encoder/nvcodec.rs:350`) は encode() 呼出時点で consume されるが、 GPU 側の実 keyframe 生成は非同期。 flush 撤廃で「RPC → 次入力 → 次 encode 投入 → GPU 側で最大 N フレーム遅延で keyframe 実生成」となり、 現状より keyframe 実観測タイミングが最大 N フレーム分遅延する既知挙動になる (機能的な破綻ではない)

### shiguredo-rust 規約整合

- モック / スタブ不使用 (`Condvar` は `std::sync` 標準機能)
- 新規 trait 追加なし
- `#[non_exhaustive]` 不使用

## 完了条件

### コード変更

- `src/encoder/nvcodec.rs:356` の `self.inner.flush()?;` が撤廃されている (encode 内)
- `src/encoder/nvcodec.rs:366` の `self.inner.flush()?;` は維持されている (finish 内、 EOS 保証)
- (α) 案の in-flight 上限管理が実装されている:
  - `input_queue` の型を `Arc<(Mutex<VecDeque<VideoFrame>>, Condvar)>` 相当に拡張
  - encode() で `len() >= N` の間は `condvar.wait_timeout(guard, Duration::from_millis(100))` で待機
  - callback Ok 分岐で `pop_front` → `sink.emit_ok` → `notify_one`
  - **callback Err 分岐でも `pop_front` → `sink.emit_err` → `notify_one` を実行する契約に反転**
  - 上限 N の初期値: 3 (`n_encoder_buffer - 1`、 実機計測で 2 との比較)
- `src/encoder/nvcodec.rs:334-335` の順序保証コメントが flush 撤廃後の担保 (Mutex + VecDeque + worker FIFO) に合わせて更新されている
- `NvcodecEncoder::new_h264` / `new_h265` / `new_av1` / `encode` / `finish` の pub シグネチャは不変

### grep 検証

- `rg 'self\.inner\.flush' src/encoder/nvcodec.rs` の hit が 1 件かつ `pub fn finish` 内であること (行番号は撤廃後のシフトを許容)
- `rg '投入直後に flush' src/encoder/nvcodec.rs` の hit が 0 件 (撤廃対象コメントの残骸検出)

### テスト

- **bp 待ち合わせロジックの単体テスト** を追加。 in-flight カウント + Condvar wait / notify のロジックを feature 非依存な薄い型 (例: `struct Pacer` を `src/encoder/pacer.rs` 等に切り出し) にまとめ、 `cargo test --workspace` (default feature) でも走らせる。 型自体は `Arc<(Mutex<VecDeque<T>>, Condvar)>` + `N: usize` のみで完結させ、 nvcodec 依存を持ち込まない。 テスト観点:
  - (i) `len() >= N` で wait が発生する
  - (ii) callback 側 pop + notify で wait が起こされる
  - (iii) timeout 経路で待機継続
  - (iv) 順序が FIFO で保持される
  - (v) **Err 経路でも pop + notify で in-flight が解放される** (F1 デッドロック回帰の防止)
- **テスト同期パターン** (flaky 回避のため Pacer API 設計時点で確定): (a) 書き手が wait に入った状態を保証するため `std::sync::Barrier` を使う (もしくは `AtomicBool` + `spin_loop_hint` で確認)、 (b) pop 側は `std::thread::spawn` で別スレッド化、 (c) timeout 経路検証は `Instant::now()` で min 経過を assert (`thread::sleep(50ms)` の素朴待ちは禁止)、 (d) callback スレッド模擬は Pacer 単独で完結できるよう `pop()` API を用意する
- integration test は実機必須のため CI 化困難 (§実機計測 §担当参照)

### 実機計測

**担当**: Decision Owner (@sile) が別 GPU マシン (Ubuntu + NVIDIA GPU) で実施

**計測条件**:

- GPU 型番、 NVIDIA driver / CUDA バージョン、 OS を本 issue 完了時に追記 (実測値埋め)
- 1080p30 / 60 秒の compose を H.264 で実行
- hisui ビルド feature: `--features nvcodec`
- 素材の出所と保存先: closed/0057 §2 §L255-259 の運用ルールに従い本 issue に記録
- 各案 5 run + ウォームアップ 1 run、 平均 ± 標準偏差
- **tokio worker 数 < 同時 encode 数の環境でも実施** (§設計方針 §tokio runtime worker 同時 block ピーク 参照)

**達成基準** (現時点は closed/0057 §2 §L261-263 由来の暫定値。 実装着手前に Decision Owner が机上評価で確定させて本 issue に上書き更新する):

- 現状 (flush あり) と本 issue (flush 撤廃 + bp α 案) の wall-clock 時間を比較して **wall-clock 短縮 15% 以上** (暫定)
- 同計測で **p99 frame latency 改善 5ms 以上** (暫定)
- `"encoder buffer is full"` エラーが発生しないこと

**未達時の close 経路**:

- close せず、 実測数値を残懸念として本 issue に追記
- Decision Owner が (a) 上限 N の再調整、 (b) 別 bp 機構の再選定、 (c) priority 降格 (Low or pending 移動)、 (d) CHANGES.md 草案の破棄 のいずれかを判断
- 判断結果を本 issue に追記のうえ、 実装は次リビジョンに繰り越すか close する

### cargo

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo check --workspace --features nvcodec` (feature 有効化下の build 保証、 test 実行は CUDA 環境で別途)
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace` (bp `struct Pacer` の単体テストはここで走る)
- `cargo test --workspace --no-default-features`
- **`cargo test --features nvcodec`** (CUDA 環境で Decision Owner が別途実施、 CI 現状未対応)

すべて通る。

## 解決方法

### 実装した内容

本ブランチ (`feature/refactor-nvcodec-encoder-flush-and-backpressure`) で以下を実装した:

- `src/encoder/pacer.rs` を新設し、`Pacer<T>` (Mutex + Condvar + VecDeque + 上限 N) 型を実装
- `src/encoder/nvcodec.rs` の `input_queue` を `Arc<Pacer<VideoFrame>>` に変更
- `NvcodecEncoder::encode` から `self.inner.flush()?;` を撤廃し、直前に `push_wait` (in-flight 上限セルフペーシング) を配置
- callback の Err 分岐にも `pop` (相当) を追加 (F1 デッドロック回帰防止)
- 単体テスト (i)-(v) を `pacer.rs` に追加

コミット履歴: `7c137068` / `705d11f0` / `7af05076` (PR #318)

### 不採用として close する理由

実装レビューで以下の設計上の欠陥が判明したため、本 issue の (α) 案は **不採用** として close する。

- `Pacer::push_wait` の `Condvar::wait_timeout(100ms)` は OS スレッドを block する同期 API
- 呼出元 `VideoEncoder::run` (`src/encoder.rs:764-820`) は `MediaPipelineHandle::spawn_processor` (`src/media_pipeline.rs:622`) 内で `tokio::spawn(async move { ... })` として登録される通常の async task で、`spawn_blocking` ではない
- したがって `push_wait` は tokio worker thread を最大 100ms 単位で block する
- compose サブコマンドのデフォルトは `--thread-count 1` (`src/sora/recording_subcommand_compose.rs:80-92` の `.default("1")`) で、単一 worker 上に decoder / mixer / encoder / writer / progress_bar 等の全 processor が同居する
- 単 pipeline 単一 encoder であっても、encoder の block は同 worker 上の他 processor の進行を止める経路になる
- 本 issue §設計方針 §tokio runtime worker 同時 block ピーク (L194) の判断根拠 (「単 pipeline 単一 encoder の block は問題化しない」) は暗黙のうちに `worker_threads >= 2` を前提としており、現行のデフォルト値と齟齬

§完了条件 §未達時の close 経路 (L253-257) の「(b) 別 bp 機構の再選定 → 実装は次リビジョンに繰り越す」に該当する。

### 後続 issue と方針変更

後続 issue: **issues/0085** (`feature/refactor-encoder-inflight-backpressure`)

新方針:

- bp 機構を `NvcodecEncoder` レイヤー (Condvar による同期 block) から `VideoEncoder` レイヤー (tokio の async task 内 usize カウンタ) に移す
- `VideoEncoder::run` の `tokio::select!` に `in_flight: usize` + `IN_FLIGHT_LIMIT` guard を追加し、上限到達時は `input_rx.recv()` を呼ばず上流 Syn/Ack 経路で自然に bp を伝える
- Mutex / Condvar / async 化はすべて不要 (async task 内ローカル状態で完結)
- `src/encoder/pacer.rs` は削除、`SharedInputQueue` は `Arc<Mutex<VecDeque<VideoFrame>>>` (Condvar 抜き) に縮小

### 本ブランチと PR #318 の扱い

本ブランチのコード変更 3 コミット (`7c137068` / `705d11f0` / `7af05076`) は develop に取り込まない。 PR #318 は本 close 追記のコミット (追記 + `git mv issues/0080-*.md issues/closed/`) を push した後に **merge せず close する**。

### 引き継ぐ分析資産

本 issue の以下の分析は 0085 の polish で参考にする:

- §現状 §shiguredo_nvcodec 2026.2.0 の実測 (crate 内 pending キュー FIFO、`n_encoder_buffer` 上限、Drop 時 drain 保証)
- §設計方針 §bp 機構の選定 (β 案 = shiguredo_nvcodec の `max_frames_in_flight` / γ 案 = 上位 bounded channel + `blocking_send` の棄却理由)
- §設計方針 §VideoEncoder wrapper 側への波及検証 (drop 順制御、メトリクスペアリング、複数 encoder ケース、RPC keyframe 応答性)
- §完了条件 §テスト の観点 (i)-(v) (in-flight bp の単体テスト共通観点)

## CHANGES.md について

NVENC の非同期パイプライン並列性回復による性能改善なので、 実機計測で有意な性能改善が確認された場合は `[UPDATE]` で記載する。 実装 PR 開設時点で `## develop` セクションに以下の草案を追加し、 実機計測完了後に X の数値を埋める。 未達の場合は §完了条件 §未達時の close 経路 に従う (草案の破棄含む):

```
- [UPDATE] nvcodec エンコーダーの非同期パイプライン並列性を回復させて 1080p30 の合成 wall-clock 時間を X% 短縮する
  - NvcodecEncoder::encode() の flush() 強制同期化を撤廃し、 内部キュー上限 Condvar によるバックプレッシャ機構に置き換える
  - Nvcodec が使えない環境 (macOS / CUDA なし Linux) では効果ゼロ
  - @sile
```

**本 issue 完了後の後続 issue 予告 (未起票)**: bp 効果観測用メトリクス (`Pacer` の `wait_timeout` 発火回数と wait 平均滞留時間) を stats 経由で追加すると、 実運用環境で N のチューニング根拠が p99 latency 外の観測手段を持てる。 本 issue 範囲外だが Decision Owner が必要と判断したら別 refactor issue として起票する。

## 関連

- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`、 2026-06-26 決定): 採用案 C の親 issue。 本 issue は §3 中核動機 (NVENC 並列性回復) を直接達成する
- closed/0067 (`feature/refactor-add-async-video-encoder`、 2026-07-08 merge、 commit `7b5f2740`): 直前依存。 Sender 化 + `error_slot` 廃止で本 issue の下地が整った。 unbounded 採用根拠が flush() 撤廃時に反転する構造は本 issue §目的 で明示
- closed/0079 (`feature/refactor-migrate-video-encoder-users-to-async`、 2026-07-08 merge、 commit `0943e9d6`): encoder 使用側移行。 本 issue の型面には影響なし
- closed/0083 (`feature/refactor-remove-sync-video-encoder-and-rename`、 2026-07-09 merge、 commit `66663c37`): wrap 削除 + `AsyncVideoEncoder` → `VideoEncoder` rename。 本 issue のシンボル名基準
- closed/0084 (`feature/refactor-remove-unused-next-encoded-frame`、 2026-07-10 merge、 commit `793abdcf`): 未使用 API 削除。 本 issue のシンボル状態基準
