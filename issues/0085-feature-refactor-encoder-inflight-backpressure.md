# NvcodecEncoder の flush() 撤廃と VideoEncoder レイヤーでの in-flight バックプレッシャ導入

- Priority: Medium
- Created: 2026-07-21
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-encoder-inflight-backpressure
- Reporter: @sile
- Decision Owner: @sile

## 目的

`NvcodecEncoder::encode()` 内の `self.inner.flush()?;` が worker 完了を毎フレーム待つため NVENC 非同期パイプライン並列性を殺している。 本 issue は `flush()` を撤廃し、代わりに `VideoEncoder::run` の `tokio::select!` に in-flight カウンタベースのバックプレッシャを追加することで、NVENC 並列性を回復させる。

closed/0080 の (α) 案 (`Pacer<T>` + `Condvar` の nvcodec レイヤーセルフペーシング) は tokio worker thread を block する設計欠陥により不採用となった (closed/0080 §解決方法 参照)。 本 issue はその後継として `VideoEncoder` レイヤーでの async task 内 usize カウンタによる bp を採用する。

## 優先度根拠

Medium。 closed/0080 と同じ NVENC 並列性回復を目的とし、実装コストは 0080 の (α) 案より小さい。

- closed/0080 §優先度根拠 の Medium 判定理由 (NVENC 並列性回復の中核) がそのまま適用される
- 実装 LOC 見積もり: ~50 LOC (0080 の ~200 LOC から大幅減)
- 依存: closed/0080 の分析資産 (shiguredo_nvcodec 2026.2.0 実測、β / γ 案棄却理由、VideoEncoder wrapper 波及検証、テスト観点 (i)-(v)) をそのまま引き継ぐ

## 現状

