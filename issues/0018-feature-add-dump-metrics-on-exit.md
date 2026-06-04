# obsws server に SIGTERM graceful shutdown と終了時メトリクス JSONL 出力フラグを追加し、失敗診断を改善する

- Priority: Medium
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished: 2026-06-04

## 目的

obsws server プロセスを SIGTERM で graceful に終了できるようにし、終了時に全メトリクスを JSON Lines で標準出力へ吐けるフラグを追加して、失敗診断を容易にする。動機は issues/0011 の調査で、obsws e2e が `/metrics` 全文を失敗メッセージに埋め込み CI ログで切り詰められ、目的のメトリクスが読めなかったこと。

本 issue の主成果物は obsws server への SIGTERM/SIGINT graceful shutdown の新設であり、メトリクスダンプ機能はその上に乗る。フラグ追加が主目的のため category は `feature-add`。

## 優先度根拠

Medium。テスト失敗や障害の原因調査の効率に直結する。ユーザー向け機能ではないため High ではない。

## 現状

- hisui は HTTP で `/metrics`（Prometheus テキスト）と `/metrics?format=json`（prom2json 準拠 JSON。`src/endpoint_http_metrics.rs:98` が `Stats::entries()` ＋ tokio runtime メトリクスを合成）を公開するが、プロセス終了時に自分でメトリクスを吐く手段が無い。
- obsws e2e は失敗時に `/metrics` 全文をスクレイプして失敗メッセージに埋め込む。`/metrics` は `BTreeMap` 由来でメトリクス名のアルファベット順のため、巨大なスナップショットが CI ログで後方から切り詰められ、後ろのメトリクス（`total_missing_*` / `total_received_*` 等）が読めない。
- obsws server には graceful shutdown 経路もシグナルハンドラも存在しない。`src/` に `tokio::signal` は皆無で、`Cargo.toml` の tokio features に `signal` も無い。accept loop（`src/obsws/server.rs:329-340`）は無限ループで、抜けるのは accept エラー時か coordinator の致命的エラー（state file 書き込み失敗、`src/obsws/coordinator.rs:309-317`）時のみ（いずれも `Err`）。`run_server` に正常 `Ok(())` を返す経路は無い。e2e の停止は SIGTERM（`e2e-tests/obsws/helpers.py:143`）だが、ハンドラ無しで即時終了しクリーンアップは走らない。
- `run_server` の呼び出し元は 2 つある。player feature 有効時は別スレッドで `block_on`（`src/subcommand_server.rs:227`）し、メインスレッドは `run_player_control_loop` でブロックする。player 無効時は単一スレッドで `block_on`（`src/subcommand_server.rs:265`）。player は default（`Cargo.toml:112`）で、e2e/CI も player ビルド（CI は `--no-default-features` を付けない、`.github/workflows/e2e-test.yml:23`）。

## 設計方針

### 1. obsws server に SIGTERM/SIGINT graceful shutdown を追加する（本機能の前提）

- `Cargo.toml` の workspace tokio（`:32-43`）の features に `signal` を追加し、用途コメントを付す。現状 `signal` は examples 6 件（`camera_record` / `hls_s3` / `camera_sora_grid` / `mpeg_dash_s3` / `sora_publish` / `sora_source`）が個別指定しているので、workspace へ集約して各 example の `features = ["signal"]` を外す。集約により `signal` 不要な `obsws_bootstrap` にも `signal` が付くが、軽量な feature なので許容する。
- accept loop（`src/obsws/server.rs:329-340`）の `tokio::select!` に SIGTERM/SIGINT 分岐を足し、受信したら loop を抜けて `Ok(())` を返す（致命的エラーの `Err` 経路とは区別する）。SIGTERM を見るのは e2e の停止が SIGTERM のため（既存 examples の `tokio::signal::ctrl_c` は SIGINT のみ）。`tokio::signal::unix` は Windows 非対応なので `#[cfg(unix)]` でガードする（CI/e2e は Linux）。
- 起動途中の取りこぼしを避けるため、`Signal` は `run_accept_loop` 内ではなく `run_server` の入口（`bind` 等の初期化 `.await` より前）で生成し、`run_accept_loop` へ渡す。これで `bind` 完了（e2e はここまでしか待たない）後・accept loop 到達前の窓で SIGTERM が来ても取りこぼさない。
- 終了の連鎖（両経路で成立。`src/subcommand_server.rs`）: `run_server` が `Ok` → `block_on` 完了 → 非 player はそのままプロセス終了、player は runtime スレッド終了で `player_command_tx` の送信端（coordinator が保持（`src/obsws/coordinator.rs:173`）、実行中の output_player も clone を持つが、いずれも runtime 内）が全て drop → `run_player_control_loop` の `recv` が `Err` でループ脱出（`src/subcommand_server.rs:294-295`）→ `runtime_thread.join()`（`:254-256`）。処理中の `spawn_local` 接続・actor タスクは中断されてよい（runtime drop に委ねる）。
- SIGTERM 分岐と既存の `shutdown_rx`（coordinator 致命エラー）分岐が同時発火した場合は `select!` が非決定的に選ぶ。稀だが、`Err` 側が選ばれるとダンプは出ない（許容する）。

