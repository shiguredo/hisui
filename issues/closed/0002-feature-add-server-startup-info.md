# server サブコマンドが起動直後に実バインド情報を JSON Lines で出力できるようにする

- Priority: Medium
- Created: 2026-05-29
- Completed: 2026-06-10
- Model: Opus 4.7
- Branch: feature/add-server-startup-info
- Polished: 2026-06-10

## 目的

`hisui server` を `--port 0` のようにポート 0 を指定して起動した際に、OS が割り当てた実際のポート番号を呼び出し側（典型的には E2E テストや起動スクリプト）が確実に取得できる手段がない。E2E テストでは固定ポートを使うと並行実行で衝突するため、ポート 0 でカーネル任せに割り当ててから実値を取得したい場面が多い。本対応では、起動直後に実バインド情報を 1 行の JSON (JSON Lines 形式) として stdout へ出力できるようにする。

## 優先度根拠

- E2E テストでポート競合を避けるための定石（ポート 0 → 実ポート取得）が踏めず、テストの並行実行性が下がる。CI の生産性に直結する。
- 実装範囲は `obsws::server::run_server` の bind 直後にごく短い処理を足すだけで、コストが低い。
- 顕在化した問題ではないため High ではないが、すぐに役立つ便利機能として Medium が妥当。

## 関連 issue

- `issues/0035-feature-refactor-e2e-port-discovery.md`: 本機能を活用して e2e-tests/ の 35 箇所の `reserve_ephemeral_port()` workaround を移行する。本 issue マージ後に着手される（0035 は本 issue に依存する側で、本 issue のマージ自体は 0035 の polish 状態と独立に進められる）。本 issue では「使い方の例」を 1 テストファイルだけ追加し、本格的な置き換えは 0035 に委ねる。
- 注意: `startup_info` の出力は `TcpListener::bind()` 成功を保証するが、`accept` ループ到達は保証しない（accept ループは startup_info を書いた後の処理パスで始まる）。listen backlog があるため呼び出し側からの TCP 接続自体は成立するが、HTTP リクエストを送って即座に応答が返るかは別。HTTP 応答 ready を待ちたい場合はポーリングが必要で、これは 0035 で `_wait_until_listening` を置き換える際の前提となる。

## 現状

### 関連コード

- `src/subcommand_server.rs:18-31`: `--host` / `--port` のパース。`--port` のデフォルトは `4455`、`u16` として受けるため `0` も指定可能。環境変数 `HISUI_SERVER_PORT` でも指定可能。
- `src/subcommand_server.rs:153`: `SocketAddr::new(host, port)` を作って `run_internal` に渡す。
- `src/subcommand_server.rs:234, 273`: `run_internal` の中で `crate::obsws::server::run_server(...)` を呼ぶ箇所が **2 箇所** ある（`#[cfg(feature = "player")]` 経路と `#[cfg(not(feature = "player"))]` 経路）。引数を増やすときは両方同時に更新する必要がある。
- `src/obsws/server.rs:179-181`: `TcpListener::bind(addr).await` で bind。失敗時は `crate::Error::new(...)` で `?` 終了。
- `src/obsws/server.rs:182`: bind 後に `tracing::info!("obsws server listening on {scheme}://{addr}")`。**`{addr}` は関数引数の値（`--port 0` のままだと `0`）であり、`listener.local_addr()` の結果ではない。**
- `src/obsws/server.rs:184-186`: `upstream_config.is_some()` のとき `tracing::info!("UI started at {scheme}://{addr}/")`。UI 有効判定はこちらが正しい（`open_ui_in_browser` は別軸 = `--ui && !--no-open`）。
- `src/obsws/server.rs:188`: `open_browser(&format!("{scheme}://{addr}/"))`。これも `addr` 引数の値で、`--port 0` だと壊れた URL になる。
- `src/obsws/server.rs:100-124`: `emit_exit_metrics_to_stdout`。本機能と同じく stdout に JSON Lines を出す既存実装。`nojson::object` + `f.member("type", "metrics")` で 1 行書き、`BrokenPipe` は黙殺・他失敗は `tracing::warn!` で続行する方針。
- `src/logger.rs:124`: `with_writer(std::io::stderr)`。`tracing` の出力先は stderr 固定で、stdout は emit-exit-metrics 以外では未使用。

