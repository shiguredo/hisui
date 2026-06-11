# e2e-tests のポート確保 workaround を startup_info JSON 経由に移行する

- Priority: Medium
- Created: 2026-06-10
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-e2e-port-discovery
- Polished:

## 目的

issue 0002 で追加される `hisui server --emit-startup-info` を活用し、e2e-tests/ の hisui server 用ポート確保を `--port 0` + stdout JSON 経由の取得に移行する。現状の `reserve_ephemeral_port()` ベースの TOCTOU race window を解消する。

## 優先度根拠

- 現状の `reserve_ephemeral_port()` は Python 側で bind → close → hisui が同 port に bind の流れで、close と hisui bind の間に他プロセスが横取りする race window がある。CI 並列実行での flaky 温床になりうる。
- ただし顕在化した flaky は現状観測されていないため High ではない。
- issue 0002 のマージ後に淡々と消化すべき定石のリファクタなので Medium が妥当。

## 現状

### 関連コード

- `e2e-tests/hisui_server.py:30-35` で `reserve_ephemeral_port()` を定義。`socket.bind(("127.0.0.1", 0))` で空きポートを取り、ソケットを保持したまま `(port, sock)` を返す。呼び出し側で `sock.close()` する必要がある。
- 利用箇所は計 35 箇所（grep ベースの素朴 count、内訳は実装時に分類）:
  - `e2e-tests/obsws/test_connection.py`: 13 箇所
  - `e2e-tests/obsws/test_http.py`: 15 箇所
  - `e2e-tests/obsws/test_output.py`: 7 箇所

### 現状の問題点

- TOCTOU race: Python が `sock.close()` を呼んでから hisui の `TcpListener::bind()` が成立するまでの隙間に、他プロセスが同 port を横取りする可能性がある。
- 戻り値 `(port, sock)` のタプル運用で、呼び出し側に socket 解放責務が漏れている（書き漏らすと leak）。`reserve_ephemeral_port()` がソケットオブジェクトを呼び出し側に晒すこと自体が抽象漏れ。

### スコープ外

- hisui server 以外に渡すポート確保（ダミー RTMP 受信側、FFmpeg のリッスンポート等）は `--emit-startup-info` 経由では取れないため対象外。`test_output.py:48,347,491` 等の `rtmp_port` 確保はこちらに該当し、本 issue では触らない。35 箇所のうち何件が hisui port / 何件がそれ以外かは実装時に分類する。

## 設計方針

- `e2e-tests/hisui_server.py` に context manager 化した新ヘルパー（例: `start_hisui_server_with_dynamic_port()`）を追加する。
  - `subprocess.Popen` で `hisui server --port 0 --emit-startup-info ...` を起動する。
  - stdout を 1 行読み、JSON LINE をパースして `server.port` / `server.url` を取り出す。
  - `__exit__` で `terminate()` + `wait()` までやり、テスト側のリーク責務を消す。
- 35 箇所の hisui port 確保を新ヘルパーに置き換える。RTMP 等の非 hisui 用途は本 issue では残す（必要なら別関数に分離）。
- hisui port 用途の利用が 0 になったら `reserve_ephemeral_port()` 自体を削除する（非 hisui 用途が残る場合は別名で分離してから旧名を消す）。
- hisui を subprocess で起動するヘルパー（`build_hisui_command` および新ヘルパー）は親 env から `HISUI_EMIT_EXIT_METRICS` 等の hisui 共通 env を pop した状態で起動し、開発者がローカルで env を設定して e2e を回した際に subprocess の stdout に終了時メトリクス行が混入してテスト解析が壊れる事故を防ぐ（issue 0025 で共通フラグ昇格に伴い顕在化した課題）。

### 依存

- 本 issue は **issue 0002 のマージ後** に着手する。0002 が入っていないと `--emit-startup-info` 自体が存在しない。

## 完了条件

- `e2e-tests/hisui_server.py` から hisui port 用途の `reserve_ephemeral_port()` 呼び出しが消えていること。非 hisui 用途のみが残る場合は関数名で意図が読み取れること。
- `e2e-tests/obsws/test_connection.py` / `test_http.py` / `test_output.py` の hisui port 確保がすべて新ヘルパー経由になっていること。
- hisui を subprocess で起動するヘルパーが親 env から `HISUI_EMIT_EXIT_METRICS` を pop した状態で起動していること。
- `e2e-tests/` を `uv run pytest` で実行し、全テストがパスすること（ローカル env で `HISUI_EMIT_EXIT_METRICS=1` を設定した状態でもパスすること）。
- CHANGES.md の `## develop` に追記すること（カテゴリは `shiguredo-changelog` スキル参照、`[CHANGE]` または `[REFACTOR]` 相当）。

## 解決方法

### 実装ステップ

1. issue 0002 がマージされていることを確認する。
2. `e2e-tests/hisui_server.py` に新ヘルパーを追加する。`subprocess.Popen` でプロセスを起動し、stdout 1 行をパースして `server.port` を取得。`__enter__` / `__exit__` でプロセス終了まで面倒を見る context manager 化を行う。
3. 35 箇所の `reserve_ephemeral_port()` 利用を hisui port / 非 hisui port に分類し、前者を新ヘルパーに置き換える。
4. hisui port 用途が 0 になった時点で `reserve_ephemeral_port()` 自体を削除（または非 hisui 用途のみ別名で分離）する。
5. CHANGES.md の `## develop` に追記する。

### 留意事項

- 新ヘルパーは startup_info JSON のパース失敗時にプロセスを kill して例外を送出する。stdout に他の行（パニックログ等）が混じるケースを silently に無視しない。
- 戻り値の構造は将来 `ui.url` 等も取り出せるよう拡張可能にしておく（最初から実装する必要はない）。
