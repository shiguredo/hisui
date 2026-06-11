# e2e-tests のポート確保 workaround を startup_info JSON 経由に移行する

- Priority: Medium
- Created: 2026-06-10
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-e2e-port-discovery
- Polished: 2026-06-11

## 目的

issue 0002 でマージ済みの `hisui server --emit-startup-info` を活用し、e2e-tests/ の hisui server 用ポート確保を `--port 0` + stdout JSON 経由の取得に移行する。具体的には次の 2 点を同時に解決する。

1. 現状の `reserve_ephemeral_port()` ベースの「Python 側で bind → close → hisui が同 port に bind」フローにある TOCTOU race window を消し、ソケット解放責務漏れの抽象漏れ（`(port, sock)` タプルを呼び出し側に晒している）を解消する。
2. issue 0025 で `--emit-exit-metrics` が hisui 共通フラグへ昇格した副作用として顕在化した「開発者のローカル env `HISUI_EMIT_EXIT_METRICS=1` が e2e の hisui 子プロセス stdout に終了時メトリクス行を混入させ、`ObswsServer.stop()` の `_emit_captured_output()` が出す診断 print を汚染する」事故を防ぐ。pop 対象は本 issue で扱う 2 env（`HISUI_EMIT_EXIT_METRICS` / `HISUI_SERVER_EMIT_STARTUP_INFO`）に限定し、将来 hisui 側が別 env で stdout に行を吐く機能を追加した場合の汎用防衛は本 issue 範囲外とする（非対象節の allowlist 方式 pop 参照）。

## 優先度根拠

- 主動機は issue 0025 で顕在化した env 漏れ事故の根治。ローカル env `HISUI_EMIT_EXIT_METRICS=1` を export した開発者の手元で `ObswsServer.stop()` の `_emit_captured_output()` が出す診断 print が壊れることが既に 1 度発生済み（issue 0025 のマージ直後、`2476f65e` で issue 文書側に追記した経緯）。本 issue で新たに stdout を `_read_startup_info()` で parse する以上、env 制御を整理しないと新ヘルパー自体が常時 flaky 化する。
- 副次的に `reserve_ephemeral_port()` の戻り値 `(port, sock)` を呼び出し側で `sock.close()` する規約に起因する socket leak 設計負債を解消する。現状の利用全てで呼び出し側が `sock.close()` を書いているが、書き漏らした 1 箇所で leak する潜在脆弱性として常に残る。
- 副次的に TOCTOU race window（Python が `sock.close()` を呼んでから hisui の `TcpListener::bind()` が成立するまでの隙間）を消す。並列 e2e 実行時の flaky 温床になりうるが、現時点で顕在化した flaky は観測されていない。
- 着手規模は対象 9 ファイル・約 126 箇所と大きいが、置換パターンは数種類に収束しており単純作業の比率が高い。issue 0002 のマージ後に淡々と消化すべき定石のリファクタなので Medium が妥当。

## 現状

### `reserve_ephemeral_port()` 利用箇所

`e2e-tests/hisui_server.py:30-35` で定義された `reserve_ephemeral_port()` の利用は `e2e-tests/obsws/test_*.py` の 9 ファイル・合計 142 箇所（`grep -hE "reserve_ephemeral_port\(\)" e2e-tests/obsws/test_*.py | wc -l` の実測。import 文を除く）。なお `e2e-tests/obsws/test_startup_info.py` には 0 箇所（issue 0002 で `subprocess.Popen` 直の独立実装になっており本ヘルパーを使わない）。

