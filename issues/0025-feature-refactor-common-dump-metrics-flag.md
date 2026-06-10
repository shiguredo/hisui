# `--dump-metrics-on-exit` を main.rs の共通フラグへ昇格し、main 側で共有 `Stats` を保持する

- Priority: Low
- Created: 2026-06-05
- Completed:
- Model: Opus 4.8
- Branch:
- Polished: 2026-06-10

## 目的

issue 0018 で server サブコマンド固有として導入した `--dump-metrics-on-exit`（プロセス終了時に全メトリクスを `{"type":"metrics", ...}` の 1 行 JSON で stdout 出力するフラグ）を、`src/main.rs` で受け付ける共通フラグへ昇格させる。あわせて、`Stats` レジストリを main 側で 1 つ作って `MediaPipeline` を持つ subcommand に渡し、終了時の dump も main 側で実行する枠組みに整える。

主目的は「将来追加する `MediaPipeline` 保有 subcommand が、コマンド側でフラグを意識せずに同じ仕組みへ素直に乗れる状態」を作ること。既存 batch 系（compose / vmaf / inspect）を実用 dump 対象として doc・出力整形に組み込むことは本 issue では行わない。

## 優先度根拠

Low。server 固有版（0018）で当面の用途（server の flaky 診断）は満たせており、既存 batch 系への対応拡大は単体ではメリットが薄い。一方、将来追加するであろう新 subcommand が共通フラグで終了時ダンプを得られる枠組みは、後で整理し直すより今のうちに整えておく方が安い。

本 issue は CLI 上のフラグ位置を `hisui server ... --dump-metrics-on-exit` から `hisui --dump-metrics-on-exit server ...` へ移すが、外部から観察可能な挙動（server で `--dump-metrics-on-exit` を ON にすると終了時に `type=metrics` の JSON Line が stdout に出る）は維持する。0018 は本 issue 着手時点で未リリース（`CHANGES.md:12` の `## develop` セクション内）であり、CLI 位置変更は develop ブランチ内の中間状態整理に当たる。リリース前の内部設計整理として `feature/refactor-` カテゴリで扱う。

## 現状

- `--dump-metrics-on-exit` の parse は `src/subcommand_server.rs:113` で server 固有。`run_server` → `run_accept_loop` の SIGTERM 分岐（`src/obsws/server.rs:405`）で `dump_metrics_to_stdout(pipeline_handle)` を呼ぶ。
- ダンプ本体 `src/obsws/server.rs:100-124` `dump_metrics_to_stdout` は `pipeline_handle.stats().to_prometheus_json_families()` を `{"type":"metrics", "metrics": ...}` で包んで stdout に書く。JSON Lines のエントリ種別（`type`）の付与は出力側の責務で、`Stats` 側には持たせない（0018 のレビューで確定済み）。
- `src/main.rs:22-30` に共通フラグの前例（`--verbose`）がある。
- `Stats`（`src/stats.rs:11-30`）は `Clone` 可能で `clone()` 同士は `shared_entries` を共有する。`set_default_label` は `Arc<StatsLabels>` を差し替える実装で片側のみに作用する。`StatsKey` は entry 登録時の `default_labels` を値として保持するため（`src/stats.rs:194-197`）、subcommand 側 stats で `set_default_label` を呼んだ後に登録された entry の labels は、main 側 stats から `entries()` を取得しても正しく復元される。
- `MediaPipeline::new()` は `MediaPipeline::new_with_config(MediaPipelineConfig::default())` への委譲、`new_with_config()` は内部で `Stats::new()` を作る（`src/media_pipeline.rs:58-92`）。
- 実プロダクションパスでの `MediaPipeline::new()` / `new_with_config()` 呼び出しは以下の 4 箇所:
  - `src/subcommand_inspect.rs:105`（`new()`）
  - `src/sora/recording_subcommand_compose.rs:269`（`new()`）
  - `src/sora/recording_subcommand_vmaf.rs:247`（`new()`）
  - `src/obsws/server.rs:270`（`new_with_config()`）
  - これ以外の `MediaPipeline::new()` 呼び出しは全て `#[cfg(test)]` 配下。