### 現状の問題点

- ポート 0 を指定すると、ログには `obsws server listening on http://127.0.0.1:0` のように指定値がそのまま出る。
- 呼び出し側が実ポートを知る手段が無く、`netstat` 相当のことを外から推測するか、ポートを固定する以外に方法がない。
- `--port 0` 時のログ表示と `open_browser` の URL も同じ理由で壊れている（`[FIX]` 相当）が、これは本機能の前提として `listener.local_addr()` を取る過程で同時に修正できる。

## 設計方針

### 出力の形式と場所

- 形式: JSON Lines (1 行 1 JSON オブジェクト、改行終端)。`nojson` で生成する。
- 出力先: `std::io::stdout`。ログが stderr に出ているので分離が容易で、テスト側で stdout を読むだけでパースできる。
- タイミング: `TcpListener::bind()` 直後、`listener.local_addr()` を取った時点で出す。それより後の処理（state file 読み込み、MediaPipeline spawn 等）が失敗してもこの 1 行は既に出ているので、呼び出し側は「JSON Lines が来た = bind 成功」と判断できる。
- 保証: `writeln!` の後で必ず `out.flush()` を呼ぶ。Rust の `std::io::Stdout` は `LineWriter` のため `writeln!` 末尾の `\n` で自動 flush される想定だが、改行漏れや後続実装での書き換え事故に備えた冗長安全策として明示する。

### 出力内容（初版）

例 (1): `--ui` なし

```json
{"type":"startup_info","server":{"scheme":"http","url":"http://127.0.0.1:54321","host":"127.0.0.1","port":54321},"pid":12345}
```

例 (2): `--ui` + `--https-cert-path` + IPv6 (`--host ::`)

```json
{"type":"startup_info","server":{"scheme":"https","url":"https://[::]:54321","host":"::","port":54321},"ui":{"url":"https://[::]:54321/"},"pid":12345}
```

フィールド定義:

| キー | 型 | 値 | 備考 |
| -- | -- | -- | -- |
| `type` | string | `"startup_info"` 固定 | stdout JSON Lines のエントリ種別。既存 `--emit-exit-metrics` の `{"type":"metrics", ...}` と同じ規約。JSON 上は文字列のまま、将来 `state_file_info` 等を追加する場合は Rust 側で enum 化する余地を残す |
| `server` | object | bind 情報のサブオブジェクト | ルートに主語のない汎用キーを置かないために名前空間を分離 |
| `server.scheme` | string | `"http"` or `"https"` | `--https-cert-path` 指定時に `"https"` |
| `server.url` | string | `format!("{scheme}://{actual_addr}")` の結果 | IPv6 は `actual_addr.to_string()` が `[::]:54321` のブラケット表記になるため URL としてもそのまま正しい形になる。`server.host` がワイルドカード (`0.0.0.0` / `::`) の場合 URL もそのまま `http://0.0.0.0:54321` 等になるため、接続に使う側で `127.0.0.1` / `::1` 等への置換が必要 |
| `server.host` | string | `actual_addr.ip()` を `nojson` 経由で出力 | `impl DisplayJson for IpAddr`（nojson 0.3.x）が JSON 文字列としてシリアライズする。IPv6 はブラケットなしの `::` 等。`--host 0.0.0.0` / `--host ::` ではワイルドカードがそのまま入る点に注意。IPv6 link-local + zone id 付き bind（`fe80::1%eth0` 等）は zone id を含む URL になり一般のクライアントで扱えないため、初版では非サポートとして扱う |
| `server.port` | number (u16, > 0) | `actual_addr.port()` | `--port 0` 指定時のカーネル割り当て後の実ポート。Linux / macOS では `TcpListener::bind` 成功後の `local_addr()` は必ず割当済みポートを返すため 0 にならない。E2E テストの「ポートだけ取り出す」用途のため URL とは別に持つ |
| `ui` | object（省略可） | UI 有効時のみオブジェクト。未指定時はフィールドごと省略する | 判定は `ui_remote_url.is_some()`（= `--ui` 指定時に Some が入る）。`open_ui_in_browser` ではないので `--ui --no-open` でも `ui` フィールドが出力される |
| `ui.url` | string | `format!("{scheme}://{actual_addr}/")` の結果 | `server.url` に末尾スラッシュを付けたもの |
| `pid` | number (u32) | `std::process::id()` の戻り値 | プロセス全体の情報のためルート直下。シェルラッパーやプロセスマネージャから hisui プロセスを後から特定する用途を想定（必須情報ではないが軽量に追加できる） |