| ファイル | hisui server 用（置換対象） | 非 hisui（RTMP/SRT 等） | Python 内 socketserver |
| --- | --- | --- | --- |
| `test_bootstrap.py` | 3 | 0 | 0 |
| `test_connection.py` | 13 | 0 | 0 |
| `test_events.py` | 13 (`ws_port`) | 4 (`rtmp_port`) | 0 |
| `test_http.py` | 13 (`port`) | 0 | 1 (`_UpstreamServer.self.port`) |
| `test_hybrid_recording.py` | 2 | 0 | 0 |
| `test_output.py` | 約 19 | 約 11 (RTMP/SRT) | 0 |
| `test_request_batch.py` | 6 | 0 | 0 |
| `test_requests.py` | 31 | 0 | 0 |
| `test_state_file.py` | 26 | 0 | 0 |
| 合計 | 約 126 | 約 15 | 1 |

合計は 126 + 15 + 1 = 142、本文冒頭の grep 実測値と一致する。本 issue の置換対象は表中央の「hisui server 用」列の合計 約 126 箇所。

### 既存の hisui server 起動ヘルパー

`e2e-tests/obsws/helpers.py:29-204` の `ObswsServer` クラスが全ての hisui server 起動を担う。要点:

- コンストラクタ引数 `port: int` で固定 port を受け取り、`start()` 内で `--port {self.port}`（CLI 経路）または `HISUI_SERVER_PORT={self.port}`（env 経路、`use_env=True` 時）として渡す。
- `_wait_until_listening()`（`helpers.py:186-204`）が `socket.create_connection((host, port), timeout=0.5)` をポーリングして listening 完了を待つ（10 秒上限、100ms 間隔）。プロセスが listening 前に exit した場合は `AssertionError("obsws process exited before listening: ...")` を送出する。この文言は `test_state_file.py:289, 307, 325` の 3 箇所で `assert "exited before listening" in str(e)` として依存されている。
- `start()`（`helpers.py:79`）は `env = os.environ.copy()` をそのまま使う。親 env から hisui 共通 env を pop していない。
- `stop()`（`helpers.py:146-163`）は `communicate(timeout=5.0)` で stdout/stderr を読み切り、`_emit_captured_output()` で `[obsws server stdout]` / `[obsws server stderr]` を print する（pytest 失敗時の診断用）。
- `--emit-exit-metrics` を CLI 末尾に append する経路（`helpers.py:131-132`）と `HISUI_EMIT_EXIT_METRICS=1` を env に積む経路（`helpers.py:99-100`）を持つ。両方とも `self.emit_exit_metrics=True`（default）で有効化される。

呼び出し側の URL 構築には 2 系統のパターンがある:

- パターン A（外側 `host` / `port` 変数経由）: 大多数。例えば `test_state_file.py:107` `ws_connect(f"ws://{host}:{port}/", ...)` のように、`with ObswsServer(... port=port, ...):` の `with` ブロック内でもローカル `host` / `port` 変数を URL に埋め込む（`as server:` 句が付かない呼び出しも多い）。`test_state_file.py` ではこのパターンが 20 箇所以上ある（`port2` を含む再起動シナリオも同パターン）。`test_bootstrap.py` の `_build_bootstrap_command(host, port, ...)`（`test_bootstrap.py:18, 121, 207, 304`）も `port` 変数を引数として渡す。
- パターン B（`server.host` / `server.port` 経由）: 少数。`test_http.py` の各テストと `test_output.py::_collect_obsws_metrics_snapshot` で使われる。

`--port 0` 動的化後は port が `__enter__` 後にしか確定しないため、パターン A は全件「`as server:` 句を追加し、URL を `server.host` / `server.port` 参照に書き換える」差分が必要。

### `test_startup_info.py`

`e2e-tests/obsws/test_startup_info.py` は issue 0002 で `--emit-startup-info` の動作確認用に追加された 4 テスト。`subprocess.Popen` 直 + `select.select([stdout], [], [], 10.0)` でタイムアウト付き readline + `json.loads()` + `terminate()` + `communicate()` のパターンが完成しており、本 issue の `ObswsServer` 改修でも基本骨格として流用する。`test_startup_info.py` 自体は `--emit-startup-info` フラグの挙動確認が目的のため、本 issue で `ObswsServer` 経由には載せず subprocess.Popen 直のままとする。

### env 漏れの現状

