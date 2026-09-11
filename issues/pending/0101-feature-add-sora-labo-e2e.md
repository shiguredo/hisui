# Sora を使った E2E テストを追加する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-sora-labo-e2e
- Polished:

## 目的

現在の E2E は ffmpeg やローカルシグナリングを相手にしたものが中心で、実際の Sora を相手にした WebRTC 入出力の確認が無い。`sora_publish` / `sora_source` の回帰を検出するため、Sora 環境を相手にした E2E テストを追加する。

## pending とした理由

- テスト用の Sora 環境・シグナリング URL・認証情報の準備が必要で、CI から常時実行できるかは運用判断が必要。
- 認証情報をリポジトリに残さない形でどう注入するか (GitHub Actions の secrets 等) も決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `e2e-tests/obsws/` は obsws プロトコルの検証が中心。`test_state_file.py` の Sora 永続化テストはダミーのシグナリング URL を使い、実際の接続は行わない。
- `examples/sora_publish` / `examples/sora_source` / `examples/camera_sora_grid` は手動実行のサンプル。
- `.github/workflows/e2e-test.yml` に Sora 接続用の環境変数・シークレットは無い。
- `sora_sdk` は `Cargo.toml` の依存にあり、`src/sora_publisher.rs` (SendOnly) と `src/sora_source.rs` (RecvOnly) が実装されている。

## 設計方針 (要検討・未確定)

- 接続先と認証情報は環境変数経由で与え、値をリポジトリに書かない。
- 常時実行が難しい場合は、手動実行または定期実行のワークフローに分離する。
- 配信 (SendOnly) と受信 (RecvOnly) の双方向を検証する範囲を決める。

## 完了条件

- Sora への SendOnly 配信と RecvOnly 受信が、CI または定期実行で検証できること。
- 認証情報がリポジトリに残らないこと。

(解決方法は pending のため、設計確定後に記載する。)
