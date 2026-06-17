# devtools の Playwright e2e で obsdc 疎通テストを追加するか検討する

- Priority: Low
- Created: 2026-06-17
- Completed:
- Model: Opus 4.7
- Branch: feature/add-devtools-obsdc-playwright-e2e
- Polished:

## 目的

devtools (TypeScript + Preact + Vite 構成、`devtools/` 配下) と hisui server (obsws WebSocket / obsdc DataChannel 経由) の疎通を、UI 経由で機械的に検証できる Playwright e2e テストを追加するかを検討する。

本 issue は **追加するか否か / 追加する場合の最小構成** を決める検討フェーズで、実装着手前の意思決定 issue として位置づける。

## 優先度根拠

Low とする。

- 現状 devtools のリリースは未確定、devtools 経由の運用も未本番化のため、検出力の欠如が直近のユーザー影響につながらない。
- 一方、issue 0003 (feature/change-obsws-json-naming) の `settingsKey` snake 化のような UI から送信される JSON のキー名変更が、現状 **手動操作でしか検証できない** ことが判明したため、検討の必要性自体は残る。
- 中長期で devtools が運用上の主要 UI になる可能性があるなら格上げ要検討。

## 現状

### devtools 側のテスト構成

- `devtools/src/**/*.test.ts` (vitest unit test): 10 ファイル存在。内訳は signaling / protocol / auth / client / stats viewer / video layout の単体テスト。
  - `rg -n 'inputSettings|settingsKey|sendObs' devtools/src/` の結果 0 件。obsws / obsdc の settings JSON キー名はテストに登場せず、UI から送信される JSON ペイロードの検証が無い。
- `devtools/playwright.config.ts`: `testDir: "tests"` の設定はあるが、`devtools/tests/` ディレクトリが存在せず e2e テストファイル 0 件。
- `devtools/package.json` に `"test:e2e": "playwright test --project=chromium"` スクリプトはあるが、実体テストが無いため実質的に空回り。

### hisui server 側のテスト構成 (参考)

- `e2e-tests/obsws/` (Python pytest + uv): obsws WebSocket を直接叩く e2e がある (`test_output.py` / `test_state_file.py` / `test_request_batch.py` / `helpers.py` / `conftest.py` 等)。
- ただし devtools UI 経由の往復は対象外。

### 直接的な動機 (issue 0003 由来)

- feature/change-obsws-json-naming で `ObsDcSourcePanel.tsx:698-733` の `settingsKey="..."` を `inputUrl` → `input_url` 等に変更。
- `vp check / build / test` (pre-commit hook) は通過するが、これは TypeScript ビルドと vitest unit test のみで、UI から送信される JSON ペイロードが snake_case で正しく整形されているかは検証していない。
- 完全な疎通確認は devtools 起動 → hisui server 接続 → input/output を実際に作成・編集する手動操作のみ。

## 設計方針

### 検討すべき判断軸

1. **そもそも追加するか**: 検出力 (手動では見逃しがちなキー名揺れ・型不整合の検出) と維持コスト (Playwright 環境構築・hisui server 起動連携・CI 時間) の比較。
2. **追加する場合の最小構成**:
   - hisui server 起動方法 (`cargo run --release -- server` を Playwright `globalSetup` で起動するか、別プロセス管理にするか)
   - vite preview vs dev server: e2e で配信するモードの選定
   - smoke test の範囲 (input 1 種類の作成 + 設定編集 + 削除程度の最小限か、全 input/output kind カバーか)
3. **CI 統合**: 既存 `.github/workflows/E2E Test.yml` (Python pytest 用) に追加するか、別 workflow を立てるか。`browser-actions/setup-chrome` 系のセットアップが必要か。
4. **既存 `e2e-tests/obsws/` (Python pytest) との役割分担**:
   - Python e2e: hisui server を直叩きしてプロトコル準拠を検証
   - devtools Playwright e2e: UI 経由の往復のみ検証 (UI → server → UI の表示反映)
   - 重複を避け、devtools e2e は UI レイヤ特有の不具合 (キー名 typo、フォーム→JSON 変換ミス、設定保存後の再描画ずれ等) に絞る。
5. **維持コスト**:
   - hisui Rust ビルドに依存するため、依存変更で e2e がフレーキーになるリスク
   - Playwright ブラウザのバージョン管理、devtools/node_modules の容量
   - 失敗時のデバッグ難易度 (UI 状態のスクリーンショット保存等)

### 検討に必要な調査項目

- 他の同種プロジェクト (Preact + Playwright + Rust server) のセットアップ事例
- hisui server の起動時間・終了処理の e2e 親和性 (`--state-file` / `--port 0` で並列化可能か)
- devtools 側で Playwright を導入する際の `vp` (`@voidzero-dev/vite-plus-core`) 互換性

## 完了条件

本 issue (検討フェーズ) の完了条件:

- 上記「検討すべき判断軸」1-5 すべてに対する結論を本 issue の `## 結論` セクションに追記する。
- 結論が「追加する」となった場合: 実装範囲・CI 統合方針・テスト粒度を確定したうえで、実装着手用の別 issue を `feature/add-` で起票する (本 issue 自体は検討で close)。
- 結論が「追加しない」となった場合: その判断根拠 (検出力対コスト) を残して close する。手動疎通確認手順を `devtools/README.md` または `docs/obsws/` 配下にメモとして残すか検討する。

## 解決方法

検討フェーズの作業:

1. 上記「検討に必要な調査項目」を 1 件ずつ調査し、本 issue にメモする。
2. devtools の現在のテスト構成 (vitest CT / playwright config) を改めて読み、Playwright 環境の前提を把握する。
3. 最小 PoC として `devtools/tests/smoke.spec.ts` を 1 ファイルだけ書いて挙動を確認する (任意。実装着手前の判断材料として)。
4. 判断軸 1-5 の結論を本 issue に追記し、ユーザーレビューを経て close する。

## スコープ外

本 issue では扱わない:

- e2e の本実装 (検討結論で「追加する」となった場合の実装は別 issue)
- e2e-tests/obsws/ (Python pytest) 側の拡充
- devtools の機能追加・UI 改修

## 関連 issue / 参考

- issue 0003 (feature/change-obsws-json-naming): snake_case 統一に伴い devtools 側の `settingsKey` を snake 化したが、UI 経由の疎通検証手段が無いことが本 issue の起票動機。
- `devtools/playwright.config.ts` / `devtools/package.json` の `test:e2e` スクリプト
- `e2e-tests/obsws/` 配下の Python e2e 構成 (役割分担の参考)
