# obsws server の player 再生中に SDL シグナルハンドラがグレースフルシャットダウンと競合しないか検証する

- Priority: Low
- Created: 2026-06-05
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

issue 0018 で obsws server に SIGTERM / SIGINT グレースフルシャットダウンと終了時メトリクスダンプ（`--dump-metrics-on-exit`）を追加した。player feature 有効時、SDL のシグナルハンドラが tokio 側のシグナルハンドラと競合し、player 再生中のグレースフルシャットダウン（とメトリクスダンプ）が走らなくなる懸念がある。これを検証し、競合する場合は対策する。

## 優先度根拠

Low。懸念は player 再生中（SDL ウィンドウ表示中）のみで、メトリクスダンプの主用途（CI 診断、`with server: pass` で player 非起動）には無関係。後述のとおり実際には走る可能性が高く、現時点で実害は未確認。

## 現状

- raw_player の SDL 初期化（`raw_player-2026.1.0/src/lib.rs:59` の `SDL_Init`）は `SDL_HINT_NO_SIGNAL_HANDLERS` を設定しない。そのため SDL は POSIX で SIGINT / SIGTERM ハンドラを登録しにいく。
- 一方 obsws server は `src/obsws/server.rs` の `ShutdownSignal::install()` を `run_server` 入口（player 初期化より前）で呼び、tokio のシグナルハンドラを登録する。player 経路では `run_server` は別スレッドの `block_on`（`src/subcommand_server.rs`）で走り、`raw_player::init()`（`src/subcommand_server.rs:312` 付近）はメインスレッドで走る。
- 懸念: player 再生中に SDL のハンドラが tokio のハンドラを上書きすると、SIGTERM が tokio の `ShutdownSignal::recv()`（`run_accept_loop`）に届かず、グレースフルシャットダウンとダンプが走らない。
- ただし SDL2 は慣例上、対象シグナルの現ハンドラが `SIG_DFL` のときだけ自分のハンドラを登録し、既存の非デフォルトハンドラは上書きしない。tokio 側の登録機構も既存ハンドラにチェーンする。よってどちらの登録順でも tokio の `Signal` は発火しうるため、実際には player 再生中でもダンプが走る可能性が高い。この見込みは player を実起動した SIGTERM テスト（ディスプレイ環境が必要）で未検証。

## 設計方針

- 検証: player を実起動（SDL ウィンドウ表示）した状態で SIGTERM を送り、グレースフルシャットダウンとメトリクスダンプが走るかを確認する。SDL2 のシグナルハンドラ登録挙動（`SIG_DFL` のときだけ登録するか）と tokio の登録機構（チェーンするか）、両者のスレッド間タイミングを実コードで裏取りする。
- 競合が確認された場合の対処: `raw_player::init()` の前に `SDL_NO_SIGNAL_HANDLERS=1` 環境変数を設定し、SDL にシグナルハンドラを登録させず tokio のハンドラを唯一の権威にする。設定後、player がグレースフルシャットダウン（サーバ終了 → player 終了の連鎖）で正常終了することを確認する。

## 完了条件

- player 再生中の SIGTERM でグレースフルシャットダウンとメトリクスダンプが走ることが確認できる（または競合せず対策不要と確認できる）。