- tune は内部で `current_exe` 経由で `vmaf` 子プロセスを起動する（`src/sora/recording_subcommand_tune.rs:301` の `Command` 構築、1 箇所のみ）。tune 親自身は stdout に何も書かない（出力は全て stderr）。tune 親は子の stdout を `nojson::RawJson::parse` で読む。
- list-codecs は `MediaPipeline` を持たず、`src/subcommand_list_codecs.rs:148` の `println!` でコーデック一覧 JSON を 1 個 stdout に書く。
- 既存 server e2e: `e2e-tests/obsws/test_output.py:2096-2132` の `test_obsws_dump_metrics_on_exit_outputs_jsonl` / `test_obsws_dump_metrics_on_exit_disabled`。両テストとも「stdout に `type=metrics` の JSON Line が含まれるか」を確認するのみ。
- e2e helpers の `ObswsServer.start()`（`e2e-tests/obsws/helpers.py`）は CLI 経路（`--dump-metrics-on-exit` を `args` に append）と env 経路（`HISUI_DUMP_METRICS_ON_EXIT=1` を env に設定）の 2 経路を持つ。
- `CHANGES.md:59-61` に 0018 関連の `[ADD]` エントリ（3 行構造: 本文 + env 補足 + `@sile`）があり、未リリース。

## 設計方針

### 1. 共通フラグ parse の移設

`--dump-metrics-on-exit`（env `HISUI_DUMP_METRICS_ON_EXIT` を維持）の parse を `src/main.rs` の `--verbose` 直後（subcommand 分岐前）へ移す。CLI 上の位置は `hisui --dump-metrics-on-exit server ...` を規約とする。env 経路は CLI 位置と独立に動く。

### 2. main 側で共有 `Stats` を作って渡す

- main で `Stats::new()` を 1 つ作って main 関数のローカル変数として保持する。`MediaPipeline` を持つ subcommand の `try_run` には `stats.clone()` を値で渡す（最初の subcommand 呼び出しで move されないよう main 側で保持し続ける）。
- 生成はフラグの ON / OFF にかかわらず常に行い、dump 呼び出し側の判定で ON / OFF を分岐する。

### 3. `MediaPipeline` への Stats 注入 API 追加

`src/media_pipeline.rs` に Stats 注入版を追加し、既存 API は委譲に書き換える:

| API | 委譲先 |
| --- | --- |
| `new_with_config_and_stats(config, stats)` | 本体（現 `new_with_config` のロジックで `Stats::new()` を引数 `stats` に置き換え） |
| `new_with_stats(stats)` | `new_with_config_and_stats(MediaPipelineConfig::default(), stats)` |
| `new_with_config(config)` | `new_with_config_and_stats(config, Stats::new())` |
| `new()` | `new_with_config_and_stats(MediaPipelineConfig::default(), Stats::new())` |

新 API は `pub fn ... -> crate::Result<Self>` のシグネチャ。本体では受け取った `stats` をそのまま `self.stats` に格納する（追加 clone しない、`set_default_label` 等の副作用は呼ばない）。既存呼び出し（テスト・obsws/source の `#[cfg(test)]` ブロックを含む）は挙動不変。

### 4. 4 箇所の実プロダクションパスを新 API に置き換え

| 既存呼び出し | 置き換え先 |
| --- | --- |
| `src/subcommand_inspect.rs:105` `MediaPipeline::new()?` | `MediaPipeline::new_with_stats(stats)?` |
| `src/sora/recording_subcommand_vmaf.rs:247` `MediaPipeline::new()?` | `MediaPipeline::new_with_stats(stats)?` |
| `src/sora/recording_subcommand_compose.rs:269` `MediaPipeline::new()?` | `MediaPipeline::new_with_stats(stats)?` |
| `src/obsws/server.rs:270` `MediaPipeline::new_with_config(pipeline_config)?` | `MediaPipeline::new_with_config_and_stats(pipeline_config, stats)?` |

### 5. `try_run` シグネチャ変更と内部関数への引き回し