`ObswsServer.start()` は親 env を `os.environ.copy()` で丸ごと継承する。開発者がローカルで `HISUI_EMIT_EXIT_METRICS=1` を export した状態で e2e を走らせると、`stop()` が読む subprocess stdout に `{"type":"metrics", ...}` の終了時メトリクス行が混入し、`_emit_captured_output()` の診断 print を汚染する。さらに、本 issue で「stdout の 1 行目を JSON parse して startup_info を取り出す」実装を入れる以上、将来 hisui 側で起動時 stdout に別の行を出す機能が追加された場合に readline が壊れる。env を pop して明示制御に揃えることでこの将来リスクも同時に塞ぐ。なお現実装の `--emit-exit-metrics` は **プロセス終了時** のみ stdout に書くため、起動直後の readline では混入しない（exit metrics 行と startup_info 行のタイミングは独立）。pop の主動機は `stop()` の `_emit_captured_output()` 出力汚染と将来防衛である。

### スコープ外

- 非 hisui port 確保（RTMP outbound/inbound、SRT outbound/inbound、ffmpeg listen）: 計 約 15 箇所。`--emit-startup-info` 経由では取れないため対象外。`reserve_ephemeral_port()` の関数自体を残す。
- `test_http.py:60-74` の `_UpstreamServer.__init__`（1 箇所）: Python 製ダミー HTTP upstream サーバの bind 用。hisui プロセスとは無関係なので触らない。
- `test_startup_info.py` の 4 テストの `ObswsServer` 化。
- 非 hisui port 用の `reserve_ephemeral_port()` を allowlist 方式の `HISUI_*` 全 env pop に拡張すること。

## 設計方針

### 1. `ObswsServer` を改修する

新ヘルパーを `hisui_server.py` に追加する案も検討したが、約 126 箇所の hisui server 用途の大半が `ObswsServer(binary_path, host=host, port=port, ...)` の形で `ObswsServer` 経由になっているため、新ヘルパー追加方式では呼び出し側 126 箇所全ての差し替えが必要になる。`ObswsServer` 内部を改修するアプローチを採れば、呼び出し側は `reserve_ephemeral_port()` / `sock.close()` / `port=port` 引数の 3 箇所を削るのに加えて、後述の URL 書き換えを行うだけで済む。

採用方針:

- `ObswsServer` のコンストラクタ引数 `port: int` を削除する。`start()` 内では常に `--port 0 --emit-startup-info`（CLI 経路）または `HISUI_SERVER_PORT=0` + `HISUI_SERVER_EMIT_STARTUP_INFO=1`（env 経路、`use_env=True` 時）で起動する。動的 port のみをサポートし、固定 port 経路は完全に廃止する。
- `_wait_until_listening()` を廃止し、`_read_startup_info()`（設計方針 3 参照）で置き換える。startup_info JSON の出力は `TcpListener::bind()` 直後 = TCP 接続自体は kernel listen backlog で成立する時点なので、現行 `_wait_until_listening()` の `socket.create_connection` 判定と等価。`src/obsws/server.rs:218` の `emit_startup_info_to_stdout` 呼び出し後、同タスク内で state file 読み込み・MediaPipeline 初期化・coordinator 起動が順次走り、`run_accept_loop`（`src/obsws/server.rs:386`）に到達するまでに数十 ms から数百 ms の遅延がある。WebSocket / HTTP の応答可能化はこの accept ループ到達まで待たされるが、これは現行の `socket.create_connection` 判定でも同じであり、本 refactor で挙動は悪化しない（呼び出し側の `aiohttp.ClientTimeout(total=10.0)` 等のリクエストタイムアウト内に accept ループは確実に到達する）。`test_http.py:319` の生 TCP RST テストは TCP listen backlog 成立で十分機能要件を満たす。

### 2. env pop と env 再設定

`ObswsServer.start()` の冒頭で `env = os.environ.copy()` した直後・`use_env` の if/else 分岐より前に、以下を実施する。

