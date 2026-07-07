# NvcodecEncoder の flush() 強制同期化を撤廃してバックプレッシャ機構を導入する

- Priority: Medium
- Created: 2026-07-07
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-nvcodec-encoder-flush-and-backpressure
- Polished: {YYYY-MM-DD}
- Reporter: @sile
- Decision Owner: @sile

## 目的

`NvcodecEncoder::encode()` (`src/encoder/nvcodec.rs:248-257`) が worker 完了を待つため `flush()` を毎フレーム強制呼出しており、 NVENC が本来持つ非同期パイプライン並列性を全く活かせていない。 本 issue はこの `flush()` 強制同期化を撤廃し、 内部キュー上限ベースのバックプレッシャ機構 (bp 機構) を導入することで NVENC 非同期パイプライン並列性を回復させる。

closed issue 0057 §3 採用案 C の中核動機 (NVENC 並列性回復) の直接的な達成 issue。 open issue 0067 (`AsyncVideoEncoder` + Sender 化 + `error_slot` 廃止) 完了で下地 (Sender 経由の Err 伝搬 + メトリクスペアリング + callback から `sink.emit_ok` への直接 push) が整っており、 その上で bp 機構を差し込む形になる。

Sender 化とは本質的に別問題 (bp 機構は 0067 で意図的にスコープ外に切り出された) で、 refactor カテゴリよりも性能改善 (perf) が主目的だが、 Branch prefix は shiguredo-git 規約に従い `feature/refactor-` を使用する。

## 優先度根拠

Medium。

- closed issue 0057 で採用案 C が確定した時点で Medium 維持の中核理由 (NVENC 並列性回復) として位置付けられていた
- 依存先: 0067 (`AsyncVideoEncoder` + Sender 化) の PR merge 後に着手 (0067 の callback → `sink.emit_ok` 経路が bp 機構の受け皿になる)
- 0079 (使用側移行) の完了は不要。 wrap `VideoEncoder` の存否に関わらず、 inner `NvcodecEncoder` の内部構造の変更で完結する
- Nvcodec が使えない環境 (macOS / CUDA なし Linux) では効果ゼロだが、 実運用では GPU 環境が主流のため影響は大きい

## 現状

open issue 0067 完了後の `src/encoder/nvcodec.rs` を基準とする。

### `NvcodecEncoder::encode()` の flush() 強制同期化 (`src/encoder/nvcodec.rs:248-257`)

```rust
self.inner.encode(&nv12_data, &encode_options, ())?;
self.input_queue.push_back(video_frame.to_stripped());
// shiguredo_nvcodec のエンコーダーは内部の worker スレッドで非同期にエンコードし、
// encode() は即時 return する。 上位パイプラインは同期 pull 型で、 上位側でペース制御
// しないと内部キューが溢れて encode() が "encoder buffer is full" で失敗するため、
// 投入直後に flush() で 1 フレーム分の完了を待って同期動作させる。
self.inner.flush()?;
self.handle_encoded_frames()?;
```

このコメントの通り、 現状は上位のペース制御が存在しないため `flush()` で 1 フレーム分の完了を強制的に待って同期動作させている。 これにより NVENC 内部の非同期 worker パイプラインが 1 フレームごとに同期障壁で止まり、 GPU 側の投入並列度が事実上 1 になっている。

### `NvcodecEncoder::finish()` の flush() (`src/encoder/nvcodec.rs:263-268`)

```rust
pub fn finish(&mut self) -> crate::Result<()> {
    // flush で in-flight 完了を待ち合わせる
    self.inner.flush()?;
    self.handle_encoded_frames()?;
    Ok(())
}
```

`finish()` 内の `flush()` は EOS 経路で in-flight の残物を確実に flush するために必須。 本 issue では **維持する** (bp 機構と直交する EOS 保証)。

### 0067 完了後の callback → sink.emit_ok 経路

0067 で `NvcodecEncoder::build_handler` は callback スレッドで直接 `sink.emit_ok(VideoFrame)` に流す形に変更済み。 `encoded_queue` (中継バッファ) と `error_slot` (エラー退避スロット) は廃止済み。 上位 `AsyncVideoEncoder` は unbounded `tokio::sync::mpsc::UnboundedReceiver` で受ける。

## 設計方針

### bp 機構の選定

closed/0057 §3 §2 で議論された案から本 issue で確定させる。 候補:

