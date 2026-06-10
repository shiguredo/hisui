# sample_entry カウンタ観測 API を廃止して writer 不変条件違反検知に置き換える

- Priority: Low
- Created: 2026-06-10
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/change-encoded-frame-sample-entry-counters
- Polished:

## 目的

issue 0030 で「エンコード済みフレームは常に sample_entry を持つ」という不変条件をリーダー / 音声入力経路と writer 補完削除に適用したことで、`Mp4WriterStats` の `received_*_sample_entry_count` / `missing_*_sample_entry_count` カウンタは意味を失う。本 issue ではこれらカウンタを廃止し、代替として writer 入口に不変条件違反検知ロギング（`tracing::warn!`）を入れる。

これは外部観測 API（`pub fn` および `/metrics` Prometheus メトリクス名）の削除を含む破壊的変更であるため `feature/change-` プレフィックスで対応し、CHANGES.md に CHANGE エントリを追加する。

## 優先度根拠

Low。本 issue は機能バグ修正ではなく、issue 0030 で不変条件を確立したことに伴う観測 API の整理。実害は無い（カウンタが意味を失っても動作上の問題は生じない）が、放置すると `received_*` は「全フレーム数」、`missing_*` は「常に 0」の死んだ指標として残り、運用者を混乱させる。観測 API の意味論を一貫させるための整理として優先度 Low で実施する。

## 現状

issue 0030 完了後、以下の状態になる:

- `Mp4WriterStats.received_*_sample_entry_count`（`src/mp4/writer.rs:93-94`）: 0030 では `hybrid_writer` の `last_*_sample_entry` フィールド・`handle_*_message` 内の保持取り込み・`add_received_*_sample_entry` 呼び出し・`SharedSampleEntry::changed_since` 判定は残されている。不変条件下では全フレームで sample_entry が `Some` になり、`changed_since` の対象（直前値）も初期化時の `None` から最初のフレームで `Some` になった後はずっと `Some` のため、計上は実質「トラック先頭の 1 回 + sample_entry の値が変わる場合」のみ。「変化数」を追う semantics は崩れていないが、不変条件下で全フレーム同一 sample_entry が支配的なケースでは情報価値が薄れる
- `Mp4WriterStats.missing_*_sample_entry_count`（`src/mp4/writer.rs:95-96`）: 0030 で `add_missing_*_sample_entry` 呼び出し（`src/mp4/hybrid_writer.rs:220` / `:259` の `.or_else()` フォールバック削除に伴うもの）が削除されたため、`pub(crate) fn add_missing_*_sample_entry` メソッド（`:229-235`）は `#[allow(dead_code)]` 属性付きで呼び出し元なしの状態。Prometheus メトリクス `hisui_total_missing_*_sample_entry_count` は 0 固定値として `/metrics` に出力され続けている
- 公開 API: `pub fn total_received_*_sample_entry_count()` / `total_missing_*_sample_entry_count()`（`src/mp4/writer.rs:337-350`）が `Mp4WriterStats` 公開メソッドとして残る
- Prometheus メトリクス: `stats.counter("total_received_*_sample_entry_count")` / `stats.counter("total_missing_*_sample_entry_count")` 経由で `/metrics` HTTP エンドポイント（`src/endpoint_http_metrics.rs:33` で `to_prometheus_text()`）から `hisui_total_received_*_sample_entry_count` / `hisui_total_missing_*_sample_entry_count` として外部に公開される

## 設計方針

### 1. カウンタとメソッドの削除

`src/mp4/writer.rs` から以下を削除する:

- `Mp4WriterStats` のフィールド（`:93-96`）:
  - `total_received_audio_sample_entry_count`
  - `total_received_video_sample_entry_count`
  - `total_missing_audio_sample_entry_count`
  - `total_missing_video_sample_entry_count`
- `stats.counter("total_*_sample_entry_count")` 初期化（`:127-134`）と struct 初期化（`:171-174`）
- `add_received_*_sample_entry()` / `add_missing_*_sample_entry()` メソッド（`pub(crate)`、`:221-235`）
- `total_received_*_sample_entry_count()` / `total_missing_*_sample_entry_count()` メソッド（`pub fn`、`:337-350`）

### 2. 違反検知ロギングの追加

`src/dash/writer.rs` / `src/hls/writer.rs` / `src/mp4/hybrid_writer.rs` / `src/mp4/writer.rs` の writer 入口で、`frame.sample_entry.is_none()`（エンコード済みフレームでの不変条件違反）を検知して `tracing::warn!` でログ出力する。

ログには codec / track_id / timestamp を含めて違反元の特定を容易にする。

fail-fast Err は採用しない。理由:

- dash / hls writer の `run` は `handle_*_frame` の Err を `tracing::warn!` で握り潰して継続する設計のため、`Err` を返しても fail-fast にならない
- issue 0011 / 0017 で確立した「録画は壊さない」精神と整合しない（1 フレームの違反で配信全体が停止するのは過剰）
- 違反は obsws 配線では起きない前提なので、検知 + ログで「上流に未対応 issue が残っている」シグナルを残せば十分

### 3. 違反フレームのフォールバック方式

違反フレーム（`frame.sample_entry.is_none()`）の扱いは以下のいずれかを採用する。実装着手時に決める:

- (a) skip（当該フレームを muxer に渡さない）
- (b) 補完値（直前の sample_entry を一時保持する小さなフィールドを残し、違反時のみ使う）

推奨は (b)。「直前の sample_entry を保持する」フィールドだけ最小限残し、通常時は `frame.sample_entry` を使い、違反時のみ補完値を使う。これは issue 0030 の「writer 補完削除」と矛盾しない（補完が常時パスではなく違反時の救済になる）。フィールド名は `fallback_*_sample_entry` 等にして「補完用途」を明示する。

### 4. CHANGES.md エントリ追加

`## develop` に以下の [CHANGE] エントリを追加する（shiguredo-changelog 規約に準拠）:

- `Mp4WriterStats` の以下の公開メソッドを削除する:
  - `total_received_audio_sample_entry_count()`
  - `total_received_video_sample_entry_count()`
  - `total_missing_audio_sample_entry_count()`
  - `total_missing_video_sample_entry_count()`
- `/metrics` HTTP エンドポイントから以下の Prometheus メトリクスを削除する:
  - `hisui_total_received_audio_sample_entry_count`
  - `hisui_total_received_video_sample_entry_count`
  - `hisui_total_missing_audio_sample_entry_count`
  - `hisui_total_missing_video_sample_entry_count`
- 廃止理由: issue 0030 の不変条件成立により観測価値を失ったため

## 完了条件

- `Mp4WriterStats` から `received_*_sample_entry_count` / `missing_*_sample_entry_count` カウンタ・メソッド・pub fn が削除されていること
- writer 入口（hybrid_writer / dash / hls / mp4_writer）に不変条件違反検知ロギング（`tracing::warn!`）が入っていること
- 違反フレームのフォールバック（skip または補完値）が実装されていること
- `feature/change-` プレフィックスのブランチで実装されていること
- CHANGES.md `## develop` に [CHANGE] エントリが追加されていること
- 既存テストが通ること
- 新規テストで違反検知ログが出ることを検証
- `/metrics` HTTP エンドポイントの出力に該当メトリクスが含まれなくなっていること

## 関連

- issue 0030（直接の前提。本 issue は 0030 の writer 補完削除に伴う観測 API 整理）
- issue 0011（カウンタの起源。録画 finalize 失敗の真因調査。closed）
- issue 0017（カウンタを `changed_since` ベースに再定義した issue。closed）
