# リアルタイム動作時に全エンコーダーに共通の低遅延パラメータを強制上書きする機構を追加する

- Priority: Medium
- Created: 2026-07-21
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/add-realtime-encoder-param-override
- Reporter: @sile
- Decision Owner: @sile

## 目的

hisui のリアルタイム動作 (WebRTC / obsws / rtmp 等の realtime 経路) では、 encoder が内部で look-ahead / warm-up バッファを持つと遅延が累積する。 現状は encoder ごとにデフォルトの品質優先パラメータが適用されるため、 リアルタイム経路でも品質優先設定 (高遅延) が使われる。

本 issue はリアルタイム動作時に以下 2 系統の変更を追加する:

1. **全 encoder に共通の低遅延パラメータを強制上書きする** 機構 (`look_ahead_distance = 0` / `lag_in_frames = 0` 等で encoder 内部の warm-up を消す)
2. **`IN_FLIGHT_LIMIT` の可変化** (`effective_limit = min(realtime_limit, encoder_limit)` で encoder 種別と経路依存に決める)

0085 で導入された in-flight バックプレッシャーは、 現状応急処置として nvcodec のみに適用されている (`VideoEncoderInner::requires_backpressure` が nvcodec のみ true)。 これは libvpx VP9 (`lag_in_frames` native default ~25) / svt_av1 (`look_ahead_distance` native default ~33) / video_toolbox が warm-up 型で、 一律 `LIMIT = 3` を適用すると deadlock するための応急処置。 本 issue でこの応急処置を解消し、 encoder 種別と経路 (realtime / compose) に応じた LIMIT を計算して bp guard を全 encoder で有効化する。

依存: issues/0085 (in-flight bp) 完了後に着手する。 0085 の応急処置を前提とする作業なので順序依存が明確。

## 優先度根拠

Medium。 リアルタイム経路の品質確保に必要だが、 依存先 (0085) 完了後に着手する。

- 0085 の応急処置により、 realtime + 非 nvcodec (libvpx VP9 / svt_av1 / video_toolbox) で bp guard が無効化されている副作用を解消する必要がある
- svt_av1 の `look_ahead_distance` (native default ~33) が realtime で使われると 33 frame 分の遅延が定常化する (30fps で ~1.1 秒)
- libvpx VP9 の `lag_in_frames` (native default ~25) も同型で ~830ms の遅延が乗る
- video_toolbox にも `max_frame_delay` 等の類似設定がある可能性 (要調査)
- 0085 の応急処置は `src/encoder.rs::VideoEncoderInner::requires_backpressure` の docstring 通り、 0087 完了時に削除する予定

## 現状

hisui の encoder パラメータ override 経路は encoder ごとに個別:

- svt_av1: `src/sora/recording_encoder_svt_av1_params.rs` で JSON config 経由の `look_ahead_distance` 指定 (主に compose 用)
- nvcodec: `src/sora/recording_encoder_nvcodec_params.rs` で個別設定
- video_toolbox: 個別設定
- libvpx / openh264: 個別設定

各 encoder は「呼出経路が realtime か compose か」を判別する情報を持たず、 統一的な realtime プロファイル定義もない。

0085 応急処置状態 (本 issue で解消対象):

- `src/encoder.rs::VideoEncoderInner::requires_backpressure` が nvcodec のみ true → nvcodec 経路でのみ bp guard 有効
- `src/encoder.rs::VideoEncoder::run` の input 腕 guard が `!self.eos && (!needs_bp || in_flight < IN_FLIGHT_LIMIT)` の形
- `src/encoder.rs::VideoEncoder::IN_FLIGHT_LIMIT: usize = 3` (nvcodec の `n_encoder_buffer - 1` 由来の const)
- 非 nvcodec 経路では bp guard 実質無効化 → warm-up 遅延がそのまま出る

## 設計方針

以下は polish で確定する。 現時点では骨組みのみ。

### `IN_FLIGHT_LIMIT` の可変化

現行の `IN_FLIGHT_LIMIT = 3` const を廃止し、 実効 LIMIT を実行時計算する:

```
effective_limit = min(realtime_limit, encoder_limit)
```

**encoder_limit** (encoder + 設定依存):

| encoder | encoder_limit の算出 | 根拠 |
|---|---|---|
| nvcodec | `n_encoder_buffer - 1` (現状 3) | buffer full 回避 |
| libvpx VP8 | `usize::MAX` (無制限) | 同期 1:1、 warm-up なし |
| libvpx VP9 | `lag_in_frames + 1`、 未指定なら crate native default (~26) | lag 分の余裕 + 1 |
| openh264 | `usize::MAX` (推定) or 実測ベース | 同期経路と推定、 要調査 |
| svt_av1 | `look_ahead_distance + 1`、 未指定なら ~34 | look-ahead 分の余裕 + 1 |
| video_toolbox | 要調査。 delay 系設定に基づく値 or `usize::MAX` | 非同期経路 |