| subcommand（crate path は `hisui::` 配下、表では簡略表記） | 現状 | 新 |
| --- | --- | --- |
| `subcommand_inspect::try_run` | `(args)` | `(args, stats: Stats)` |
| `sora::recording_subcommand_compose::try_run` | `(args)` | `(args, stats: Stats)` |
| `sora::recording_subcommand_vmaf::try_run` | `(args)` | `(args, stats: Stats)` |
| `subcommand_server::try_run` | `(args)` | `(args, stats: Stats)` |
| `subcommand_list_codecs::try_run` | `(args)` | 据え置き |
| `sora::recording_subcommand_tune::try_run` | `(args)` | 据え置き（内部で `env_remove` を追加。設計方針 9 参照） |

- 各 subcommand 内部の `MediaPipeline::new*` を呼ぶ最深層関数まで `Stats` を引き回す。`pipeline_handle` を受けるだけで pipeline を作らない関数には渡さない。具体的なチェーンは「現状」の `MediaPipeline::new*` 呼び出し 4 箇所から最深層関数まで遡って特定する。
- server の `run_internal` は player feature 有効時のワーカースレッドへ `move` するクロージャに `stats` を取り込む。player / 非 player 両経路で `run_server` の引数に渡す。
- server の `dump_metrics_on_exit: bool` 受け渡しチェーン（`src/subcommand_server.rs:113-117, 171, 192, 247, 286` と `src/obsws/server.rs:140, 364, 389, 405-407`）は削除し、代わりに `stats` を `run_server` まで通す。`run_accept_loop` には `stats` を渡さない（dump は main 末尾に一本化）。SIGTERM 分岐内の dump 呼び出しも削除する。

### 6. dump 関数の配置

- 現状の `src/obsws/server.rs:100-124` `dump_metrics_to_stdout` は削除する。
- `src/metrics_dump.rs` を新設して `pub fn dump_metrics_to_stdout(stats: &Stats)` を置き、`src/lib.rs` から `pub mod metrics_dump;` として公開する。main からは `hisui::metrics_dump::dump_metrics_to_stdout(&stats)` で呼ぶ。`Stats` モジュール（`src/stats.rs`）には JSON Lines の `type` 規約を持ち込まない（0018 のレビュー判断を踏襲）。lib crate 配下にするのは `src/main.rs` を薄く保つため。
- 関数の中身（`stats.to_prometheus_json_families()` → JSON Lines で stdout writeln、BrokenPipe 黙殺、その他 I/O エラーは警告ログ、`to_prometheus_json_families` の `Err` も警告ログを出して dump を諦め終了は続行）は 0018 で確立した既存仕様をそのまま移植する。

### 7. main 末尾での dump 呼び出し

- main の subcommand 分岐の `||` チェーン戻り値を `matched: bool` で受け取り、以下を全て満たすときに `hisui::metrics_dump::dump_metrics_to_stdout(&stats)` を呼ぶ:
  - `--dump-metrics-on-exit` が ON
  - `matched == true`（いずれかの subcommand が `try_run` で `true` を返した。subcommand 未指定や unknown subcommand では dump しない）
  - `args.metadata().help_mode == false`（ヘルプ表示モードでは dump しない。subcommand 内 help_mode 早期 return で `try_run` が `Ok(true)` を返すケースを除外）
  - 全 `try_run` が `Ok` で抜けた（`?` で `Err` 抜けした場合は dump しない。bind error / coordinator 致命エラー等で現状の「SIGTERM 分岐内のみ dump」と等価）
- 呼び出し位置は `args.finish()?` の前に置く。`args.finish()` 時点で unknown argument 等の `Err` が返る場合、dump は既に出ている（最大努力で dump を出す方針。0018 の現挙動と等価）。

### 8. server の dump タイミング変化

現状: SIGTERM 分岐内で dump → graceful shutdown → return。

新方式: SIGTERM 受信 → graceful shutdown → `subcommand_server::try_run` から `Ok` で抜け → `args.finish()` 直前の main 末尾で dump。

e2e テストは「stdout に `type=metrics` 行が含まれるか」だけを見ているためタイミングのズレは試験結果に影響しない。運用上は dump 行のタイミングが「SIGTERM 受信直後」から「runtime 終了後」へ移動するが、0018 が未リリースのため既存ユーザーの前提に影響しない。

### 9. 共通フラグ化に伴う stdout 混在の扱い

main 末尾で dump する設計のため、`--dump-metrics-on-exit` を指定した上で server 以外を実行すると subcommand の stdout 出力と dump の JSON Line が並ぶ:

- compose / vmaf / inspect は結果 JSON を stdout に書くため、dump 行が後ろに並ぶ「単一 JSON + JSON Line」の混在出力になる。stdout を機械処理する用途では注意が必要。
- list-codecs は `MediaPipeline` を持たず `Stats` が空のため、`{"type":"metrics","metrics":[]}` の空行が後続する。`HISUI_DUMP_METRICS_ON_EXIT=1` を env で常設した状態で `hisui list-codecs | jq` を実行するとパースが壊れる。これは「list-codecs / tune は dump 対象外」という暗黙の仕様（`MediaPipeline` を持たないため）の帰結として doc 注意でカバーする。
- tune は子 `vmaf` の stdout を `nojson::RawJson::parse` で解析する。env 継承で子側にも dump が出ると tune 親のパースが壊滅する。このため `src/sora/recording_subcommand_tune.rs` の `Command::new(&hisui_exe)` 直後のメソッドチェーンで `cmd.env_remove("HISUI_DUMP_METRICS_ON_EXIT")` を追加する。本対応は本 issue のスコープに含める。

doc 文では特定 subcommand を名指しせず、「プロセス終了時に内部メトリクスを JSON Lines 形式で標準出力へ 1 行出力する。標準出力を機械処理する用途では他の subcommand 出力との混在に注意」相当の中立的記述とする。

### 10. 実装順序

中間状態でビルドが壊れないよう以下の順で進める。Step 1 は委譲化のみで挙動不変、Step 2 は dump 関数モジュール追加のみで使われない、Step 3 で全変更を同一コミットに集約する。

1. `src/media_pipeline.rs` に `new_with_stats` / `new_with_config_and_stats` を `pub` で追加し、既存 `new()` / `new_with_config()` を委譲に書き換える（全テスト通る、挙動不変）。
2. `src/metrics_dump.rs` を新設し `pub fn dump_metrics_to_stdout(stats: &Stats)` を実装、`src/lib.rs` から公開する（既存経路はまだ無変更、ビルド通る）。
3. 同一コミット内で以下を全て実施する:
   - 4 箇所の `MediaPipeline::new*` 呼び出しを新 API に置き換え、各 subcommand の `try_run` → 内部関数チェーンへ `stats` を引き回す。
   - `src/main.rs` で `--dump-metrics-on-exit` を parse、`Stats::new()` を生成、各 `try_run` に `stats.clone()` を渡し、末尾で dump を呼び出す。
   - `src/subcommand_server.rs` と `src/obsws/server.rs` から `dump_metrics_on_exit` 受け渡しチェーン・SIGTERM 分岐内 dump 呼び出し・`obsws/server.rs` の `dump_metrics_to_stdout` 自由関数を削除する。
   - `src/sora/recording_subcommand_tune.rs` の `Command::new(&hisui_exe)` 直後のメソッドチェーンに `cmd.env_remove("HISUI_DUMP_METRICS_ON_EXIT")` を追加する。
   - `e2e-tests/obsws/helpers.py` の CLI 経路でフラグ位置を `server` の前に移す。
4. `CHANGES.md` の 0018 関連エントリを共通フラグ前提に書き換える（ビルド独立、別コミットでもよい）。

## 完了条件

- `--dump-metrics-on-exit`（および env `HISUI_DUMP_METRICS_ON_EXIT`）が `src/main.rs` の共通フラグとして subcommand 分岐前に受け付けられること。
- main で 1 度 `Stats::new()` を呼び、その `clone()` が `MediaPipeline` を持つ全 subcommand（server / compose / vmaf / inspect）の `try_run` 引数に渡されていること。
- `--dump-metrics-on-exit` ON で server を SIGTERM 停止したとき、`{"type":"metrics", ...}` の 1 行が stdout に出ること。
- 以下の経路で dump が出ないこと:
  - `?` 抜け（bind error / coordinator 致命エラー等）
  - ヘルプモード（`hisui --help`、`hisui server --help` 等）
  - subcommand 未指定 / unknown subcommand（`hisui --dump-metrics-on-exit unknown-subcmd` 等）