closed/0080 の実装コミットは develop に取り込まれず (PR #318 は un-merged close)、`origin/develop` 時点の状態から作業する。

- `src/encoder/nvcodec.rs::encode` の末尾に `self.inner.flush()?;` が残っている
- `src/encoder/pacer.rs` は存在しない
- `src/encoder.rs::VideoEncoder::run` の `Message::Syn(_)` arm は `{}` で暗黙 drop
- `src/encoder.rs::VideoEncoder::poll_output` は `try_recv` ベース

行番号や実測値は polish 時点で再確認する。

## 設計方針

### bp 機構の位置付け

closed/0080 の (α) 案は「nvcodec レイヤーで bp」だったが、本 issue は「`VideoEncoder` レイヤーで bp」に置き換える。

- `NvcodecEncoder` は同期 API のまま、`self.inner.flush()?;` を撤廃するだけ
- `VideoEncoder::run` に `in_flight: usize` + `IN_FLIGHT_LIMIT` を追加し、`tokio::select!` の入力腕に `if in_flight < IN_FLIGHT_LIMIT` guard を付ける
- LIMIT 到達時は `input_rx.recv()` を呼ばず、上流の Syn/Ack 経路で mixer 側の自主ペーシングが自然に停止する
- `Message::Syn(_)` は encoder レイヤーで即 drop する (forward しない、後述の「不採用案」参照)

### 実装ディテール (骨組み)

数値と細部は polish で確定する。

```rust
// VideoEncoder::run のイメージ (骨組み)
const IN_FLIGHT_LIMIT: usize = 3;  // n_encoder_buffer - 1 = 3 (closed/0080 の分析より引き継ぎ)
let mut in_flight: usize = 0;

loop {
    tokio::select! {
        message = input_rx.recv(), if in_flight < IN_FLIGHT_LIMIT => {
            match message {
                Message::Media(sample) => {
                    self.handle_input_sample(Some(sample))?;
                    in_flight += 1;
                }
                Message::Eos => self.handle_input_sample(None)?,
                Message::Syn(_) => {}  // encoder レイヤーで drop
            }
        }
        result = self.rx.recv() => {
            let frame = result.expect("sink dropped before rx")?;
            output_tx.send_media(MediaFrame::video(frame));
            in_flight -= 1;
        }
        // 既存 RPC 腕はそのまま
    }
}
```

### NvcodecEncoder 側の変更

- `encode()` の `self.inner.flush()?;` を撤廃
- `input_queue` は metadata FIFO (`Arc<Mutex<VecDeque<VideoFrame>>>`) として維持 (Condvar / Pacer なし)
- callback の Err 分岐にも `pop_front` を実装 (順序保証。 closed/0080 の contract を継承)
- `finish()` の `self.inner.flush()?;` は EOS 保証のため維持

### 不採用案

#### 案 β: Syn を writer に forward して end-to-end bp

encoder レイヤーで drop せず writer まで forward する end-to-end 方式は理論的には理想的 (writer 遅延も上流に伝わる) だが、正しく実装するには「Syn 受信時点で in-flight の全 Media が emit されて writer に転送されるまで Syn を保持」する必要がある (Syn だけ先に届くと writer の recv 順が壊れる)。 実装ロジック:

- Syn 受信時に `input_rx.recv()` を停止
- `rx.recv().await` ループで `in_flight = 0` まで drain
- その後 Syn を forward
- 全期間で Media/Syn の順序を writer 側で保証

実装複雑度が現時点の bp 要件に対して過剰。 まずは encoder レイヤーで drop する簡易方式で対応し、writer 遅延時の bp が要件になった場合に別 issue で再検討する。

#### 案 γ: writer 側の実装変更 (write 完了時に Syn を drop)

writer が Syn を write 完了時に drop する形にすると、writer の内部バッファリング効果が消えて Syn 保持期間が過剰になる。 writer は現状の即 drop (受信時 drop) で問題ない (writer の recv 頻度で pace が伝わる)。

### writer 遅延時の bp について (現状の限界)

案 X (encoder レイヤー drop) では writer 遅延時の bp は上流に伝わらない。 writer が MP4 書き込みで詰まった場合、`encoder → writer` の unbounded channel (`MessageReceiver` は `tokio::sync::mpsc::UnboundedReceiver`) にフレームが溜まる可能性がある。 これは既存の hisui でも取れていない挙動で、本 issue のスコープ外。 writer 遅延が実観測された場合、上記案 β または writer 側 self-pacing を別 issue で検討する。

### decoder 側の bp について

現状 nvcodec の decoder は bp なしで成立している。 理由:

- `NvcodecDecoder::decode()` は投入直後 return し、`self.inner.flush()?` は `finish()` (EOS 時) でのみ呼ばれる
- 順序保証は shiguredo_nvcodec 側の `pending_user_data` FIFO で担保
- 上流 (mp4 / webm / recording reader) が `send_syn` / `ack.await` で自主ペーシングしているため、decoder への投入も自然に pace される

本 issue の変更で encoder 側に in-flight bp を入れても decoder 側の挙動は不変。 将来 decoder 側で `"buffer full"` 相当のエラーが観測された場合は、同型 (VideoDecoder レイヤーで select! guard by in_flight) を別 issue で検討する。 現時点では pending issue も切らない (observation-driven に判断する)。

## 完了条件

### コード変更

- `src/encoder/nvcodec.rs::encode` から `self.inner.flush()?;` が撤廃されている
- `src/encoder/nvcodec.rs::finish` の `self.inner.flush()?;` は維持されている
- callback の Err 分岐で `pop_front` (`input_queue` の順序保証) が実装されている
- `src/encoder.rs::VideoEncoder::run` の `tokio::select!` に `in_flight: usize` カウンタと `if in_flight < IN_FLIGHT_LIMIT` guard が追加されている
- `Message::Syn(_)` は encoder レイヤーで drop (現状の `{}` 相当を維持)
- `poll_output` を「`try_recv` 版 (integration test 用)」と「`rx.recv().await` 版 (run 内 select! 用)」の両方に対応させる
- `IN_FLIGHT_LIMIT` の初期値と選定基準は polish 時点で確定 (closed/0080 の 3 = `n_encoder_buffer - 1` を叩き台にする)

### grep 検証

polish で確定。

### テスト

- `VideoEncoder::run` の in-flight bp を検証する integration test を追加 (モック / スタブは不使用、実 encoder で確認)
- テスト観点は closed/0080 §完了条件 §テスト の (i)-(v) を参考にする

### 実機計測

polish で確定 (closed/0080 §実機計測 の基準をベースに)。

### cargo

polish で確定 (closed/0080 §完了条件 §cargo 準拠)。

## 解決方法

polish 完了後に追記する。

## CHANGES.md について

polish で確定。 closed/0080 §CHANGES.md について の草案 (nvcodec 非同期パイプライン並列性回復) をベースに更新する。

## 関連

- issues/0086 (`feature/add-realtime-video-mixer-skip-on-encoder-backpressure`): 本 issue の後続。 realtime mixer 側で encoder 詰まり時のフレームスキップを追加する。 本 issue の in-flight bp が入って初めて mixer が「詰まり」を認識できる依存関係を持つ
- closed/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`、 2026-07-21 closed、 PR #318 は un-merged close): 直接の前身。 (α) 案が tokio worker block 問題で不採用となり本 issue に方針変更した経緯を持つ。 分析資産 (β / γ 案棄却、shiguredo_nvcodec 実測、VideoEncoder wrapper 波及検証、テスト観点) を全面的に継承する
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`、 2026-06-26 決定): 採用案 C の親 issue。 本 issue は §3 中核動機 (NVENC 並列性回復) を直接達成する
- closed/0067 (`feature/refactor-add-async-video-encoder`、 2026-07-08 merge、 commit `7b5f2740`): Sender 化 + `error_slot` 廃止で本 issue の下地が整った
- closed/0079 (`feature/refactor-migrate-video-encoder-users-to-async`、 2026-07-08 merge、 commit `0943e9d6`): encoder 使用側移行
- closed/0083 (`feature/refactor-remove-sync-video-encoder-and-rename`、 2026-07-09 merge、 commit `66663c37`): wrap 削除 + rename
- closed/0084 (`feature/refactor-remove-unused-next-encoded-frame`、 2026-07-10 merge、 commit `793abdcf`): 未使用 API 削除
