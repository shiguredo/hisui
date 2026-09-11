# 永続化をデフォルトで有効にすることを検討する

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/change-default-persistence
- Polished:

## 目的

obsws の state file による永続化は `--state-file` (または環境変数 `HISUI_SERVER_STATE_FILE`) を指定した場合のみ有効で、未指定時は一切永続化されない。サーバーを再起動するとシーン・入力・出力の設定が失われるため、デフォルトで永続化する方式を検討する。

## pending とした理由

- デフォルトの保存先パスをどこにするか (カレントディレクトリ / ユーザーキャッシュ / 実行ファイル隣接)、書き込み権限が無い場合の扱い、既存利用者への互換性、無効化する手段を用意するかなど、利用環境に影響する設計判断が必要。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/subcommand_server.rs` に `--state-file` (環境変数 `HISUI_SERVER_STATE_FILE`) がある。
- `src/obsws/server.rs` は `state_file_path` が `Some` のときだけ `load_state_file` で読み込み、`src/obsws/state.rs` の `restore_persistent_data` / `get_persistent_data` / `set_persistent_data` と `src/obsws/state_file.rs` の `save_state_file` で読み書きする。
- `src/obsws/coordinator.rs` の `is_state_persisted_request` は成功時に state file を保存する。
- state file が未指定の場合に永続化を行わないことは `docs/obsws/STATE_FILE.md` に明記されている。

## 設計方針 (要検討・未確定)

- デフォルト保存先を決め、`--state-file` 未指定でも読み書きを有効にする。
- 永続化を無効化するフラグ (例: `--no-persist` 相当) を用意するか検討する。
- 保存に失敗した場合 (権限不足・ディスク不足) に起動を止めるか警告に留めるかを決める。

## 完了条件

- `--state-file` を指定しなくても、再起動後に設定が復元されること。
- 既存の `--state-file` 指定時の挙動に回帰がないこと。
- 永続化の有効・無効と保存先がドキュメントに記載されていること。

(解決方法は pending のため、設計確定後に記載する。)
