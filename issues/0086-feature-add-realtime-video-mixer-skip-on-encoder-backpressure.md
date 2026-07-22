# リアルタイム video mixer に encoder 詰まり時のフレームスキップを追加する

- Priority: Medium
- Created: 2026-07-21
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/add-realtime-video-mixer-skip-on-encoder-backpressure
- Polished: 2026-07-22
- Reporter: @sile
- Decision Owner: @sile

## 目的

リアルタイム用途 (WebRTC / obsws 経路) で、 下流 encoder の in-flight バックプレッシャーによって `src/mixer/video.rs::VideoRealtimeMixerRunner::handle_output_tick` の `noacked_sent > MAX_NOACKED_COUNT` 分岐 (`src/mixer/video.rs:423-429`) 内の `waiting_ack.await` (`src/mixer/video.rs:425`) が長時間 block されると、 mixer の合成 tick pace が遅延し映像がリアルタイム性を失う。

本 issue は realtime mixer に「Ack が SKIP_TIMEOUT 内に返らなければ現在の tick を skip して次の合成 tick に進む」処理を追加し、 encoder 詰まり時にリアルタイム性を優先する挙動を導入する。

### 効果範囲 (0085 応急処置による限定)

本 issue の frame skip が実際に発火する条件は `Ack.await` が SKIP_TIMEOUT を超えて block することだが、 現行 develop の `src/encoder.rs::VideoEncoderInner::requires_backpressure` (`src/encoder.rs:1049-1058`) は応急処置として nvcodec 経路のみ true を返す:

- **realtime + nvcodec 経路**: encoder が `IN_FLIGHT_LIMIT = 3` (`src/encoder.rs:772`) に到達すると `input_rx.recv()` が停止し、 Syn が queue に留まって `Ack.await` が block する。 本 issue の skip が発火する
- **realtime + 非 nvcodec 経路 (libvpx / openh264 / svt_av1 / video_toolbox)**: bp guard が実質無効なので `input_rx.recv()` が止まらず Syn は即 consume される。 `Ack.await` は即返り、 本 issue の skip は事実上 no-op

非 nvcodec 経路への波及は issues/0087 で `requires_backpressure` 応急処置を解消したあと成立する。 本 issue 単体では nvcodec 環境の効果検証のみ可能な点を注意する (issues/0087 完了後に非 nvcodec でも効果検証する)。

## 優先度根拠

Medium。 リアルタイム経路の品質確保の中核だが、 依存先 (issues/0085) 完了直後で予防的に投入する。

- 依存先 issues/0085 は develop に merge 済み。 encoder に in-flight バックプレッシャーが入って初めて mixer が「詰まり」を認識できる状況が成立した
- 実運用でリアルタイム経路の遅延が観測される前に予防的に入れておく

## 現状

`src/mixer/video.rs::VideoRealtimeMixerRunner` は 101 tick 周期で 1 回だけ `send_syn()` + `Ack.await` で自主ペーシングする (`src/mixer/video.rs:126, 423-429, 447`):

```rust
const MAX_NOACKED_COUNT: u64 = 100;  // src/mixer/video.rs:17

// VideoRealtimeMixer::run 初期化時 (:126)
let ack = Some(output_tx.send_syn());

// handle_output_tick 内 (:423-429)
if self.noacked_sent > MAX_NOACKED_COUNT {
    if let Some(waiting_ack) = self.ack.take() {
        waiting_ack.await;   // ← encoder 詰まり時に長時間 block する
    }
    self.ack = Some(self.output_tx.send_syn());
    self.noacked_sent = 0;
}

// (通常経路: compose_frame → send_video → add_output_video_frame_count → noacked_sent += 1)
```

`MAX_NOACKED_COUNT = 100` の pace 特性:

- 分岐条件は strict greater than (`> 100`)。 判定は tick 先頭、 `noacked_sent += 1` は tick 末尾で行われる。 初期状態 `noacked_sent = 0` から始めると tick 101 で `100 > 100` は false、 tick 102 で `101 > 100` が true となり分岐入りする。 つまり **102 tick 目** で最初の `Ack.await` に到達する
- 2 回目以降は分岐入りごとに `noacked_sent = 0` にリセットされるため、 前回分岐入りから **101 tick 周期** で次の `Ack.await` に到達する
- 発火間隔は 60fps で ~1.68 秒、 30fps で ~3.37 秒、 25fps で ~4.04 秒に 1 回
- 発火機会の間は encoder への queue (subscribe_track が返す unbounded_channel、 `src/media_pipeline.rs:1098`) に無条件でフレームを流し込む
- 本 issue の skip は「101 tick 周期の 1 回の待ちを SKIP_TIMEOUT で打ち切る」動作となる。 毎 tick で timeout する設計ではない

hisui default frame_rate は `FrameRate::FPS_30` (`src/mixer/video.rs:60` の `frame_rate.unwrap_or(...)`)。 25 / 30 / 60 で切り替わり、 obsws / JSON 経由で可変。 SKIP_TIMEOUT は frame_rate 依存で動的算出する (§設計方針 参照)。

### `crate::Ack` の意味論

`crate::Ack` は `Receiver<()>` の thin wrapper (`src/media_pipeline.rs:1166`)。 Ack が復帰する契機は「対応する Syn の `Sender<()>` clone が全て drop されたこと」(`src/media_pipeline.rs:1168-1177` の `poll_recv` が `Ready(None)` を返す)。 `tokio::time::timeout` で `Ack.await` を打ち切って Receiver を drop しても、 queue に残った Syn は各 subscriber の `Message::Syn(_)` arm で drop されて自然に消化される (encoder 側は `src/encoder.rs:846` で `Message::Syn(_) => {}` で即 drop)。

pipeline shutdown が SKIP_TIMEOUT 進行中に発生した場合は、 subscriber rx drop で Sender clone が drop され `Ack.await` は Ok で早期復帰する。 その後の `send_syn()` は失敗した subscriber を retain_mut で除去し、 次の `send_video` 失敗経路で mixer が自然終了する。

## 設計方針

### 骨組み

`handle_output_tick` (`src/mixer/video.rs:412`) の `noacked_sent > MAX_NOACKED_COUNT` 分岐に `tokio::time::timeout` を挟み、 timeout 発火時はその tick 分の合成・送信を発行せず早期 return する。 `handle_output_tick` は `async fn` 単体で内部に loop を持たないため、 外側 loop (`VideoRealtimeMixerRunner::run` の `:385-400`) を次 tick に進める方法は `return Ok(true)` で `handle_output_tick` を早期終了させる形になる (continue は使えない)。

以下は match block + その直後の共通処理までを含めた完全形の骨組み。 通常経路の Ok(()) arm は空で、 match 直後の共通処理 (`self.ack = Some(send_syn())`; `self.noacked_sent = 0`) に fall-through することで新規 syn を 1 回だけ発行する (Ok arm 内に `send_syn` を書くと二重発行になるので書かない):

```rust
// src/mixer/video.rs::VideoRealtimeMixerRunner::handle_output_tick 内の該当分岐
if self.noacked_sent > MAX_NOACKED_COUNT {
    if let Some(waiting_ack) = self.ack.take() {
        // frame_rate は `update_config` が変更要求を Err で return するため immutable。
        // 毎回再計算しても cost は無視できるので cache しない。
        let skip_timeout = skip_timeout_for(self.frame_rate);
        match tokio::time::timeout(skip_timeout, waiting_ack).await {
            Ok(()) => {
                // 通常経路: Ack 復帰。 新規 send_syn は match 直後の共通処理で発行する。
            }
            Err(_) => {
                // encoder 詰まり: この tick は compose / send を発行せず次 tick へ。
                tracing::warn!("mixer skipped output tick due to encoder backpressure timeout");
                self.stats.total_encoder_backpressure_skipped_video_frame_count.inc();
                self.noacked_sent = 0;
                return Ok(true);
            }
        }
    }
    self.ack = Some(self.output_tx.send_syn());
    self.noacked_sent = 0;
}

// (通常経路: 以下、既存の compose_frame → send_video → add_output_video_frame_count → noacked_sent += 1)
```