```python
env = os.environ.copy()
env.pop("HISUI_EMIT_EXIT_METRICS", None)
env.pop("HISUI_SERVER_EMIT_STARTUP_INFO", None)
# ここから use_env の if/else 分岐
```

pop 対象を 2 件に限定する根拠: 本 issue で `ObswsServer` が stdout を直接 parse / 印字する経路は、(a) 起動直後の `_read_startup_info()` での 1 行 readline、(b) `stop()` の `_emit_captured_output()` での全 stdout 印字、の 2 つ。これらが汚染されうる env は「hisui が stdout に行を出す既存仕様」に対応する `HISUI_EMIT_EXIT_METRICS` と、本 issue で同フラグを CLI 経路から明示的に渡す前提のため env 重複を排除する `HISUI_SERVER_EMIT_STARTUP_INFO` の 2 件に集約される。それ以外の `HISUI_SERVER_HOST` / `HISUI_SERVER_PORT` / `HISUI_SERVER_PASSWORD` 等は hisui の挙動を変えるが stdout には行を吐かないため pop しない。

pop 後の再設定方針:

- CLI 経路（`use_env=False`、`helpers.py:101-132` の else 節）: `--emit-startup-info` を args 末尾に append する（`args.append("--emit-startup-info")` を `helpers.py:131-132` の `--emit-exit-metrics` append の直前に置く）。`--emit-exit-metrics` は既存どおり `self.emit_exit_metrics` で append。env 側は pop しっぱなしで再設定しない。
- env 経路（`use_env=True`、`helpers.py:81-100` の if 節）: pop 後に `env["HISUI_SERVER_PORT"] = "0"` と `env["HISUI_SERVER_EMIT_STARTUP_INFO"] = "1"` を立てる。`self.emit_exit_metrics=True` のときは `env["HISUI_EMIT_EXIT_METRICS"] = "1"` も立てる（既存挙動の維持）。`args.append("--emit-startup-info")` は呼ばない（env 経路の args は host/port を含まずスタブ的な内容で、CLI フラグを混入させない）。

### 3. startup_info の読み取り

`ObswsServer.start()` から `_wait_until_listening()` を削除し、`_read_startup_info()` 相当の private メソッドに置き換える。実装方針:

- `select.select([self._process.stdout], [], [], startup_timeout)` でタイムアウト付き readline する。`select.select` は POSIX 限定で、e2e の CI は Linux/macOS のみで運用されており Windows は対象外。
- `readline()` がブロックする可能性は、hisui 側が startup_info JSON を 1 回の `writeln!` + `flush()` で出力するため発生しない（`src/obsws/server.rs` の `emit_startup_info_to_stdout` 実装が 1 行 1 write を保証している）。
- `startup_timeout` は **60.0 秒** とする。`e2e-tests/hisui_server.py:11-27` の `build_hisui_command` は `cargo run --quiet --bin hisui` 経由で起動するため、ビルドキャッシュ未温下でのコールドスタートを許容する。`test_startup_info.py` 側の 10 秒値は本 issue では変更しない（独立した subprocess.Popen 経路のため）。
- `os.environ.get("HISUI_E2E_STARTUP_TIMEOUT")` を直接読んで override 可能にする（子プロセスへ渡す `env` ではなく Python 側の挙動制御のため `os.environ` から読む）。値は秒数（float）でパースし、`float()` 失敗時は `ValueError` を素通しさせる（CI 環境設定ミスを silent に default に戻すと診断しづらくなるため）。未設定なら default 60.0。
- 失敗時の `AssertionError` 文言は次の 3 種類に分ける:
  - **プロセス exit 由来**（`readline()` で EOF / `select` タイムアウト後に `self._process.poll() is not None` で exit 検知）: `"obsws server exited before startup_info: ..."`。state_file 破損以外の起動失敗（bind error 等）で startup_info を出さずに exit したケースの診断用。
  - **タイムアウト + プロセス生存（hang）**（`select` タイムアウト後に `self._process.poll() is None` で生存確認）: `"obsws server startup_info timeout: ..."`。CI 等で hisui が startup_info を出さないまま hang したケースの診断用。
  - **プロセス生存 + parse 失敗**（`json.loads()` 失敗、`type != "startup_info"`、`server.port` 不在）: `"obsws server startup_info invalid: ..."`。プロセスは生存しているので「exited」表現を避けて診断性を保つ。