### 2. 終了時メトリクスダンプ（フラグ）

- `--dump-metrics-on-exit`（`noargs::flag`、env: `HISUI_DUMP_METRICS_ON_EXIT`、boolean）を `server` サブコマンド（`src/subcommand_server.rs`）に追加し、`run_internal` → `run_server` → `run_accept_loop` へ `bool` 引数で引き回す。`run_server` の呼び出しは player（`:227`）と非 player（`:265`）の 2 箇所あり、両方に引数を追加する（`run_server` / `run_accept_loop` は既に `#[expect(clippy::too_many_arguments)]`）。
- 有効時、設計方針 1 の signal 分岐で loop を抜ける直前（`run_accept_loop` 内。`pipeline_handle` は per-connection で clone するが本体は関数に残る）に、`pipeline_handle.stats()`（`MediaPipelineHandle::stats()`、`src/media_pipeline.rs:709`）で `Stats` を取得し、`Stats::entries()`（`src/stats.rs:151`）で全メトリクスを取り、各 `StatsEntry`（`src/stats.rs:524-529`）を 1 行 1 メトリクスの JSON Lines で **stdout** に出力する（tracing ログは stderr に出る（`src/logger.rs:124`）ため分離）。致命的エラー終了（`Err` 経路）でのダンプは対象外。
- `StatsEntry` には `DisplayJson` 実装が無いため、`src/stats.rs` に公開関数を追加し、`nojson` で `name` / `labels` / `value` の object を組む（`PrometheusMetricSample` の `DisplayJson`（`src/stats.rs:557`）が前例）。JSONL の 1 行スキーマ:
  - 例: `{"name":"total_finalize_failure_count","labels":{"processor_id":"output:record:mp4_writer:0","processor_type":"hybrid_mp4_writer"},"value":1}`
  - `name` は `hisui_` prefix 無しの `StatsEntry.metric_name`（prefix は Prometheus 描画時のみ付与、`src/stats.rs:120`）。
  - `labels` は `StatsLabels` の既存 `DisplayJson`（`src/stats.rs:513-522`）。
  - `value` は `StatsValue` の既存 `DisplayJson`（`src/stats.rs:321-335`: Counter→u64, Gauge→i64, GaugeF64→f64, Duration→秒 f64, Flag→bool, StringValue→文字列）。型混在を許容し、`/metrics?format=json` のような StringValue の value ラベル展開はしない。
  - tokio runtime メトリクス（HTTP 経路のみ合成）は含めず、`Stats` レジストリ由来のみ（本 issue の「全メトリクス」はこの意味）。
- 録画 processor 終了後も `Stats` レジストリにメトリクスが残る（`Stats` は `Arc<Mutex<BTreeMap>>`、`src/stats.rs:11-12`。issues/0011 の CI 失敗時、StopRecord 後の `/metrics` に mp4_writer のメトリクスが残っていた）。

### 3. e2e 側: フラグを既定で使い、診断を終了ダンプに委ねる