`tracing::warn!` の import 追加は不要 (既存の `tracing::error!` prefix 呼び出しが `src/mixer/video.rs:496` に存在し、 同形式で書ける)。 skip 発火時 warn は encoder 詰まりが継続する間 101 tick 周期で連続発火し得るが、 これは詰まり継続を運用側に可視化する signal なので rate limit は入れない (spam ではなく状態表示)。

skip 経路の `.inc()` 直接呼び出しは通常経路の `add_output_video_frame_count()` accessor 呼び出しと隣接して並ぶが、 これは既存 `_skipped_` 系との命名・呼び出し規約対称を優先した意図的な非対称 (§stats 命名と inc 方式 参照)。

### skip_timeout_for

`skip_timeout_for` は `frames_to_timestamp` (`src/mixer/video.rs:1266`) の隣に置く free fn として実装する (const fn 化は `Duration` の `Div<u32>` が stable const でないため不可)。 内部で `frames_to_timestamp(frame_rate, 1)` を呼び 1 tick 間隔を返す:

```rust
/// SKIP_TIMEOUT の実質定義。 1 tick 分の `Duration` を返す。
/// 見直しの方針は §スコープ外 参照。
fn skip_timeout_for(frame_rate: FrameRate) -> Duration {
    frames_to_timestamp(frame_rate, 1)
}
```

`skip_timeout_for` が返す値 (発火間隔は §現状 の pace 特性参照):

- 25fps: 40.0ms
- 30fps: ~33.3ms
- 60fps: ~16.7ms

**false-positive skip リスクの注意**: 1 tick 間隔は tokio scheduler の tick 遅延 + subscriber の poll 応答遅延 + CPU 高負荷時の task switching で容易に超過し得るタイトな叩き台。 encoder が詰まっていないのに timeout する false-positive が現実的に発生する可能性がある。 効果検証時は false-positive rate を注視する。 定量的な判定基準は本 issue のスコープ外 (§スコープ外 参照)。

`FrameRate` の実用範囲 (`hisui` は 25 / 30 / 60 fps を主要想定) では極端値ガードは不要。 obsws / JSON 経由で予期しない極端値が入った場合でも `skip_timeout` の実質最小値になるだけで実害はない。

### skip 経路と通常経路の状態遷移

skip / 通常の 2 経路で mixer 状態変数と副作用が受ける影響を明示する。 skip 経路でも `advance()` (`src/mixer/video.rs:419-421`) と finishing フラグ判定 (`:413-416`) は通常経路と同じく実行される (skip は `noacked_sent > MAX_NOACKED_COUNT` 分岐入り後の一部だけを短絡させる)。

| 変数・副作用 | 通常経路 (Ack 復帰) | skip 経路 (timeout 発火) |
|---|---|---|
| `for state.advance(now)` (`:419-421`) | 実行する | 実行する |
| `self.ack` | `Some(send_syn())` を新規代入 | `take()` 済み `None` のまま |
| `self.noacked_sent` | 0 リセット、 以降 send 毎に inc する | 0 リセット |
| `self.output_frame_index` | inc する (timestamp を消費) | inc しない (この tick は compose・send を発行しない) |
| `self.stats.add_output_video_frame_count()` | 呼ぶ | 呼ばない |
| `self.stats.total_encoder_backpressure_skipped_video_frame_count.inc()` (新規) | 呼ばない | 呼ぶ |
| `compose_frame` の呼び出し | 呼ぶ (text_overlay 描画含む) | 呼ばない (text_overlay layer の `ensure_rendered` も skip tick では走らない。 次に通常経路が回った tick で dirty 時のみ再描画、 それ以外は cached_frame を再利用する) |

`output_frame_index` を inc しない設計理由: 状態管理の simple 化のため。 通常経路は `output_frame_index++` → compose → send の 3 副作用が同 tick 内で対になっており、 skip 経路は 3 副作用すべてを skip する形で対称性を保つ。 次 tick で `catch_up_output_frame_index` (`src/mixer/video.rs:1271-1279`) が wall-clock 経過分を追いつかせるため busy-loop 化はしない (inc しても `catch_up` が同じ 1 tick 分の gap を追うため実挙動は同じだが、 「送信しなかった tick は index も進めない」意味論を統一する)。

