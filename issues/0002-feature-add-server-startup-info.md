# server サブコマンドが起動直後に実バインド情報を JSON LINE で出力できるようにする

- Priority: Medium
- Created: 2026-05-29
- Completed:
- Model: Opus 4.7
- Branch: feature/add-server-startup-info
- Polished: 2026-05-29

## 目的

`hisui server` を `--port 0` のようにポート 0 を指定して起動した際に、OS が割り当てた実際のポート番号を呼び出し側（典型的には E2E テストや起動スクリプト）が確実に取得できる手段がない。E2E テストでは固定ポートを使うと並行実行で衝突するため、ポート 0 でカーネル任せに割り当ててから実値を取得したい場面が多い。本対応では、起動直後に実バインド情報を 1 行の JSON (JSON Lines 形式) として出力できるようにする。

## 優先度根拠

- 現状でも CLI の `--port` で固定ポートを指定すれば一応動作するため、業務が止まる類ではない。
- 一方、E2E テストでポート競合を避けるための定石（ポート 0 → 実ポート取得）が踏めず、テストの並行実行性が下がる。CI を回す上で生産性に直結する。
- 実装範囲は `obsws::server::run_server` の冒頭にごく短い処理を足すだけで済み、コストが低い。
- 以上から High ではないが、すぐに役立つ便利機能として Medium が妥当。

## 現状

### 関連コード

- `src/subcommand_server.rs:18-31` (`--host` / `--port` のパース)
  - `--port` のデフォルトは `4455`。`u16` として受けるため `0` も指定可能。
  - 環境変数 `HISUI_SERVER_PORT` でも指定可能。
- `src/subcommand_server.rs:148`
  - `SocketAddr::new(host, port)` を作って `run_server` に渡す。
- `src/obsws/server.rs:120-130`
  - `TcpListener::bind(addr).await` で bind。
  - bind 後は `tracing::info!("obsws server listening on {scheme}://{addr}")` でログ出力。
  - **ここで出ているのは `addr` 引数の値（ポート 0 の場合は `0`）であって、実際にカーネルが割り当てたポート番号ではない。**
  - 実バインド情報を取りたいなら `listener.local_addr()` を呼び出す必要がある。
- `src/logger.rs:120-124`
  - `tracing` の出力先は `std::io::stderr` 固定。stdout は現状未使用。

### 現状の問題点

- ポート 0 を指定した場合、ログには `obsws server listening on http://127.0.0.1:0` のように指定値がそのまま出る。
- 呼び出し側が実ポートを知る手段が無く、`netstat` 相当のことを外から推測するか、ポートを固定する以外に方法がない。
- E2E テストのほか、開発時のスクリプト（`hisui server` を別プロセスで起動し、対向側がエンドポイントを知る必要があるケース全般）でも同じ要件が出てくる。

## 設計方針

### 出力の形式と場所

- **形式**: JSON Lines (1 行 1 JSON オブジェクト、改行終端)。`nojson` で生成する。
- **出力先**: `std::io::stdout`。ログが stderr に出ているので分離が容易で、テスト側で `stdout` を読むだけでパースできる。
- **タイミング**: `TcpListener::bind()` 直後、`listener.local_addr()` を取った時点で出す。それより後の処理が失敗してもこの 1 行は出ているので、呼び出し側は「JSON LINE が来た = bind 成功」と判断できる。
- **保証**: 必ず flush する。プロセス終了直前まで buffered のままだとテストが詰むため、flush ミス対策として `writeln!` + `stdout().flush()` を明示する。

### 出力内容（初版）

例 (1): `--ui` なし

```json
{"type":"startup_info","server":{"scheme":"http","url":"http://127.0.0.1:54321","host":"127.0.0.1","port":54321},"ui":null,"pid":12345}
```

例 (2): `--ui` + `--https-cert-path` + IPv6 (`--host ::`)

```json
{"type":"startup_info","server":{"scheme":"https","url":"https://[::]:54321","host":"::","port":54321},"ui":{"url":"https://[::]:54321/"},"pid":12345}
```