- `ObswsServer.__init__`（`e2e-tests/obsws/helpers.py:32-57`）に `dump_metrics_on_exit: bool = True` を追加し、`start()`（`:68-136`）の CLI 引数経路と `use_env=True` 経路（env のみで設定、`:78` のコメント参照。`HISUI_DUMP_METRICS_ON_EXIT`）の両方で、この引数が真のとき付与する（無効化テストはこの引数を `False` で起動する）。env 経路では `noargs::flag` が「変数が存在し空でなければ有効」と解釈するため、無効化は変数値を `false` 等にするのではなく変数自体を設定しないこと。
- `stop()`（`:138-153`）は SIGTERM 送信後に `wait(timeout=5.0)` してから `communicate()` で pipe を drain する。終了ダンプで stdout pipe が詰まるとサーバが exit できず 5 秒 SIGKILL でダンプを失うため、`stop()` を `communicate(timeout=5.0)`（待機しつつ pipe を並行 drain）ベースに変更する（`TimeoutExpired` を捕捉して `kill()` 後に再度 `communicate()` する。再呼び出しで既読出力は失われない）。
- 失敗診断は終了ダンプ（`_emit_captured_output`、`:169-173` が captured stdout/stderr を print し pytest が失敗時に表示）に委ねる。obsws 録画系テストが `/metrics` 全文を失敗メッセージに埋め込む箇所（`_format_obsws_diagnostics` 経由の `_inspect_mp4(diagnostics_text=...)`、および `AssertionError` への直接 f-string 埋め込み）を `grep -n "_collect_obsws_metrics_snapshot\|metrics_snapshot" e2e-tests/obsws/*.py` で棚卸しする（grep 結果のうち録画系の埋め込みのみが廃止対象。`test_bootstrap.py` の成功時 print と配信系は下記「残すもの」の通り除外）。廃止後、assert は簡潔な失敗理由のみにする。未使用の `_print_obsws_diagnostics`（`:520`、参照ゼロ）は削除する。
- 残すもの: 配信系テスト（rtmp/srt stream）の「配信中スナップショット」は最終状態で代替できないため残す（本 issue の主目的は録画系の切り詰め解消）。`test_bootstrap.py` の成功時 print も対象外。録画中のライブ待ち（`_has_positive_metric` で `/metrics` を見て映像サンプル等を待つ）は終了前の状態が必要なので HTTP `/metrics` のまま残す。

## 完了条件

- フラグ有効時、obsws server を SIGTERM で停止すると、全メトリクス（`Stats` レジストリ全件）が JSON Lines で stdout に出力され、1 行 1 メトリクスで grep / parse できること（形式は設計方針 2 の例）。`__init__` 引数を `False` にすると出力されないこと。
- 非 player ビルドがコンパイルできること（CI の `cargo clippy --workspace --no-default-features`）。ダンプ挙動の検証は player ビルドの e2e で行う。
- E2E サーバが既定で `--dump-metrics-on-exit` を使い、SIGTERM で停止する録画系テストの失敗時に、最終メトリクス（finalize 成否 / sample_entry の received・missing 等）が切り詰められず確実に読めること（`server.kill()`（SIGKILL）で止めるテストは終了ダンプの対象外）。
- 録画系の失敗メッセージへの `/metrics` 全文埋め込み（上記 grep で特定した箇所）が無くなり、未使用 `_print_obsws_diagnostics` が削除されていること。
- 変換関数の単体テストが追加されていること（下記テスト戦略）。
- `CHANGES.md` の `## develop` に `[ADD]` エントリを追記すること（フラグ追加と SIGTERM graceful 終了。現状の SIGTERM は OS 既定の即時終了で hisui が保証した挙動ではなく、graceful 経路の新設は後方互換を壊さないため `[ADD]`）。e2e 改修は `### misc` に記載する。

## テスト戦略

- hisui 側: 1 `StatsEntry` → 1 JSON 行の変換関数の単体テストを `src/stats.rs` の `#[cfg(test)] mod tests` に追加する（本リポジトリに PBT クレートは未整備のため単体テストで対応。各 `StatsValue` 型 Counter / Gauge / GaugeF64 / Duration / Flag / StringValue について、出力が valid JSON でパースでき name / labels / value が保持されることを検証）。
- e2e 側: 録画系テストで「SIGTERM 停止後、captured stdout に JSONL が出ており目的メトリクスがパースできる」結合検証と、`dump_metrics_on_exit=False` でダンプが出ないことの確認を追加する。

## 想定エッジケース

- `Stats::entries()` がロック poison で `Err`: ダンプを諦め警告ログ（英語）を出し、終了は続行する。
- メトリクスが空: 0 行で正常終了する。
- stdout への書き込み失敗: 警告ログを出して終了を続行する（終了処理をブロックしない）。

## 関連

- issues/0011（調査元。reopen 済み）
- issues/0017（音声 sample_entry 要求機構の修正。その flaky 検証時に本機能の診断が役立つ）
