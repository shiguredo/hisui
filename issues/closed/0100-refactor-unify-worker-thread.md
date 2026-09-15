# WebRTC factory の worker thread を network thread に統一する

- Created: 2026-09-15
- Completed: 2026-09-15
- Branch: feature/refactor-unify-worker-thread
- Polished: {YYYY-MM-DD}

## 目的

WebRTC factory のために専用の worker thread を生成するのをやめ、network thread を worker thread として使う。libwebrtc の issue 558821261「Deprecate and remove PeerConnectionFactoryDependencies::worker_thread」で worker thread が廃止される方針が示されており、専用 worker thread を持つ構成は将来の libwebrtc で維持できなくなる。専用 worker thread を先に無くしておくことで、削除系 CL をマージした libwebrtc への追随を `set_worker_thread` の呼び出しを消すだけの作業にする。

## 現状

- `src/webrtc/factory.rs` の `WebRtcFactoryBundle` が `_network` / `_worker` / `_signaling` の 3 つの `Thread` をフィールドに持つ
- `WebRtcFactoryBundle::new()` が network（`Thread::new_with_socket_server()`）/ worker（`Thread::new()`）/ signaling（`Thread::new()`）を生成して `start()` し、`PeerConnectionFactoryDependencies` に対して `set_network_thread(&network)` / `set_worker_thread(&worker)` / `set_signaling_thread(&signaling)` を呼んでいる
- `examples/obsws_bootstrap/src/client.rs` の `BootstrapSession` と `bootstrap_session()` に同じ処理が重複している。この example は hisui クレートに依存しないが、同じ workspace の member なので CI の `cargo check --workspace` / `cargo clippy --workspace --all-targets` / `cargo test --workspace` でビルドされる
- 生成した `Thread` は、factory が内部で参照し続けるため factory より後に drop されるようフィールドとして保持している。この意図は `BootstrapSession` のフィールドコメントに書かれている
- 依存は `shiguredo_webrtc = "=0.150.3"`（crates.io）。`PeerConnectionFactoryDependencies::set_network_thread` は既に存在する
- libwebrtc の issue 558821261「Deprecate and remove PeerConnectionFactoryDependencies::worker_thread」で worker thread が廃止される。CL 501620「Default worker thread to network thread」と CL 502480「Warn when a distinct worker thread is configured」はマージ済みで、削除系 CL（499302 / 501640 / 501720 / 502000 / 502500 / 502860 / 502940 / 502960）はレビュー中である
- 時雨堂の方針は 2 段階で、本 issue は方針 1（いますぐ実施。専用 worker thread をやめて network thread を使う）に対応する。方針 2（558821261 を実装した libwebrtc をマージした後に worker_thread の利用箇所と API を全て無くす）は後続 issue で対応する

## 設計方針

- `WebRtcFactoryBundle` と `examples/obsws_bootstrap/src/client.rs` の両方で、worker 用の `Thread::new()` と `start()` を削除し、`_worker` フィールドを削除する
- 方針 1 では `set_worker_thread` に network thread を渡す。現在の libwebrtc は `worker_thread` が未設定だと内部で専用スレッドを生成するため、`set_worker_thread` の呼び出し自体は残して network thread を渡す。方針 2 の後続 issue でこの行を削除する
- `_network` / `_signaling` の保持と、フィールド宣言順による drop 順は維持する
- `WebRtcFactoryBundle` と example で同じ変更を行う。example は hisui クレートに依存していないため、共通化は行わず両方に同じ変更を入れる

## 完了条件

- hisui 本体と `examples/obsws_bootstrap` の両方から worker thread の生成（`Thread::new()` と `start()`）と `_worker` フィールドが消えていること
- 両方の `set_worker_thread` に network thread が渡されていること
- `cargo check --workspace` / `cargo clippy --workspace --all-targets` / `cargo test --workspace` が通ること
- `CHANGES.md` の `## develop` に追記すること

## 解決方法

WebRTC factory の worker thread として network thread を使うようにした。

- `src/webrtc/factory.rs` の `WebRtcFactoryBundle` から `_worker` フィールドと worker 用の `Thread::new()` / `start()` を削除し、`set_worker_thread` に network thread を渡すようにした
- `examples/obsws_bootstrap/src/client.rs` の `BootstrapSession` と `bootstrap_session()` も同じ変更を行い、drop 順のコメントを更新した
- `CHANGES.md` の `## develop` に追記した

確認:

- `cargo check --workspace` / `cargo clippy --workspace --all-targets` / `cargo test --workspace` が通ることを確認した