- list-codecs / tune では `{"type":"metrics","metrics":[]}` の空 1 行が出ること（仕様として許容）。
- 既存の server e2e 2 件がフラグ位置変更後も通ること。
- `e2e-tests/obsws/helpers.py` の CLI 経路で `--dump-metrics-on-exit` が `server` サブコマンドの前に置かれるよう改修されていること。env 経路は env 名 `HISUI_DUMP_METRICS_ON_EXIT` 維持のため改修不要。
- server 固有の `dump_metrics_on_exit` 受け渡しチェーン・SIGTERM 分岐内 dump 呼び出し・`dump_metrics_to_stdout` 自由関数が削除され、`src/metrics_dump.rs` 経由で main 末尾から dump が呼ばれていること。
- `src/sora/recording_subcommand_tune.rs` の `Command::new(&hisui_exe)` 直後で `env_remove("HISUI_DUMP_METRICS_ON_EXIT")` が呼ばれていること。
- `CHANGES.md:59-61` の 0018 関連 `[ADD]` エントリを書き換えること（書き換え例は下記）。新規エントリは追加しない（shiguredo-changelog 規約「中間状態の修正は別エントリにしない」準拠）。`@sile` 行は維持する。
- フラグの doc 文に「プロセス終了時に内部メトリクスを JSON Lines 形式で標準出力へ 1 行出力する。標準出力を機械処理する用途では他の subcommand 出力との混在に注意」相当の記述を入れること。

### `CHANGES.md` 書き換え例

書き換え前（`CHANGES.md:59-61`）:

```
- [ADD] obsws server が SIGTERM / SIGINT でグレースフルシャットダウンするようになり、`--dump-metrics-on-exit` 指定時はプロセス終了時に全メトリクスを JSON Lines で標準出力へ出力する
  - 環境変数 `HISUI_DUMP_METRICS_ON_EXIT` でも有効化できる
  - @sile
```

書き換え後（案）:

```
- [ADD] obsws server が SIGTERM / SIGINT でグレースフルシャットダウンするようになる。あわせて hisui 共通フラグとして `--dump-metrics-on-exit` を追加し、subcommand の終了時に内部メトリクスを JSON Lines 形式で標準出力へ出力する
  - 環境変数 `HISUI_DUMP_METRICS_ON_EXIT` でも有効化できる
  - @sile
```

## テスト戦略

- 既存 server e2e（`test_obsws_dump_metrics_on_exit_outputs_jsonl` / `test_obsws_dump_metrics_on_exit_disabled`）がフラグ位置変更後も通ることで検証する。
- 本 issue では新規テストを追加しない（`metrics_dump::dump_metrics_to_stdout` の単体テスト、batch 系での dump 動作確認、tune の `env_remove` 結合テスト、いずれも）。`Stats::to_prometheus_json_families` 単体テスト群と server e2e で 0018 と同等のカバー範囲を維持する。BrokenPipe / I/O 警告ログ経路はテストしない。

## 非対象

- compose / vmaf / inspect / tune / list-codecs を能動的に dump 対象として整備すること（doc 列挙、出力混在の解消、subcommand 内での dump 抑止、各 subcommand 専用整形等）。
- compose の `--stats-file` との出力経路統合（両者は独立した経路として残す）。
- server のエラー return 時（SIGTERM 以外の原因）にも dump すること。RAII guard 等は導入しない。
- inspect の Decoder 内 `Stats::new()` 個別生成箇所（`src/subcommand_inspect.rs:196, 219`）の共有 `Stats` への合流。これらは `MediaPipelineHandle::stats()` を経由しておらず、本 issue 後も inspect の dump 出力には AudioDecoder / VideoDecoder のメトリクスが含まれない（pipeline 側 `shared_entries` に登録された family のみ含まれる）。inspect を能動的に dump 対象として整備する際に別途扱う。

## 関連

- issues/closed/0018-feature-add-dump-metrics-on-exit.md（server 固有版。本 issue はその枠組み整理）
- issues/closed/0026-bug-fix-sdl-signal-graceful-shutdown.md（0026 で「tokio の SIGTERM ハンドラが SDL と競合しない」ことを確認済み。dump 位置を main 末尾に移しても player 経路の `runtime_thread.join()` 完了→ main 末尾到達は同じ前提に依存する）
