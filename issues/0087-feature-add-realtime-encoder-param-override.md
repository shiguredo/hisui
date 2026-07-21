# リアルタイム動作時に全エンコーダーに共通の低遅延パラメータを強制上書きする機構を追加する

- Priority: Medium
- Created: 2026-07-21
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/add-realtime-encoder-param-override
- Reporter: @sile
- Decision Owner: @sile

## 目的

hisui のリアルタイム動作 (WebRTC / obsws / rtmp 等の realtime 経路) では、encoder が内部で look-ahead / warm-up バッファを持つと遅延が累積する。 現状は encoder ごとにデフォルトの品質優先パラメータが適用されるため、リアルタイム経路でも品質優先設定 (高遅延) が使われる。

本 issue はリアルタイム動作時に **全 encoder に共通の低遅延パラメータを強制上書きする** 機構を追加する。 個別 encoder 単位の対応 (svt_av1 の `look_ahead_distance` だけ、等) ではなく、リアルタイム経路であることを検出したら関連する全 encoder パラメータを一括で低遅延プロファイルに切り替える枠組みを目指す。

依存: issues/0085 (in-flight bp) 完了後に着手する。 0085 の bp guard が入って初めて encoder 内部 warm-up が「顕在化する遅延」として観測される (それまでは encoder が自主的にバッファリングしても bp なしで上流が全速供給するため、遅延が pipeline 全体の均等化で埋もれる)。

## 優先度根拠

Medium。 リアルタイム経路の品質確保に必要だが、依存先 (0085) 完了後に着手する。

- 0085 が入って初めて encoder in-flight bp が発動し、encoder 内部の warm-up 遅延が顕在化する
- svt_av1 の `look_ahead_distance` (デフォルト SVT-AV1 native default ~33) が realtime で使われると 33 frame 分の遅延が定常化する (30fps で ~1.1 秒)
- video_toolbox にも `max_frame_delay` 等の類似設定がある可能性 (要調査)
- 現時点で observation-driven に動く緊急性はないが、0085 完了後の realtime 経路の遅延実測次第で優先度が上がる可能性

## 現状

hisui の encoder パラメータ override 経路は encoder ごとに個別:

- svt_av1: `src/sora/recording_encoder_svt_av1_params.rs:110-111` で JSON config 経由の `look_ahead_distance` 指定 (主に compose 用の設定として使われる)
- nvcodec: `src/sora/recording_encoder_nvcodec_params.rs` で個別設定
- video_toolbox: 個別設定
- libvpx / openh264: 個別設定

各 encoder は「呼出経路が realtime か compose か」を判別する情報を持たず、統一的な realtime プロファイル定義もない。 現状のリアルタイム経路 (`src/mixer/video.rs::VideoRealtimeMixerRunner` 等) は VideoEncoder を作る際に compose 用のデフォルト config を利用しているため、encoder 内部の warm-up 挙動もそのまま流用される。

## 設計方針

以下は polish で確定する。 現時点では骨組みのみ。

### realtime プロファイルの定義

encoder ごとに低遅延化パラメータを 1 箇所にまとめる (例: `src/encoder/realtime_profile.rs`)。 対象パラメータ (要調査):

- **svt_av1**: `look_ahead_distance = 0`、必要に応じて `preset` (低遅延プリセット)
- **video_toolbox**: `max_frame_delay = 0`、`real_time = true` 相当のフラグ (crate API を要調査)
- **nvcodec**: `idr_period` / `frame_interval_p` (現状 1 固定なので影響小)
- **libvpx**: `lag_in_frames = 0`
- **openh264**: `usage_type` (camera vs screen) / `enable_multi_thread` 等

### realtime 判定の伝播

polish で確定する。 候補:

- (a) VideoEncoder に「realtime モード」flag を持たせる
- (b) realtime 経路の call site (`src/mixer/video.rs` 系) で config を差し替えてから VideoEncoder を作る
- (c) `VideoEncoderOptions` に realtime プロファイルの enum フィールドを追加

### 上書きの粒度

- 個別パラメータレベル (`svt_av1.look_ahead_distance = 0`) か encoder 全体の config レベルか
- ユーザー指定の JSON config を realtime プロファイルが上書きするか、両者をマージするか

### compose 経路への影響なし

compose では品質優先パラメータを維持する。 realtime プロファイルの適用は realtime 経路の call site に限定する。

## 完了条件

polish で確定。

## 解決方法

polish 完了後に追記する。

## CHANGES.md について

polish で確定。 リアルタイム経路の挙動変更として `[UPDATE]` で草案を書く。

## 関連

- issues/0085 (`feature/refactor-encoder-inflight-backpressure`): 依存先。 本 issue は 0085 完了後に着手する
- issues/0086 (`feature/add-realtime-video-mixer-skip-on-encoder-backpressure`): 同じく 0085 完了後の後続。 本 issue と 0086 を組み合わせることで realtime 経路の低遅延化 (encoder 内部の warm-up 削減 + mixer 側のフレームスキップ) を実現する
- closed/0080 (2026-07-21 closed): 0085 の前身。 背景理解に参照