- 失敗時の共通フロー:
  1. `self._process.terminate()` で SIGTERM 送信
  2. `self._process.communicate(timeout=2.0)` で stdout/stderr を読み切る
  3. （timeout した場合）`self._process.kill()` で SIGKILL → 再度 `communicate()`
  4. `_emit_captured_output()` で診断 print
  5. 上記文言いずれかで `AssertionError` 送出
- `server.port` を取り出して `self.port` に上書きする（parse 成功時のみ。失敗パスでは `self.port = 0` のまま）。`self.host` は呼び出し側が渡した値をそのまま保持する。
- `ObswsServer` に `wait_for_exit(timeout: float = 5.0) -> int | None` メソッドを追加する。`self._process.wait(timeout=timeout)` をラップし、timeout 内に exit したら returncode を返し、しなければ None を返す。後述の state_file 破損テスト書き換えで使う。

#### state_file 破損テストの書き換え

`src/obsws/server.rs:218-219` の `emit_startup_info_to_stdout` は state_file load（`:223-256` の `load_state_file`）よりも **前** で実行される（issue 0002 closed の明示的な設計判断: 「bind 後の state file 読み込み等が失敗してもこの 1 行は既に出ているので、呼び出し側は『JSON Lines が来た = bind 成功』と判断できる」）。本 issue ではこの設計を尊重し、hisui 本体側は触らない。

その結果、state_file 破損ケース（`test_state_file.py:283-289, 303-307, 321-325` の 3 テスト）では:

1. startup_info JSON が正常に stdout 出力される
2. `_read_startup_info()` は成功して `start()` が return
3. その後 state_file load が失敗してプロセスが exit

という順序になる。本 issue ではこの 3 テストの assertion 構造を「`start()` 内で `AssertionError` が出るのを期待」から「`start()` 後に短時間内のプロセス exit を検出する」に書き換える。

Before（`test_state_file.py:283-289` 等の典型）:

```python
try:
    with ObswsServer(binary_path, host=host, port=port, state_file=state_file):
        assert False, "server must not start with corrupted state file"
except AssertionError as e:
    assert "exited before listening" in str(e)
```

After:

```python
with ObswsServer(binary_path, host=host, state_file=state_file) as server:
    returncode = server.wait_for_exit(timeout=5.0)
    assert returncode is not None, "server should have exited due to corrupted state file"
    assert returncode != 0, f"unexpected returncode {returncode}"
```

`_wait_until_listening()` が送出していた `"obsws process exited before listening: ..."` 文言は廃止する（新文言 `"exited before startup_info"` には依存テストが残らない）。3 テストの assertion はプロセス exit と returncode で起動失敗を直接検証する形になり、`_read_startup_info()` の文言マッチには依存しない。

### 4. URL 構築タイミングの変更（呼び出し側）

`port` が `ObswsServer.__enter__()` 後にしか確定しなくなるため、呼び出し側で URL を組み立てている箇所のうち、外側 `host` / `port` 変数を使っているものを `server.host` / `server.port` 参照に書き換える。

Before（パターン A、`test_state_file.py:107` 等の典型）:

```python
host = "127.0.0.1"
port, sock = reserve_ephemeral_port()
sock.close()

with ObswsServer(binary_path, host=host, port=port, use_env=False) as server:
    async with aiohttp.ClientSession() as session:
        ws = await session.ws_connect(f"ws://{host}:{port}/", protocols=[OBSWS_SUBPROTOCOL])
```

After:

```python
host = "127.0.0.1"

with ObswsServer(binary_path, host=host, use_env=False) as server:
    async with aiohttp.ClientSession() as session:
        ws = await session.ws_connect(f"ws://{server.host}:{server.port}/", protocols=[OBSWS_SUBPROTOCOL])
```

Before（パターン B、`test_http.py` 等の `server.port` 経由）:

```python
port, sock = reserve_ephemeral_port()
sock.close()

with ObswsServer(binary_path, host=host, port=port, use_env=False) as server:
    status, _, _ = asyncio.run(_http_get(f"http://{server.host}:{server.port}/.ok"))
```

After:

```python
with ObswsServer(binary_path, host=host, use_env=False) as server:
    status, _, _ = asyncio.run(_http_get(f"http://{server.host}:{server.port}/.ok"))
```

`test_state_file.py` の `port2` での再起動シナリオ（例: `test_state_file.py:130, 173` 等）も同じパターンで、各 `with ObswsServer(...) as server2:` ブロックで `server2.port` を都度参照する。`grep "port2.*reserve_ephemeral_port" e2e-tests/obsws/test_state_file.py` の全 13 箇所は新規取得のみで「同じ port を意図的に再利用するシナリオ」は無い（polish 時に grep で確認済み）ため、動的 port 化で支障は出ない。

`test_bootstrap.py` の `_build_bootstrap_command(host, port, ...)` を呼ぶ箇所（`test_bootstrap.py:121, 207, 304`）も、`with ObswsServer(...) as server:` ブロック内で `_build_bootstrap_command(server.host, server.port, ...)` に書き換える。なお `_build_bootstrap_command` の関数本体（`test_bootstrap.py:18-59` 周辺の `--port str(port)`）は obsws_bootstrap 側の「接続先サーバ port」を指定する引数であり hisui server の起動 CLI ではないため、関数本体は無変更で渡される `port` 値が動的化されるだけで完結する。`helpers.py::_collect_obsws_metrics_snapshot(host, port)` も同様で、呼び出し側 8 箇所（`test_bootstrap.py:127, 216, 314`、`test_output.py:457, 595, 730, 1203, 1342`）のみ `server.host` / `server.port` 渡しに書き換え、関数定義側は無変更。

### 5. CHANGES.md の扱い

e2e-tests/ のみの内部リファクタで、外部から観察可能な hisui の挙動（CLI、env、stdout、テストの合否）は何も変わらない。`shiguredo-changelog` 規約（`/Users/tohta/.claude/skills/shiguredo-changelog/SKILL.md`）の「機能に直接影響しない変更は `### misc` サブセクションに記載する」に従い、`CHANGES.md:237` 周辺に既存の `## develop` 配下 `### misc` サブセクションに 1 行追記する。`[CHANGE]/[ADD]/[UPDATE]/[FIX]` の独立エントリは追加しない。env pop 追加分も同じ develop 内の中間状態整理に当たるため独立エントリを立てない。

## 実装ステップ

中間状態でビルド・テストが壊れないよう以下の順で進める。

1. `ObswsServer` を改修する。同コミット内で次を全て実施する。中間状態でテストが落ちないよう、本ステップではコンストラクタ引数 `port` を `port: int | None = None` のオプショナル形に変え、受け取った値は黙殺する（後方互換 shim）。ステップ 2 で全呼び出し側の `port=port` 引数を消した後、ステップ 3 で `port` 引数自体を完全削除する:
   - コンストラクタ引数を `port: int | None = None` に変える。受け取った値は使わず無視する。`__init__` で `self.port: int = 0` を初期化する（実 port は `_read_startup_info()` の parse 成功時に上書きされる）。
   - `start()` 内で常に `--port 0 --emit-startup-info`（CLI 経路）または `HISUI_SERVER_PORT=0` + `HISUI_SERVER_EMIT_STARTUP_INFO=1`（env 経路）で起動する。
   - `start()` 内で env pop 2 件（`HISUI_EMIT_EXIT_METRICS` / `HISUI_SERVER_EMIT_STARTUP_INFO`）を `os.environ.copy()` 直後・`use_env` 分岐前に実施する。
   - `_wait_until_listening()` を `_read_startup_info()`（設計方針 3）に置き換える。
   - `wait_for_exit(timeout: float = 5.0) -> int | None` メソッドを追加する。
   - `test_state_file.py:283-289, 303-307, 321-325` の 3 テストを設計方針 3 末尾の After 例に従い書き換える（`assert "exited before listening" in str(e)` の文言依存をやめ、`server.wait_for_exit(timeout=5.0)` でプロセス exit と returncode を直接検証する形にする）。
