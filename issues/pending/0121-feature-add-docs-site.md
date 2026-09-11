# docs の markdown を Web 配信することを検討する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-docs-site
- Polished:

## 目的

markdown は LLM に扱いやすいが、検索や RAG を考えると Web サイトとして閲覧できると良い。`docs/` 配下の markdown をレンダリングして配信することを検討する。

## pending とした理由

- 静的サイト生成と配信基盤の選定が必要。
- 既存の Cloudflare Pages 基盤 (devtools) を流用できるか、別基盤にするかを決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `docs/` 配下の markdown (`build.md` / `usage.md` / `command_*.md` / `obsws/*.md` / `server/*.md` / `internals/*.md`) は GitHub 上で参照されるのみ。
- デプロイされる静的サイトは `devtools/` の SPA のみで、`.github/workflows/deploy-devtools.yml` が `cloudflare/wrangler-action` で Cloudflare Pages にデプロイする。
- `docs/` の変更をトリガーにするワークフローは無い。
- `--ui-remote-url` の既定は devtools の公開 URL であり、docs とは別。

## 設計方針 (要検討・未確定)

- markdown をレンダリングして静的サイトにする仕組みを追加する。
- 配信先 (Cloudflare Pages など) と、リンクや検索の扱いを決める。
- LLM / RAG から参照しやすい形 (1 ファイルにまとめる等) も検討する。

## 完了条件

- docs が Web で閲覧できること。
- ビルドとデプロイが自動化されていること。

(解決方法は pending のため、設計確定後に記載する。)