- `type`: stdout JSON Lines のエントリ種別を表す固定キー（初版は `"startup_info"` のみ）。既存の `--dump-metrics-on-exit` が `{"type":"metrics", ...}` を出力しており、本機能の出力もこの規約に揃える。将来別種のエントリを追加する余地を残すため固定キーで持つ。
- `server`: OBS WebSocket リスナーの bind 情報を子オブジェクトでまとめる。ルートに `scheme` / `host` / `port` のような主語のない汎用キーを置くと、将来別主語のフィールドを足したくなった時に衝突するため、最初から名前空間を分離しておく。
  - `server.scheme`: `http` または `https` （`--https-cert-path` 指定時）。
  - `server.url`: `{scheme}://{actual_addr}` を組み立てた完成形 URL（例: `http://127.0.0.1:54321`、IPv6 は `http://[::]:54321`）。呼び出し側に IPv6 ブラケット規則を持たせないために JSON 側で組み立てて持つ。
  - `server.host`: `actual_addr.ip().to_string()`。IPv6 の場合 `::` のような bracket なしの表記。
  - `server.port`: `actual_addr.port()`。`--port 0` 指定時のカーネル割り当て後の実ポート。E2E テストでの「ポートだけ取り出して別用途で使う」需要に応えるため URL とは別に持つ。
- `ui`: `--ui` 指定時のみオブジェクトが入る。未指定なら `null`。`ui != null` を見るだけで UI 有効判定ができる。
  - `ui.url`: UI を開くための完成形 URL（例: `http://127.0.0.1:54321/`）。`server.url` に末尾スラッシュを付けたもの。
- `pid`: 呼び出し側のプロセス制御用 (`std::process::id()`)。プロセス全体の情報なのでルート直下に置く。

### 常に出力するか / フラグ駆動か

選択肢を 3 つ列挙し、本 issue では **(B)** を推奨する。

| 案 | 内容 | メリット | デメリット |
| -- | ---- | -------- | ---------- |
| (A) 常に出力する | 引数なしで stdout に必ず 1 行出す | 呼び出し側が常に同じ手順でパースできる | インタラクティブに使う人にとって stdout に余計な行が混じる、tee / リダイレクト下のテスト出力で目立つ |
| (B) フラグで切り替える | `--emit-startup-info` のようなフラグ（兼 env `HISUI_SERVER_EMIT_STARTUP_INFO`）でオン | デフォルトは互換維持で安全、必要な人だけ on にできる | テスト側がフラグを意識して付ける必要がある |
| (C) 出力先パスをフラグで指定する | `--startup-info-file <PATH>` で書き出す | stdout を汚さない、複数プロセス並行時もファイル衝突を回避しやすい | ファイル I/O が増える、テスト後のクリーンアップが必要 |

- (A) は CLAUDE.md の「後方互換のない変更は feature/change-」に該当しうる。stdout を消費する既存ユーザーは想定しにくいので影響は限定的だが、後方互換性を厳格に取るなら避ける。
- (C) はファイルベースで安定するが、E2E テストのために hisui 側で一時ファイルの面倒を見たくない。
- (B) が最もシンプルで、後で (A) や (C) に切り替えるオプションを残しやすい。

**(B) を採用する前提で本 issue を進める。フラグ名は `--emit-startup-info` とし、env は `HISUI_SERVER_EMIT_STARTUP_INFO` とする。**

### 既存ログとの関係

- `tracing::info!("obsws server listening on {scheme}://{addr}")` の `{addr}` は引数値のままだが、ポート 0 だと誤解を招く。これは JSON 出力と独立に、**`listener.local_addr()` の結果でログメッセージを差し替える** ように合わせて修正する（ログとして人間が読む際にも実ポートが見えた方が便利）。
- 既存の UI URL ログ `"UI started at {scheme}://{addr}/"` も同様に実 addr 表記へ揃える。

## 完了条件

- `cargo test` がすべて成功すること。
- `hisui server --port 0 --emit-startup-info` を実行すると、stdout に `{"type":"startup_info", ...}` 形式の 1 行 JSON が即時に出力され、`server.port` / `server.url` にカーネルが割り当てた実ポート番号が反映されていること。
- `--emit-startup-info` を付けない場合、stdout への出力は従来通り無い（後方互換）こと。
- `--port 0` を指定したときのログ (`obsws server listening on ...`) も実ポートで表示されること。
- `--ui` 指定時には `ui` フィールドにオブジェクトが入り、`ui.url` に実 addr ベースの URL が入ること。`--ui` 未指定時は `ui` が `null` になること。
- CHANGES.md の `## develop` に `[ADD] server サブコマンドに --emit-startup-info を追加する` を追記すること。
- E2E テスト（`e2e-tests/`）で本機能を活用する例を 1 つ追加し、ポート 0 起動 → 実ポート取得 → 接続 までを動作させる。