### skip 発火から次の send_syn 発行までのタイミング

skip 発火後の tick 進行を精緻に整理する。 skip 発火 tick を N tick と呼ぶ。 tick 頭で分岐判定、 tick 末尾で `noacked_sent += 1` する構造 (§現状 の擬似コード参照) を前提とする:

- N tick: skip 発火 → `noacked_sent = 0`, `self.ack = None` のまま early return (末尾 inc なし)
- N+1 〜 N+101 tick: 各 tick 頭の `noacked_sent` は 0 → 100 の間で `> 100` false。 `self.ack = None` のまま `compose_frame` → `send_video` → `noacked_sent += 1`。 各 tick 末尾で `noacked_sent = 1 → 101` に到達 (N+1 末尾で 1、 N+101 末尾で 101)。 合計 101 tick 分 send される
- N+102 tick: 頭で `noacked_sent = 101 > 100` true → 分岐入り → `self.ack.take()` は `None` なので `Ack.await` は実行されず match block は skip → 直後の共通処理で `self.ack = Some(self.output_tx.send_syn())` を発行、 `noacked_sent = 0` にリセット → その後 handle_output_tick 後半の compose+send が通常経路と同様に走り、 末尾 inc で `noacked_sent = 1`
- N+103 〜 N+202 tick: 各 tick で send し、 末尾 inc で 2 → 101 に到達 (100 tick 分の send)
- N+203 tick: 頭で `noacked_sent = 101 > 100` true → 分岐入り → `self.ack.take()` は N+102 で発行した Syn の `Some(...)` → `waiting_ack.await` に **到達**

したがって skip 発火から次の `Ack.await` 到達までは追加 203 tick 経過 (30fps で ~6.77 秒)。 この間 nvcodec 内の in-flight フレームが drain されなければ次の `Ack.await` で再度 timeout skip が発生する。 backlog 累積の副作用は §スコープ外 に整理する。

### stats 命名と inc 方式

新規カウンタ名は `total_encoder_backpressure_skipped_video_frame_count`。 命名軸:

- 既存の `total_video_encoder_backpressure_count` (`src/encoder.rs:517`) と対称 (原因は "encoder backpressure")
- 既存の `total_crop_skipped_draw_count` / `total_resize_skipped_draw_count` (`src/mixer/video.rs:337-338`) と対称 (原因 + `_skipped_` + 単位の 3 軸命名)
- 単位は `video_frame_count` (tick 全体を落とすため `draw_count` ではない)
- outcome メトリクスの位置付け: skip は「バックプレッシャーに反応した失敗経路」の計測

inc 方式は accessor ではなく直接 `.inc()` を呼ぶ (既存 `_skipped_` 系との対称)。 既存の `total_crop_skipped_draw_count` / `total_resize_skipped_draw_count` は `src/mixer/video.rs:1032, 1067` で `stats.total_crop_skipped_draw_count.inc()` / `stats.total_resize_skipped_draw_count.inc()` の形で直接呼び出されており、 accessor は持たない。 命名軸を `_skipped_` 系に合わせた以上、 inc 方式も対称に揃える。 `add_output_video_frame_count` 系 accessor パターンとの整合は取らない (frame_count 系と skipped 系で既存の慣習が分かれている事実に従う)。

struct フィールドの追加位置は `total_resize_skipped_draw_count` の直後 (末尾追加)。 `_skipped_` 系でまとめる。

## 完了条件

### コード変更