**realtime_limit** (経路依存):

- **realtime 経路**: 1 or 2 (低遅延優先、 issues/0086 の frame skip とセット)。 具体値は polish + 実機計測で確定
- **compose 経路**: `usize::MAX` (encoder_limit に完全に委ねる)

**effective_limit の帰結**:

- compose + nvcodec: 3 (buffer full 回避)
- compose + libvpx VP8 / openh264: `usize::MAX` (bp guard 事実上無効)
- compose + libvpx VP9 / svt_av1 (warm-up あり): 26 / 34 相当 (deadlock 回避、 warm-up 許容)
- realtime + nvcodec: 1 or 2 (低遅延)
- realtime + 他 encoder (warm-up 上書き済み): 1 or 2 (低遅延、 下記の encoder パラメータ強制上書きと連動)

### realtime プロファイルの定義

encoder ごとに低遅延化パラメータを 1 箇所にまとめる (例: `src/encoder/realtime_profile.rs`)。 対象パラメータ (要調査):

- **libvpx (VP8 / VP9)**: `lag_in_frames = 0` (VP9 の native default ~25 を無効化)
- **svt_av1**: `look_ahead_distance = 0`、 必要に応じて `preset` (低遅延プリセット)
- **video_toolbox**: `max_frame_delay = 0`、 `real_time = true` 相当のフラグ (crate API を要調査)
- **nvcodec**: `idr_period` / `frame_interval_p` (現状 1 固定なので影響小)
- **openh264**: `usage_type` (camera vs screen) / `enable_multi_thread` 等 (warm-up 挙動要調査)

### realtime 判定の伝播

polish で確定する。 候補:

- (a) `VideoEncoder` に「realtime モード」flag を持たせる (VideoEncoder::new の引数)
- (b) realtime 経路の call site (`src/mixer/video.rs` 系) で config を差し替えてから VideoEncoder を作る
- (c) `VideoEncoderOptions` に realtime プロファイルの enum フィールドを追加

### 上書きの粒度

- 個別パラメータレベル (`svt_av1.look_ahead_distance = 0`) か encoder 全体の config レベルか
- ユーザー指定の JSON config を realtime プロファイルが上書きするか、 両者をマージするか

### 0085 応急処置の削除

- `src/encoder.rs::VideoEncoderInner::requires_backpressure` を削除
- `src/encoder.rs::VideoEncoderInner::encoder_limit(&self) -> usize` を追加 (encoder ごとの LIMIT を返す)
- `src/encoder.rs::VideoEncoder::run` の guard を `in_flight < effective_limit` に修正 (`effective_limit = min(realtime_limit, encoder_limit)`)
- `src/encoder.rs::VideoEncoder::IN_FLIGHT_LIMIT` const 削除 (または encoder_limit の nvcodec 分岐に移す)

### compose 経路への影響なし

compose では品質優先パラメータを維持する。 realtime プロファイルの適用は realtime 経路の call site に限定する。 `effective_limit` の compose 経路挙動は `realtime_limit = usize::MAX` を返すことで担保する。

## 完了条件

polish で確定。 主要項目 (骨組み):

- `VideoEncoderInner::encoder_limit(&self) -> usize` メソッド追加
- `VideoEncoderInner::requires_backpressure` 削除 (0085 応急処置の解消)
- `VideoEncoder::IN_FLIGHT_LIMIT` const 削除
- realtime プロファイル (`src/encoder/realtime_profile.rs` 等) 実装
- realtime flag の伝播経路 (polish で確定)
- realtime 経路で encoder パラメータ強制上書き
- `VideoEncoder::run` の guard を `effective_limit = min(realtime_limit, encoder_limit)` ベースに修正
- integration test 追加 (compose + libvpx VP9 で deadlock しないこと、 realtime + 全 encoder で低遅延化されること)
- cargo (fmt / check / clippy / test) がすべて通る

## 解決方法

polish 完了後に追記する。

## CHANGES.md について

polish で確定。 リアルタイム経路の挙動変更として `[UPDATE]` で草案を書く。

## 関連

- issues/0085 (`feature/refactor-encoder-inflight-backpressure`): 依存先。 本 issue は 0085 完了後に着手する。 0085 の応急処置 (`VideoEncoderInner::requires_backpressure`) を本 issue 完了時に削除する
- issues/0086 (`feature/add-realtime-video-mixer-skip-on-encoder-backpressure`): 同じく 0085 完了後の後続。 0086 の frame skip は bp guard が動作している前提のため、 本 issue で全 encoder に bp guard を戻すことで 0086 の効果が全 encoder に及ぶようになる
- closed/0080 (2026-07-21 closed): 0085 の前身。 背景理解に参照