### 常に出力するか / フラグ駆動か

選択肢を 3 つ列挙し、本 issue では **(B)** を採用する。

| 案 | 内容 | 採否 |
| -- | -- | -- |
| (A) 常に出力する | 引数なしで stdout に必ず 1 行出す | 不採用。後方互換性を厳格に取るなら避ける |
| (B) フラグで切り替える | `--emit-startup-info` フラグ（兼 env `HISUI_SERVER_EMIT_STARTUP_INFO`）でオン | **採用**。既存 `--emit-exit-metrics` がフラグ駆動なので規約も揃う |
| (C) 出力先パスをフラグで指定する | `--startup-info-file <PATH>` で書き出す | 不採用。一時ファイル管理コストを hisui に負わせたくない |

フラグ名は `--emit-startup-info`、env は `HISUI_SERVER_EMIT_STARTUP_INFO` とする。

### 既存ログの実 addr ベース化（[FIX] 相当）

本機能の前提として `listener.local_addr()` を取得するので、その値を使って既存の以下を同時に修正する。これは `--port 0` 指定時の表示バグの修正であり、CHANGES.md では `[FIX]` 行として別出しする。

- `src/obsws/server.rs:182` の `obsws server listening on {scheme}://{addr}` を `actual_addr` ベースに差し替え。
- `src/obsws/server.rs:185` の `UI started at {scheme}://{addr}/` を `actual_addr` ベースに差し替え。
- `src/obsws/server.rs:188` の `open_browser(&format!("{scheme}://{addr}/"))` を `actual_addr` ベースに差し替え。

### 書き出し失敗時の方針（既存 emit_exit_metrics_to_stdout との方針差）

- 既存 `emit_exit_metrics_to_stdout`（`src/obsws/server.rs:100-124`）は `BrokenPipe` を黙殺し、他失敗は `tracing::warn!` で続行する。これは「終了処理を妨げない」設計。
- 本機能 `startup_info` は **書き出し失敗時に明示的にエラー終了する**。理由は次のとおり:
  - 起動直後に呼び出し側が stdout を `readline()` でブロックして待っている前提の機能で、書き出しに失敗するとブロックされた側がタイムアウトまでハングする。
  - 起動失敗としてプロセス終了させる方が呼び出し側に状況が伝わりやすい。`crate::Error::new(...)` で `?` 終了すると `noargs::Result<()>` 経由で exit code 1 で終了する。stderr への詳細出力は `main` 終端の表示パス次第のため、呼び出し側が確実に判定したい場合は exit code を見るのが堅い（特に `BrokenPipe` 時は親側 stderr 取得有無も保証できない）。
  - `BrokenPipe` も同様に致命扱いとする（startup_info を期待する呼び出し側が pipe を閉じている時点で連携が壊れているため）。

## 完了条件

- `cargo test` がすべて成功すること。
- `hisui server --port 0 --emit-startup-info` を実行すると、stdout に `{"type":"startup_info", ...}` 形式の 1 行 JSON が即時に出力され、`server.port` / `server.url` にカーネルが割り当てた実ポート番号が反映されていること。
- `--emit-startup-info` を付けない場合、stdout への出力は従来通り無い（後方互換）こと。
- `--port 0` を指定したときの既存ログ (`obsws server listening on ...`, `UI started at ...`) と `open_browser` の URL が実ポートで表示されていること（`[FIX]` 相当の付随修正）。
- `--ui` 指定時には `ui` フィールドにオブジェクトが入り、`ui.url` に実 addr ベースの URL が入ること。`--ui --no-open` でも `ui` フィールドが出力されること。`--ui` 未指定時は `ui` フィールド自体が省略されること。
- `e2e-tests/obsws/test_startup_info.py`（新規）が追加され、`subprocess.Popen` で `hisui server --port 0 --emit-startup-info` を起動し、`stdout.readline()` で 1 行読んで JSON パース、`server.port > 0` を assert、その後 `terminate()` + `wait()` で正常に終了することを検証していること。bind 直後で accept ループ未開始でも、kernel の listen backlog によって TCP 接続自体は成立するため、TCP 接続可能性 (`socket.create_connection`) のみで bind 完了を担保できる（HTTP 応答 ready の検証は不要）。**ここでの「1 例追加」が本 issue のスコープであり、35 箇所の `reserve_ephemeral_port()` の本格的な移行は issue 0035 で行う。**
- CHANGES.md の `## develop` に以下 2 行を追記すること（`shiguredo-changelog` スキル参照）:
  - `[ADD] server サブコマンドに --emit-startup-info を追加する`
  - `[FIX] server サブコマンドの起動ログ・UI URL を bind 後の実アドレスに揃える`