- (α) **内部キュー上限ベースのセルフペーシング**: `encode()` 内で `input_queue.len()` を監視し、 上限到達時は `condvar` / `park_timeout` / 短時間 spin で待ち合わせる。 影響範囲が `src/encoder/nvcodec.rs` に閉じる
- (β) **NVENC 側 `max_frames_in_flight` パラメータ**: `shiguredo_nvcodec::EncoderConfig` の該当パラメータ (存在する場合) で GPU 側の同時投入数を制限する。 hisui 側からは薄いラッパー
- (γ) **上位 `AsyncVideoEncoder` の bounded channel**: 0067 で unbounded を採用したが、 encoder 側だけ bounded (`tokio::sync::mpsc::channel(N)`) に切り替えて `tx.blocking_send` で callback を block させる。 ただし CUDA worker を tokio bounded で block させると deadlock 懸念があるため慎重に検討

判断は実装段階で shiguredo_nvcodec の API 実測 + 実機計測を経て確定する。 暫定推奨は (α) (影響範囲が閉じる)。

### `encode()` からの flush() 撤廃

`src/encoder/nvcodec.rs:254` の `self.inner.flush()?;` を撤廃し、 encode 後は投入直後に return する (worker 完了を待たない)。 callback → `sink.emit_ok` の経路は 0067 で確立済みなので、 完了フレームは非同期に上位 `AsyncVideoEncoder.rx` に届く。

`self.handle_encoded_frames()?;` (`:255`) は 0067 で callback 経路に統合されているため、 本 issue で追加削除する必要なし。

### `finish()` の flush() 維持

`finish()` 内の `flush()` (`:265`) は EOS 時の残物完全 flush のために必須で維持する。 本 issue の変更対象外。

### shiguredo-rust 規約整合

- モック / スタブ不使用
- 新規 trait 追加なし
- `#[non_exhaustive]` 不使用

## 完了条件

- `src/encoder/nvcodec.rs:254` の `self.inner.flush()?;` が撤廃されている (encode 内)
- `src/encoder/nvcodec.rs:265` の `self.inner.flush()?;` は維持されている (finish 内、 EOS 保証)
- 選定した bp 機構 (α / β / γ のいずれか) が実装されている
- 実機計測で以下を満たす (closed/0057 §3 §2 の暫定基準):
  - 1080p30 / 60 秒の compose を H.264 で走らせ、 現状 (flush あり) と本 issue (flush 撤廃 + bp 機構) の wall-clock 時間を比較して **wall-clock 短縮 15% 以上**
  - 同計測で **p99 frame latency 改善 5ms 以上**
  - "encoder buffer is full" エラーが発生しないこと
- 未達の場合は数値とともに残懸念として本 issue に追記し、 Decision Owner が判断 (別途 bp 機構の再選定 or 本 issue の priority 降格)
- 計測条件を本 issue に記録:
  - GPU 型番、 NVIDIA driver / CUDA バージョン、 OS
  - hisui ビルド feature (`--features nvcodec`)
  - 計測素材の出所と保存先
  - 各案 5 run + ウォームアップ 1 run、 平均 ± 標準偏差
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

1. `shiguredo_nvcodec` の bp 機構 API を実測 (`max_frames_in_flight` パラメータ有無、 その他 GPU 側キュー上限指定手段)
2. bp 機構の選定 (α / β / γ) を実機計測で確定
3. 選定した bp 機構を実装
4. `src/encoder/nvcodec.rs:254` の `self.inner.flush()?;` を撤廃
5. 完了条件の実機計測 (H.264 / 1080p30 / 60 秒 compose) を実施
6. 完了条件の cargo コマンドを default + `--no-default-features` の両方で通す

## CHANGES.md について

NVENC の非同期パイプライン並列性回復による性能改善なので、 実機計測で有意な性能改善が確認された場合は `[UPDATE]` で「nvcodec エンコーダーの非同期パイプライン並列性を回復させて wall-clock 時間を X% 短縮した」旨を記載する。 Decision Owner (@sile) が実装段階で判断する。

## 関連

- open/0067 (`feature/refactor-add-async-video-encoder`): 依存先。 本 issue は 0067 の PR merge 後に着手する (0067 で callback → `sink.emit_ok` 経路が確立され、 `error_slot` / `encoded_queue` が廃止されるため、 本 issue の bp 機構が差し込める下地が整う)
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の親 issue。 本 issue は §3 中核動機 (NVENC 並列性回復) を直接達成する。 §3 §2 で議論された案 D-1 / D-2 / D-3 と本 issue の (α) / (β) / (γ) が概ね対応
- open/0079 (`feature/refactor-migrate-video-encoder-users-to-async`): encoder 系の使用側移行 refactor issue。 本 issue と独立に着手可能 (どちらが先でも 0067 完了さえしていればよい)