- `src/mixer/video.rs::VideoRealtimeMixerRunner::handle_output_tick` の `noacked_sent > MAX_NOACKED_COUNT` 分岐に `tokio::time::timeout` による skip 経路が追加されている
- free fn `fn skip_timeout_for(frame_rate: FrameRate) -> Duration` が `frames_to_timestamp` の隣 (`src/mixer/video.rs:1266` 付近) に追加され、 内部で `frames_to_timestamp(frame_rate, 1)` を呼んでいる
- skip 発火経路で §skip 経路と通常経路の状態遷移 の各セル通りに副作用が更新されている
- `VideoRealtimeMixerStats` (`src/mixer/video.rs:328-339`) に `total_encoder_backpressure_skipped_video_frame_count: crate::stats::StatsCounter` フィールドが `total_resize_skipped_draw_count` の直後に追加され、 `new` の struct literal と `.counter("total_encoder_backpressure_skipped_video_frame_count")` 登録が既存パターンに揃った形で追加されている
- skip 発火時に `tracing::warn!` で 1 行ログを出す (英語)
- 既存の通常経路 (`Ack.await` 即復帰時) の挙動は変更されていない
- `finishing = true` フラグ経路 (`src/mixer/video.rs:413-416`) の挙動は変更されていない

### grep 検証

- `rg 'tokio::time::timeout\(skip_timeout' src/mixer/video.rs` で **1 件** (handle_output_tick 内、 テスト側 hit と分離するため skip_timeout との共起で条件付ける。 テスト側では変数名 `skip_timeout` を使わない)
- `rg 'fn skip_timeout_for' src/mixer/video.rs` で **1 件** (関数定義)
- `rg 'total_encoder_backpressure_skipped_video_frame_count' src/mixer/video.rs` で **非テストコード内で 3 件** (struct フィールド定義・`new` 内の登録・`.inc()` 呼び出し)。 テスト内の assert は別途複数出現するため合計行数は 6〜8 件になる想定

### テスト

`#[cfg(test)] mod tests` (`src/mixer/video.rs` 末尾) 内に **各観点それぞれ別の `#[tokio::test]` 関数** として追加する。 モック / スタブ不使用 (CLAUDE.md 準拠) の下で以下を検証する。 hisui コードベースには `tokio::time::pause` の使用実績が 0 件のため、 本 issue のテストも実 wall-clock で書く。 テストごとに各 assert の意図と期待する状態遷移を日本語コメントで明示する (CLAUDE.md「テストはコメントを重視すること」に準拠)。

#### 共通の実装方針

- **frame_rate 選定**: すべてのテストで `FrameRate::FPS_25` を使う (SKIP_TIMEOUT = 40ms で判定マージンを最大化)。 60fps だと SKIP_TIMEOUT = 16.7ms で scheduler jitter に食われる
- **slow drain 実装**: `sink_processor.subscribe_track(output_track_id)` が返した `MessageReceiver` をテスト task 側で **`recv()` を呼ばずに保持**。 このレシーバーが drop されるまで Syn Sender clone は queue に留まり `Ack.await` が block する (§`crate::Ack` の意味論 参照)。 これは実 receiver の drain 遅延であり、 CLAUDE.md のモック / スタブ禁止規約には抵触しない
- **slow drain 解除**: テスト task 側で `MessageReceiver` を `drop(rx)` する形で解除する (subscriber cleanup が `retain_mut` (`src/media_pipeline.rs:1264`) 経由で走り、 次の `send_syn` 呼び出し時に Syn Sender clone が全 drop されて Ack が復帰する)。 `recv()` で drain する方式は選ばない (drop 方式のほうが cleanup タイミングが決定的)
- **stats counter 事前 capture**: `VideoRealtimeMixer::run(mixer_processor)` は `mixer_processor` を move で消費するため、 spawn 後にテスト task 側から `mixer_processor.stats()` を呼ぶことはできない。 spawn する前に `let mut mixer_stats = mixer_processor.stats(); let skip_counter = mixer_stats.counter("total_encoder_backpressure_skipped_video_frame_count"); let output_counter = mixer_stats.counter("total_output_video_frame_count");` の形で counter handle を事前 capture する。 mixer 側が spawn 後に register する counter は `Stats::counter` の shared_entries 仕様 (`src/stats.rs` の `get_or_insert_entry`) により同じ `Arc<AtomicU64>` 実体を指す。 テストからは `skip_counter.get()` / `output_counter.get()` で状態を観察する
- **CI flakiness 対策**: `#[tokio::test]` は既定で並列実行され、 CPU 高負荷時に SKIP_TIMEOUT = 40ms を task switching で越える false-positive が起きうる。 CI で flaky が観測された場合は該当テストに `#[serial_test::serial]` 相当の serial 化 (crate 追加が要る場合は別 issue で扱う) or `--test-threads=1` の CI 側指示 or SKIP_TIMEOUT を `frames_to_timestamp(frame_rate, 2)` に緩めた test 専用 helper への差し替えを検討する (優先順位は上から)。 これらの選択は実装時 flaky 実測後に判断する