## 解決方法

### 実装ステップ

1. `src/subcommand_server.rs:113-117` 付近（`--emit-exit-metrics` の隣）に `--emit-startup-info` フラグを追加する。`noargs::flag("emit-startup-info").env("HISUI_SERVER_EMIT_STARTUP_INFO").doc("起動直後にバインド情報を JSON Lines で標準出力へ出力する")` の形。`run_internal`（`subcommand_server.rs:176`）に `emit_startup_info: bool` として渡し、`run_internal` から `run_server` に渡す。`run_internal` も既に `#[expect(clippy::too_many_arguments)]` 付きで引数追加でも抑制属性を増やす必要はない。
2. `src/subcommand_server.rs:234` と `:273` の **両方** の `run_server` 呼び出しに、`dump_metrics_on_exit` の隣（player 引数より前）として `emit_startup_info` を追加する。片方を忘れないこと。
3. `src/obsws/server.rs::run_server` のシグネチャに `emit_startup_info: bool` を追加し、`#[expect(clippy::too_many_arguments)]` はそのまま維持する。
4. `src/obsws/server.rs:179-188` 付近で:
   - `let actual_addr = listener.local_addr().map_err(|e| crate::Error::new(format!("failed to get local addr: {e}")))?;` を取得する。
   - 既存出力 3 箇所（L182 / L185 の `tracing::info!` ログと、L188 の `open_browser` 引数 URL）を `addr` から `actual_addr` ベースに差し替える。
   - `emit_startup_info` が true の場合、以下を stdout に書き出す。既存 `emit_exit_metrics_to_stdout`（L100-124）と同じ `nojson::object` パターンで書く。`format_args!` を `f.member` に渡すと `DisplayJson` 不適合でコンパイルできないので、URL は事前に `String` 化する。
     ```rust
     use std::io::Write as _;
     let server_url = format!("{scheme}://{actual_addr}");
     // UI 有効判定は ui_remote_url.is_some()（= --ui 指定時）。open_ui_in_browser ではないので
     // --ui --no-open でも ui フィールドはオブジェクトになる。
     let ui_url: Option<String> = ui_remote_url
         .as_ref()
         .map(|_| format!("{scheme}://{actual_addr}/"));
     let line = nojson::object(|f| {
         f.member("type", "startup_info")?;
         f.member("server", nojson::object(|f| {
             f.member("scheme", scheme)?;
             f.member("url", &server_url)?;
             f.member("host", actual_addr.ip())?;
             f.member("port", actual_addr.port())?;
             Ok(())
         }))?;
         f.member("ui", ui_url.as_ref().map(|url| nojson::object(|f| {
             f.member("url", url)?;
             Ok(())
         })))?;
         f.member("pid", std::process::id())?;
         Ok(())
     });
     let stdout = std::io::stdout();
     let mut out = stdout.lock();
     writeln!(out, "{line}")
         .map_err(|e| crate::Error::new(format!("failed to write startup_info: {e}")))?;
     out.flush()
         .map_err(|e| crate::Error::new(format!("failed to flush startup_info: {e}")))?;
     ```
   - URL を事前に `String` 化する理由は「設計上の留意点」を参照（`format_args!` の `Arguments<'_>` が `Fn` クロージャに閉じ込められない）。
   - `ui` フィールドは `Option<DisplayJson>` を `f.member` に渡せる nojson の仕様を利用する。`ui_url.as_ref().map(|url| nojson::object(...))` の形にすれば `--ui` 指定時のみオブジェクトが入り、未指定時は JSON 上 `null` になる。
   - 書き出しまたは flush に失敗した場合は `crate::Error::new(...)` で `?` 終了する。`BrokenPipe` も同様に致命扱いとする（理由は「書き出し失敗時の方針」参照）。
