# obsws session の単体テストファイルを機能群ごとに分割する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-split-obsws-session-tests
- Polished:

## 目的

`src/obsws/session/tests.rs` が 4781 行に達して肥大化している。 機能群 (scene / output / stream service / player / text overlay 等) ごとにサブモジュールへ分割し、 ファイル単体の行数を合理的範囲 (目安 500-1500 行) に抑える。

## 優先度根拠

機能的には動作しており即時の影響はない。 ただし以下の負担が継続的に発生する:

- 1 ファイル 4781 行はレビュー時の全体把握が困難
- CI 上の compile 時間が伸びる
- IDE 検索 / コンフリクト解消 / git diff の負荷が高い
- 新しい obsws リクエスト追加時のテストの居場所が不明確

業務影響はないため Low。

## 現状

- `src/obsws/session/tests.rs` 4781 行 (hisui 内で群を抜いて大きい単一ファイル、 次点の `mixer/video.rs` 2184 行と比較して 2 倍超)
- テストは機能群ごとにまとまっており、 自然な分割境界がある:
  - Scene 系 (Create / Remove / List / SetCurrent / Item 等)
  - Output 系 (Create / Remove / Settings / Start / Stop)
  - StreamService 系 (Get / Set / Toggle)
  - Player 系
  - TextOverlay 系 (~830 行)
  - その他 (Input / SceneItem 等)
- 既存の `src/obsws/coordinator/` 配下では `output_*.rs` 等で同種の分割が既に行われており、 同じパターンを `session/tests/` 配下に適用可能

## 設計方針

- `src/obsws/session/tests.rs` を `src/obsws/session/tests/` ディレクトリに変更し、 機能群ごとにサブモジュールに分割する。
- `mod.rs` は採用しない (hisui のスタイル踏襲)。 `src/obsws/session/tests.rs` をエントリポイントにし、 中で `mod scene; mod output; ...` を宣言する形にする。
- 共通ヘルパー (`default_coordinator_handle` / `parse_request_status` / `identify_session` / `create_initialized_coordinator_with_text_overlay` / `process_text_overlay_request` 等) はエントリポイント or 共通サブモジュール (`tests/common.rs` 等) に集約する。
- 各サブモジュールは 500-1500 行を目安にし、 さらに増えそうな機能群は更にサブ分割する。

## 完了条件

- `src/obsws/session/tests.rs` のエントリポイントが概ね < 300 行 (mod 宣言と共通ヘルパーのみ)。
- 各サブモジュールが目安 500-1500 行内に収まる。
- 既存テスト全 pass (`cargo test --all-targets`)。
- `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` も pass。
