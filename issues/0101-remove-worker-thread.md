# libwebrtc の worker_thread 削除に追随する

- Created: 2026-09-15
- Completed: {YYYY-MM-DD}
- Branch: feature/remove-worker-thread
- Polished: {YYYY-MM-DD}

## 目的

libwebrtc の issue 558821261「Deprecate and remove PeerConnectionFactoryDependencies::worker_thread」が実装されて `PeerConnectionFactoryDependencies::worker_thread` と `PeerConnectionFactoryInterface::worker_thread()` が削除された後も hisui をビルドできるようにする。`worker_thread` の利用箇所と API を全て無くすことで、この削除に追随する。

## 現状

- `src/webrtc/factory.rs` の `WebRtcFactoryBundle::new()` が `PeerConnectionFactoryDependencies::set_worker_thread` を呼んでいる。方針 1（専用 worker thread をやめて network thread を使う）の対応後も、network thread を渡すこの行は残る
- `examples/obsws_bootstrap/src/client.rs` の `bootstrap_session()` にも同じ `set_worker_thread` の呼び出しが残る
- 依存は `shiguredo_webrtc = "=0.150.3"`（crates.io）
- libwebrtc の削除系 CL（499302 / 501640 / 501720 / 502000 / 502500 / 502860 / 502940 / 502960）はレビュー中で、`PeerConnectionFactoryDependencies::worker_thread` と `PeerConnectionFactoryInterface::worker_thread()` は将来削除される
- 時雨堂の方針は 2 段階で、本 issue は方針 2（558821261 を実装した libwebrtc をマージした後に worker_thread の利用箇所と API を全て無くす）に対応する。方針 1 は先行 issue で対応する
- 本 issue の前提条件は、webrtc-rs が 558821261 を実装した libwebrtc に追随して `shiguredo_webrtc` をリリースしていることである。そのバージョンは現時点で存在しないため、前提が満たされるまで着手できない

## 設計方針

- `shiguredo_webrtc` を 558821261 を実装した libwebrtc に追随したバージョンへ更新する
- `src/webrtc/factory.rs` の `WebRtcFactoryBundle::new()` と `examples/obsws_bootstrap/src/client.rs` の `bootstrap_session()` から `set_worker_thread` の呼び出しを削除する
- 方針 1 の対応で `worker_thread` を保持するフィールドと worker 用の `Thread` の生成は既に削除されているため、本 issue では残っている `set_worker_thread` の呼び出しを削除する

## 完了条件

- `git grep worker_thread` が issues 以外で 0 件であること
- `shiguredo_webrtc` の依存が更新されていること
- `cargo check --workspace` / `cargo clippy --workspace --all-targets` / `cargo test --workspace` が通ること
- `CHANGES.md` の `## develop` に追記すること