5. `src/subcommand_server.rs` の `--emit-startup-info` の help 文を簡潔に追加する（`docs/` 配下の更新は本 issue のスコープ外）。
6. `e2e-tests/obsws/test_startup_info.py` を新規追加する。テスト本体:
   - `e2e-tests/hisui_server.py::build_hisui_command` を使って `hisui server --port 0 --emit-startup-info` のコマンドを組み立てる。戻り値は `(cmd, cwd)` の 2 タプルなので、`subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)` のように `cwd` を必ず渡して起動する。`stderr` は `PIPE` で受けて、テスト失敗時に `proc.stderr.read()` を assert メッセージに含めてデバッグ容易性を保つ。
   - bind 失敗等で startup_info の行が出ないまま hisui が exit するケースに備え、`stdout.readline()` には `select.select([proc.stdout], [], [], 5.0)` で 5 秒のタイムアウトをかける。タイムアウト時は `proc.terminate()` + `proc.wait(timeout=2.0)` で確実に殺し、`stderr` 内容を含めて `pytest.fail` させる。これが無いと bind 失敗時に CI で無限ハングする。
   - 取得した 1 行を `json.loads()` でパースし、`body["type"] == "startup_info"`, `body["server"]["port"] > 0`, `body["server"]["url"].startswith("http://")` を assert する。
   - 取得した `server.port` を使って `socket.create_connection(("127.0.0.1", port), timeout=2.0)` で TCP 接続可能なことを確認する。
   - `terminate()` + `wait(timeout=5.0)` で正常終了させる。
   - テスト関数名は `test_emit_startup_info_returns_actual_port` のような意図が読める命名にする（pytest の自動収集に依存）。
   - 既存 `e2e-tests/obsws/test_*.py` で広く使われている `reserve_ephemeral_port()` の置き換え（35 箇所）は本 issue では行わず、issue 0035 で実施する。
7. CHANGES.md の `## develop` に `[ADD]` と `[FIX]` の 2 行を追記する。詳細書式は `shiguredo-changelog` スキルを参照する。

### 設計上の留意点

- `nojson::object<F>` は `F: Fn(&mut JsonObjectFormatter) -> fmt::Result` で `Fn` 制約（`FnOnce` ではない）。複数回呼ばれうる前提で、クロージャは `Copy` 型（`scheme: &'static str`、`actual_addr: SocketAddr` はいずれも `Copy`）と、`String` への共有参照 (`&server_url`, `&ui_url`) を borrow キャプチャする。`src/obsws/state_file.rs::SoraSection::fmt` のネスト `nojson::object` 利用と同じパターン。
- `format_args!()` を `f.member` の値として直接渡すと、`Arguments<'_>` の temporary scope が statement 末尾までしか伸びず `Fn` クロージャに `'_` lifetime を閉じ込められないため、URL は事前に `String` 化する。
- stdout は `lock()` を取って 1 回の `writeln!` + `flush()` で書く。`emit_exit_metrics_to_stdout` も同じパターンを使っているので、コードレビューで揺れないよう参照を残す。
- `tests/` 配下に Rust の統合テストを置かない理由: `hisui server` は終了しないため、`#[tokio::test]` から起動するとシャットダウン制御が複雑になる。常駐サーバを子プロセスとして起動 → stdout を読む → terminate するというフローは `e2e-tests/` の pytest ベース基盤の方が自然。issue 0018（`--emit-exit-metrics`、closed）も同様の判断で単体テストを置かず e2e 寄せにしている。
- 依存追加: `nojson` / `noargs` / `tokio` / `tracing` は本対応で必要な機能をいずれも既存依存で賄えるため、`Cargo.toml` の変更は不要。
- `use std::io::Write as _;` は `run_server` 関数内ローカルに置く（既存 `emit_exit_metrics_to_stdout` (`src/obsws/server.rs:101`) と同じパターン）。モジュールトップに置く必要はない。