#### 各テスト

- (a) **回帰**: 既存の `video_realtime_mixer_two_tracks_smoke` (`src/mixer/video.rs:1677-1815`) が変更なしで通ること。 このテストは 5 frame 生成のみで `noacked_sent > MAX_NOACKED_COUNT` に到達しないため skip 分岐に入らない
- (b) **skip 発火** (実行時間 ~5 秒 想定): slow drain 状態で mixer を起動し、 sender タスクからは既存 `video_realtime_mixer_two_tracks_smoke` (`src/mixer/video.rs:1677-1815`) の wall 10ms 間隔 send パターンを流用して十分な入力フレーム (例: 150 frame) を back-to-back に送る (mixer tick は入力ペースから独立に自走するため送信レートは skip 判定に影響しない)。 mixer_start から 5 秒 sleep したあと以下を assert:
  - `skip_counter.get() >= 1` (skip 経路に到達した)
- (c) **skip boundary** (実行時間 ~4.5 秒 想定): slow drain 状態で sender から十分な入力を送り、 mixer_start から `frames_to_timestamp(FPS_25, 101)` = 4.04 秒 相当の待ち (この時刻が mixer 側の最初の branch entry と同時刻。 `waiting_ack.await` に到達している状態) から SKIP_TIMEOUT の半分 (20ms) だけ待ち、 その直後に `drop(rx)` で slow drain を解除する。 続けて 0.5 秒 待って以下を assert:
  - `skip_counter.get() == 0` (Ack.await が復帰したので skip は発火していない)
- (d) **skip 後の復旧** (実行時間 ~10 秒 想定): (b) と同じ手順で 5 秒 sleep して skip 発火を作る。 5 秒 sleep 直後に `let baseline_output = output_counter.get();` で baseline を capture してから `drop(rx)` で slow drain を解除する。 続けて追加 5 秒 待って以下を assert:
  - `skip_counter.get() == 1` (追加 skip は発火せず 1 のまま推移。 25fps で N+203 tick 到達は skip 発火から 8.12 秒後 (203 tick × 40ms)、 追加 5 秒 経過時点では次の Ack.await 判定 tick に到達しないため 2 回目の skip は起こらない)
  - `output_counter.get() - baseline_output >= 100` (drop 後 5 秒間で少なくとも 100 tick 分の compose+send が走った。 25fps × 5 秒 = 125 tick から余裕をもった下限)
- (e) **text_overlay 併用**: `text_overlay_config = Some(...)` を持つ mixer 構成で (b) と同じ手順・同じ assert を行う (text_overlay 有無で skip 分岐挙動が変わらないことの回帰確認)。 `TextOverlayConfig` は `TextOverlayConfig::new(PathBuf::from("testdata/fonts"), "PublicSans-Regular.ttf".to_owned())` を流用する (`src/mixer/video/text_overlay/layer.rs::tests::make_layer` のパターン)

### CHANGES.md

- `## develop` セクションに `[UPDATE]` エントリが追加されている (§CHANGES.md について の草案通り)

### cargo

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。 `--features nvcodec` は CUDA 環境で Decision Owner が別途実施 (`requires_backpressure` の nvcodec 側効果検証を兼ねる)。

## 解決方法

polish 完了後に追記する。

## CHANGES.md について

