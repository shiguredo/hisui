# optuna 相当の多目的最適化を自前実装に置き換えることを検討する

- Priority: Medium
- Created: 2026-06-02
- Model: Opus 4.8
- Branch: feature/refactor-replace-optuna-with-builtin-nsga2

## 目的

現状の `hisui tune` サブコマンドは、パラメータチューニングのために外部の `optuna` CLI コマンド (Python 製) に依存している。これを Rust の自前実装に置き換えられないかを検討する。

置き換えによって以下が期待できる:

- 外部 `optuna` バイナリ (Python + optuna パッケージ) の事前インストールが不要になり、ビルド・実行環境のセットアップが単純になる
- `optuna create-study` / `ask` / `tell` / `best-trials` という CLI 実行 → 標準出力の JSON パースという間接的な処理を、ライブラリ内呼び出しに置き換えられる
- 外部プロセス起動・SQLite ファイル経由のやり取りを排除でき、挙動が hisui 内で完結する

これは [[0009-feature-refactor-replace-vmaf-with-vmaf-rs]] と同じ「`hisui tune` / `hisui vmaf` が依存する外部 CLI バイナリ依存を排除する」という方向性の取り組みである。

## 優先度根拠

`hisui tune` は開発・チューニング用途のサブコマンドであり、エンドユーザー向けの合成機能には影響しない。そのため最優先ではない。一方で、Python + optuna という重い外部依存の排除は開発体験とポータビリティを明確に改善するため Medium とする。

## 現状

- `src/optuna.rs` が optuna CLI のラッパーを担う
  - `OptunaStudy::check_optuna_availability()`: `optuna --version` で外部バイナリの存在を確認する
  - `create_study()`: `optuna create-study --directions minimize maximize` で多目的スタディを作成する (合成時間の最小化 + VMAF スコア平均の最大化)
  - `ask()`: `optuna ask --search-space <JSON>` で次に試すパラメータセットを問い合わせる
  - `tell()` / `tell_fail()`: `optuna tell` で試行結果 (成功/失敗) を伝える
  - `get_best_trials()`: `optuna best-trials -f json` で現時点のパレートフロントを取得する
  - ストレージは `sqlite:///<tune_working_dir>/optuna.db` (SQLite ファイル)
- `src/sora/recording_subcommand_tune.rs` が上記を駆動する
  - `ask` → レイアウトへパラメータ適用 → 合成 + VMAF 評価 → `tell` のループを `trial_count` 回繰り返す
  - 各試行後に `get_best_trials()` でパレートフロントの更新を表示する
- 探索空間は `SearchSpace` / `ParameterDistribution` で表現される
  - `Numeric { min, max }`: 整数のみなら IntDistribution、それ以外は FloatDistribution
  - `Categorical(choices)`: CategoricalDistribution

つまり hisui は optuna の多くの機能のうち **2 目的の多目的最適化のみ** を利用しており、分散・並列最適化、可視化、プルーニング、単目的最適化などの機能は使っていない。デフォルトの最適化アルゴリズムは NSGA-II である (optuna は多目的最適化に `NSGAIISampler` を既定で用いる)。

## 設計方針

検討・実装にあたって確認・対応すべき点:

1. **実装するアルゴリズムは NSGA-II のみとする**
   - hisui が使うのは 2 目的 (合成時間 minimize / VMAF 平均 maximize) の多目的最適化だけなので、NSGA-II だけを実装すればよい
   - NSGA-II の構成要素: 非劣ソート (non-dominated sorting)、混雑度距離 (crowding distance)、トーナメント選択、交叉 (SBX 等)、突然変異 (polynomial mutation 等)
   - 探索空間の各分布型 (整数・浮動小数・カテゴリカル) に対する交叉・突然変異の扱いを定める
   - optuna 既定の `NSGAIISampler` のパラメータ (population_size など) と挙動の差異を把握し、必要なら合わせる。完全一致は目的としないが、最適化品質が劣化しないことを確認する

2. **ストレージは JSON Lines でのファイル保存で十分とする**
   - optuna の SQLite ストレージは不要。各試行 (パラメータ・評価値・状態) を 1 行 1 JSON として追記する JSON Lines ファイルで永続化する
   - 既存の `optuna.db` (SQLite) との互換は不要 (フォーマットを変える)
   - 中断・再開 (既存ファイルから過去の試行を読み込んで継続) をどこまでサポートするか方針を決める。現状 `--skip-if-exists` で既存スタディを再利用しているため、同等の挙動が必要かを確認する
   - optuna の分散・並列最適化機能は hisui では不要

3. **乱数の扱い (AGENTS.md 準拠)**
   - NSGA-II は乱数を使う。再現性のためにシードを指定できるようにするか検討する
   - 依存は最小限にすること。乱数ライブラリを追加する場合は用途をコメントで明記し、マイナーバージョンまで指定する

4. **既存インターフェースの維持**
   - `SearchSpace` / `ParameterDistribution` / `Trial` / `TrialValues` / `BestTrial` といった既存の型・概念は可能な限り再利用し、`recording_subcommand_tune.rs` 側の変更を最小化する
   - `OptunaStudy` 相当の構造体 (例: `Study`) が `ask` / `tell` / `tell_fail` / `best_trials` 相当のメソッドを提供する形にできるか検討する
   - 外部プロセス起動がなくなるため `check_optuna_availability()` は不要になる

5. **テスト方針 (AGENTS.md 準拠)**
   - モックやスタブは使わない
   - 非劣ソート・混雑度距離・パレートフロント抽出などの純粋関数は PBT (proptest) で検証する (例: パレートフロント上の解は互いに非劣であるといった不変条件)
   - JSON Lines の保存・読み込みはラウンドトリップを PBT で検証する
   - 任意入力でパニックしないことの検証は fuzzing の役割とする

## 完了条件

- NSGA-II の自前実装による多目的最適化が動作し、`hisui tune` が外部 `optuna` バイナリなしで実行できる
- `src/optuna.rs` の外部プロセス起動 (`Command::new("optuna")`) が削除されている
- 試行履歴が JSON Lines ファイルに永続化される
- 既存の optuna ベース実装と比較して、最適化品質 (得られるパレートフロントの良さ) が明確に劣化しないことを確認している
- NSGA-II の主要ロジックが PBT でカバーされている
- README / docs/command_tune.md / docs/build.md の optuna インストール手順に関する記述が更新されている (ドキュメント自体の編集はこの issue の範囲外だが、依存変更を反映する必要がある旨を記録する)

## 備考

- 自前実装が困難 (NSGA-II の実装コストが見合わない、最適化品質が担保できない等) と判明した場合は、その理由を明記して `issues/pending/` へ移動すること
- 既存の optuna ベース実装と最適化品質を比較する際は、同一の探索空間・同一の合成対象で複数回試行し、得られるパレートフロントを比較すること (NSGA-II は乱数を使うため 1 回の比較では不十分)
- CHANGES.md には外部 `optuna` バイナリ依存の有無の変化を反映する (後方互換に関わるため種別を慎重に選ぶ)
