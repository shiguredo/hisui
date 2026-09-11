# trusted publish で crates.io へ公開する

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/add-trusted-publish
- Polished:

## 目的

タグを打つと GitHub Actions から crates.io へ公開できるようにする。今後 macOS ではビルドできないコーデックライブラリへの対応が増えると、手元でのビルドを前提にせず公開できると楽になる。

## pending とした理由

- crates.io の trusted publishing の設定と、タグ起点の公開ワークフローの追加が必要。
- 公開対象 (hisui 本体のみか、workspace の他 crate を含むか) を決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `.github/workflows/` に crates.io へ公開するワークフローは無く、`cargo publish` の記述も無い。
- PyPI への公開は `pypi-publish.yml` の trusted publishing (`id-token: write` + `pypa/gh-action-pypi-publish`) で行っている。
- `README.md` には crates.io のバッジ、`docs/build.md` には `cargo install hisui` の記述がある。
- `Cargo.toml` に `publish` の指定は無い。

## 設計方針 (要検討・未確定)

- crates.io の trusted publishing を有効化し、タグ push で `cargo publish` する ジョブを追加する。
- 公開前に検証 (ビルド・テスト) を挟むか、リリースワークフローと統合するかを決める。
- workspace 内の他 crate を公開するか (依存の公開状態を確認する) を決める。

## 完了条件

- タグ push で crates.io に公開されること。
- 認証情報がリポジトリに残らないこと。

(解決方法は pending のため、設計確定後に記載する。)