タグは `[UPDATE]` を採用する (根拠: 既存 realtime mixer の block-and-wait 挙動を skip 挙動に置き換える形になり、 下流受信側のフレーム間隔特性が変わりうる後方互換維持の変更のため。 「機能追加」寄りとして `[ADD]` にする案もあるが、 変わっているのは既存挙動なので [UPDATE] が寄っている)。

実装 PR 開設時に以下の草案を追加する (issue 番号は shiguredo-issues 規約により本文に露出させない):

```
- [UPDATE] リアルタイム映像 mixer に下流エンコーダー詰まり時のフレームスキップを追加する
  - 下流エンコーダーが詰まって mixer の Ack 待ちがタイムアウトしたら、その tick の合成をスキップして次 tick へ進む
  - 現行の encoder in-flight バックプレッシャーの応急処置により、 効果が及ぶのは realtime + nvcodec 経路のみ (他エンコーダー経路は応急処置解消後に波及する)
  - @sile
```

## スコープ外

- **`MAX_NOACKED_COUNT = 100` の値見直し**: 100 tick batch という値は `src/mp4/reader.rs:24, 1487` と `src/webm/file_reader.rs:8` と `src/obsws/source/color_source.rs:7` と `src/obsws/source/png_file.rs:8` に同名 const で散在する共通 pace 定数 (`src/sora/recording_reader.rs:41, 279` は const 名ではなく literal `100` で書かれている)。 realtime 経路だけ値を下げる or 全 pace 箇所横断で変更するかの判断は本 issue のスコープ外。 別 issue で扱う。 本 issue の frame skip は現状の 100 tick batch 前提で成立する
- **`recording_video_mixer` の同型対応**: recording (compose) mixer には `send_syn` / Ack 機構がなく (`grep -c send_syn src/sora/recording_video_mixer.rs` が 0 件)、 品質優先で全フレームを保持する要件のため本 issue の対象外
- **`AudioRealtimeMixer` の同型対応**: `src/mixer/audio.rs` は `send_syn` / `Ack.await` を使わない設計 (`grep -c send_syn src/mixer/audio.rs` が 0 件)。 encoder Ack ブロック問題自体が発生しないため本 issue の枠組み自体が非適用。 `tokio::time::interval` + `MissedTickBehavior::Skip` (`src/mixer/audio.rs:516-517`) は tick timer catch-up 遅延の skip 機構であり本 issue の課題とは別種
- **`webm/file_reader.rs` / `mp4/reader.rs` / `sora/recording_reader.rs` の frame skip 導入**: reader 系は compose / vmaf 経路で使われ、 品質優先が要件のため対象外
- **encoder 入力 unbounded channel の bounded 化**: `src/media_pipeline.rs:1098` の `subscribe_track` が返す unbounded_channel により、 skip 連続発火時 (encoder が慢性的に catch up しないケース) は 2 回の skip 発火間 (N tick と N+203 tick の間) に送出される 202 tick 分の video + 1 syn がすべて encoder 入力 channel に積み上がり memory を圧迫する。 本 issue の skip は「101 tick 周期の long block 打ち切り」だけを目的とし、 backlog 累積の bounded 化・運用観測 (memory 圧迫、 GC 遅延、 system slowdown 等) は別 issue で扱う
- **SKIP_TIMEOUT 微調整の判定基準**: 「false-positive skip が N 回/1000 tick 以下」等の quantitative な受け入れ基準は本 issue のスコープ外。 実運用で false-positive skip が観測されたら別 issue で SKIP_TIMEOUT を 2〜3 tick 相当まで拡大するかを検討する

## 関連

- issues/0085 (`feature/refactor-encoder-inflight-backpressure`): 依存先 (実装 PR は develop に merge 済み、 issue ファイルは open のまま closed 移動待ち)
- issues/0087 (`feature/add-realtime-encoder-param-override`、 open): 相互補完。 0087 の `IN_FLIGHT_LIMIT` 可変化と `requires_backpressure` 応急処置解消により、 本 issue の効果が全 encoder に波及する
- closed/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`、 2026-07-21 closed): 0085 の前身。 本 issue の背景理解に参照
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`、 2026-06-26 決定): 大枠の親 issue
