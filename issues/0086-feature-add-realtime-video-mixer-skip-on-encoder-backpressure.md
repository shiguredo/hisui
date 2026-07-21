# リアルタイム video mixer に encoder 詰まり時のフレームスキップを追加する

- Priority: Medium
- Created: 2026-07-21
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/add-realtime-video-mixer-skip-on-encoder-backpressure
- Reporter: @sile
- Decision Owner: @sile

## 目的

リアルタイム用途 (WebRTC / obsws 経路) で、下流 encoder の in-flight バックプレッシャによって `src/mixer/video.rs::VideoRealtimeMixerRunner` の `output_tx.send_syn()` の `Ack.await` (`src/mixer/video.rs:424-427`) が長時間 block されると、mixer 側の合成 pace がずれ、映像がリアルタイム性を失う。

本 issue は realtime mixer に「Ack が一定時間内に返らなければ現在のフレームを skip して次の合成タイミングに進む」処理を追加し、encoder 詰まり時にリアルタイム性を優先する挙動を導入する。

`src/sora/recording_video_mixer.rs` の recording (compose) mixer は品質優先 (全フレーム保持) が要件のため、本 issue のスコープ外 (現状の block-and-wait を維持)。

## 優先度根拠

Medium。 リアルタイム経路の品質確保の中核だが、依存先 (issues/0085) 完了後に着手する。

- 依存: issues/0085 (encoder に in-flight bp が入って初めて mixer が「詰まり」を認識できる)
- 0085 が入るまでは既存挙動 (encoder が block しない、mixer は全速送信可) のため本 issue の症状は顕在化しない
- 0085 完了後、実運用でリアルタイム経路の遅延が観測される前に予防的に入れておくのが望ましい

## 現状

`src/mixer/video.rs::VideoRealtimeMixerRunner` は以下のパターンで自主ペーシングする (`src/mixer/video.rs:126, :424-427`):

```rust
// 初期化時
let ack = Some(output_tx.send_syn());

// 合成ループ内
if let Some(waiting_ack) = self.ack.take() {
    waiting_ack.await;   // ← ここが encoder 詰まり時に長時間 block する
}
self.ack = Some(self.output_tx.send_syn());
```

0085 導入前の現状 (encoder が Syn を即 drop) では `Ack.await` はほぼ即復帰するため、この block は問題化していない。 0085 導入後は encoder の in-flight LIMIT 到達時に `input_rx.recv()` が止まり、Syn の Ack 復帰が遅れる → mixer の `Ack.await` が伸びる。

## 設計方針

### 骨組み

`Ack.await` を `tokio::time::timeout` で囲み、timeout 発火時はそのフレームを skip して次に進む。

```rust
// イメージ (骨組み、polish で確定)
const SKIP_TIMEOUT: Duration = Duration::from_millis(16);  // 60fps の 1 フレーム間隔を叩き台

if let Some(waiting_ack) = self.ack.take() {
    match tokio::time::timeout(SKIP_TIMEOUT, waiting_ack).await {
        Ok(()) => {}  // 通常経路
        Err(_) => {
            // encoder が詰まっている: このフレーム分は skip して次の合成タイミングに進む
            self.stats.total_skipped_video_frame_count.inc();
            continue;  // or 現在のフレーム生成を skip
        }
    }
}
self.ack = Some(self.output_tx.send_syn());
```

### 検討事項 (polish 時点で確定)

- **SKIP_TIMEOUT の適正値**: 1 フレーム間隔 (fps 依存) をベースに、encoder の callback レイテンシ実測値を加味
- **Skip 単位**: 「1 フレーム skip して次を送る」か「Ack 復帰まで複数フレーム skip し続けるか」の設計
- **stats**: `total_skipped_video_frame_count` の追加。 stats 名は既存の命名 (`total_output_video_frame_count` 等) に揃える
- **timeout 発火時の Syn の扱い**: 保持したまま次を送るのか、drop して新規 send_syn するのかで意味論が変わる (drop すると mixer 側の pending Ack が失われる → 累積計算に影響)
- **realtime 経路と recording 経路の分岐**: 現状 `VideoRealtimeMixerRunner` は realtime 専用の型なので分岐不要。 ただし共通化されている部分があれば整理する

## 完了条件

polish で確定。

## 解決方法

polish 完了後に追記する。

## CHANGES.md について

polish で確定。 リアルタイム経路の挙動変更 (`[UPDATE]` or `[ADD]`) として草案を追加する。

## 関連

- issues/0085 (`feature/refactor-encoder-inflight-backpressure`): 依存先。 本 issue は 0085 完了後に着手する
- closed/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`、 2026-07-21 closed): 0085 の前身。 本 issue の背景理解に参照
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`、 2026-06-26 決定): 大枠の親 issue