2. 1 ファイルずつ呼び出し側を書き換える。1 ファイル 1 コミットで段階的に進める。各ファイルで次を実施する:
   - `reserve_ephemeral_port()` / `sock.close()` の 2 行削除。
   - `ObswsServer(... port=port, ...)` から `port=port` 引数の削除。
   - `with ObswsServer(...):` を `with ObswsServer(...) as server:` に変える（`as server:` 句が無いと `server.host` / `server.port` 参照ができない）。`test_state_file.py` の `port2` 再起動シナリオは `as server2:` 等の別名を使う。
   - 外側 `host` / `port` 変数を URL に埋め込んでいる箇所を `server.host` / `server.port` 参照に書き換える。
   - `_build_bootstrap_command(host, port, ...)` のような外部関数呼び出しは `_build_bootstrap_command(server.host, server.port, ...)` 等に書き換える。
   - 順序は `test_connection.py`（13 箇所、最小規模で動作確認しやすい）→ `test_http.py`（13 + `_UpstreamServer` は残す）→ `test_bootstrap.py`（3、`_build_bootstrap_command` への port 渡し変更を含む）→ `test_hybrid_recording.py`（2）→ `test_request_batch.py`（6）→ `test_events.py`（13、RTMP の `rtmp_port` 4 箇所は残す）→ `test_output.py`（19、RTMP/SRT 11 箇所は残す）→ `test_state_file.py`（26、`port2` 含む URL 書き換え）→ `test_requests.py`（31）。
3. ステップ 1 で残した `port: int | None = None` を完全に削除して `start()` を動的 port 専用に整理する。
4. `e2e-tests/hisui_server.py:30-35` の `reserve_ephemeral_port()` は非 hisui 用途で残るため、関数名と docstring を維持する。リネームは別 issue で扱う。
5. `CHANGES.md` の `## develop` 配下の `### misc` サブセクションに 1 行追記する。

## 完了条件

- `e2e-tests/obsws/test_*.py` 全 9 ファイル（`test_startup_info.py` を除く）から hisui server 用途の `reserve_ephemeral_port()` 呼び出しが消えていること（grep で実数確認）。非 hisui 用途（RTMP/SRT/`_UpstreamServer`）は残ること。
- `e2e-tests/obsws/helpers.py::ObswsServer` の `port` 引数が削除され、`start()` 内部で `--port 0 --emit-startup-info` 起動 + stdout JSON readline で実 port を取得する経路に統一されていること。CLI 経路と env 経路（`use_env=True`）の両方で動的 port が機能すること。
- `ObswsServer.start()` で `env = os.environ.copy()` した直後・`use_env` 分岐前に `env.pop("HISUI_EMIT_EXIT_METRICS", None)` と `env.pop("HISUI_SERVER_EMIT_STARTUP_INFO", None)` を呼び、CLI 経路では再設定せず（`--emit-startup-info` を args 経由で明示）、env 経路では `HISUI_SERVER_EMIT_STARTUP_INFO=1` を立て直していること。
- startup_info JSON 取得失敗時に `_emit_captured_output()` で診断 print した上で、プロセス exit 由来は `"obsws server exited before startup_info: ..."`、タイムアウト + プロセス生存（hang）は `"obsws server startup_info timeout: ..."`、プロセス生存 + parse 失敗は `"obsws server startup_info invalid: ..."` を `AssertionError` で送出していること。
- `ObswsServer.wait_for_exit(timeout: float = 5.0) -> int | None` メソッドが追加され、`test_state_file.py:283-289, 303-307, 321-325` の 3 テストが設計方針 3 末尾の After 例に従いプロセス exit と returncode 直接検証の形に書き換わっていること。
- `e2e-tests/` を `uv run pytest` で実行し、全テストがパスすること（ローカル env で `HISUI_EMIT_EXIT_METRICS=1` を設定した状態でもパスすること）。
- `test_startup_info.py` の 4 テストが引き続き subprocess.Popen 直の独立実装のままで通ること（`ObswsServer` 経由にしない）。
- `CHANGES.md` の `## develop` 配下の `### misc` サブセクションに「e2e-tests の hisui server 起動を `--port 0` + `--emit-startup-info` 経由の動的 port 取得に統一する」相当の 1 行を追加すること。独立した `[CHANGE]/[ADD]/[UPDATE]/[FIX]` エントリは追加しない。