## 解決方法

### 実装ステップ

1. `src/subcommand_server.rs` に `--emit-startup-info` フラグ（兼 `HISUI_SERVER_EMIT_STARTUP_INFO`）を追加し、`run_server` に `emit_startup_info: bool` として渡す。
2. `src/obsws/server.rs::run_server` 内で:
   - `TcpListener::bind(addr).await` 直後に `let actual_addr = listener.local_addr().map_err(...)?;` を取得する。
   - 既存のログメッセージを `actual_addr` ベースに差し替える。
   - `emit_startup_info` が true の場合、以下を stdout に書き出す。既存の `dump_metrics_to_stdout` (`src/obsws/server.rs:100-124`) と同じ `nojson::object` パターンで書く。
     ```rust
     // `--ui` 指定時のみ ui オブジェクトが入る。未指定なら None を渡して JSON 上は null。
     let ui_value = open_ui_in_browser.then(|| {
         nojson::object(|f| {
             f.member("url", format_args!("{scheme}://{actual_addr}/"))?;
             Ok(())
         })
     });
     let line = nojson::object(|f| {
         f.member("type", "startup_info")?;
         f.member("server", nojson::object(|f| {
             f.member("scheme", scheme)?;
             f.member("url", format_args!("{scheme}://{actual_addr}"))?;
             f.member("host", actual_addr.ip())?;
             f.member("port", actual_addr.port())?;
             Ok(())
         }))?;
         f.member("ui", &ui_value)?;
         f.member("pid", std::process::id())?;
         Ok(())
     });
     let stdout = std::io::stdout();
     let mut out = stdout.lock();
     writeln!(out, "{line}").map_err(...)?;
     out.flush().map_err(...)?;
     ```
   - 書き出しに失敗した場合は明示的にエラー終了する（テスト側がパースに失敗するより、起動失敗の方が明快なため）。
3. `--emit-startup-info` の help 文と、`docs/` 配下に該当する情報を追記する（ドキュメントは本 issue の範囲外として、別途必要なら追記）。
4. テスト:
   - `tests/test_server_startup_info.rs` 相当として、`std::process::Command` で `hisui server --port 0 --emit-startup-info` を起動し、stdout を 1 行読んでパース、`port > 0` を確認する。
   - ただし `hisui server` は終了しないため、子プロセスにシグナルを送って終わらせるテスト基盤が必要。`e2e-tests/` 側にスクリプトを置くのが扱いやすい。
5. ログ差し替えに関する単体テストは追加せず、E2E でカバーする。

### 設計上の留意点

- IPv6 アドレスに対応するため、`server.url` は `format!("{scheme}://{actual_addr}")` で組み立てる（IPv6 では `actual_addr.to_string()` が `[::1]:54321` のように bracket 付きになり、URL としてもそのまま正しい形になる）。`server.host` / `server.port` は bracket なしの分解値を別途持ち、呼び出し側がポート単独取得などで使えるようにする。
- JSON LINE 出力は **bind 成功と起動完了の中間** に出る。具体的には bind 後、`MediaPipeline::run()` を spawn する前。bind までで失敗したら出さない、bind 後の処理が失敗してもこの 1 行は既に出ている、というセマンティクスを README/docs に明文化する。
- stdout は **lock を取って 1 回の `writeln!` で書く**。他の場所で stdout を使っていない想定だが、将来的に競合しないように lock は明示する。
- TLS 経路 (`--https-cert-path` 指定時) でも `local_addr()` は取れるので分岐は不要。
- `--ui` で UI を有効にした場合の `ui.url` は実 addr ベースで組み立てる。`open_ui_in_browser` の URL も既に差し替え対象だが、これは本対応で `actual_addr` ベースに揃える方が一貫する。

### 将来の拡張余地

- `type` フィールドを残しておくことで、起動情報以外のエントリ種別（例えば `"state_file_info"` や `"shutdown_info"`）も同じ JSON LINE 経路で追加できる。
- フラグ駆動 (B) で開始し、運用上ノイズにならないと確認できたら (A) 「常に出力」へ切り替える選択肢も残る。その場合は CHANGES.md で `[CHANGE]` 扱いになる。