## テスト戦略

- 既存 e2e 全件（`uv run pytest e2e-tests/`）がパスすることで `ObswsServer` 改修後も挙動が保たれていることを検証する。
- 「ローカル env `HISUI_EMIT_EXIT_METRICS=1` を設定した状態でも全件パスすること」をステップ 1 完了直後に手動で 1 度確認する。CI には組み込まない。
- `ObswsServer.start()` の `_read_startup_info()` 失敗パス（プロセス即 exit、`select` タイムアウト、JSON parse error、`type != "startup_info"`）は単体テストを追加しない。state_file 破損ケース（`test_state_file.py:283-289` 等の 3 テスト）は startup_info 出力後に exit するため EOF パスを通らず、本 issue 後はこれらの失敗パスを自然にカバーするテストは存在しない。失敗パスは診断用文言として実装するが、注入テストでカバーするコストは見合わない。

## 非対象

- 非 hisui port 確保（RTMP outbound/inbound、SRT outbound/inbound、ffmpeg listen）の動的化。
- `test_http.py:60-74` `_UpstreamServer.__init__` の `reserve_ephemeral_port()` 利用（1 箇所）。
- `test_startup_info.py` の 4 テストの `ObswsServer` 化。
- `HISUI_*` 全 env の allowlist 方式 pop への拡張。

## 単一目的チェック

本 issue は `feature/refactor-` カテゴリで起票している。リファクタリングと別目的作業の混在を以下の通り整理する。

- ポート確保 refactor（主目的）と env pop 追加（副次）は技術的に密結合: `ObswsServer.start()` の `_read_startup_info()` で stdout を JSON parse する経路と、`stop()` の `_emit_captured_output()` で stdout を診断 print する経路の両方が、`HISUI_EMIT_EXIT_METRICS` 由来の終了時メトリクス行に汚染される。env pop 無しで本 refactor を成立させると、ローカル env を設定した開発者の手元で常時 `stop()` の診断出力が壊れる。
- TOCTOU race window 解消は refactor の副次効果であり、独立した「バグ修正」ではない（顕在化していないため）。
- カテゴリは `feature/refactor-` 一本に収まる。env pop も `### misc` 行きの内部リファクタとして同一カテゴリ。

## 関連

- `issues/closed/0002-feature-add-server-startup-info.md`: 依存元。本 issue が利用する `--emit-startup-info` フラグを追加。0002 closed の「bind 後の state file 読み込み等が失敗してもこの 1 行は既に出ているので、呼び出し側は『JSON Lines が来た = bind 成功』と判断できる」設計判断を本 issue でも維持する（state_file 破損テストはテスト側書き換えで対応する。設計方針 3 末尾参照）。
- `issues/closed/0025-feature-refactor-common-exit-metrics-flag.md`: env 漏れ事故の原因となった `--emit-exit-metrics` の共通フラグ昇格。本 issue で env pop を入れる動機の源。
