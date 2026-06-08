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

これは [[0009-feature-change-replace-vmaf-with-vmaf-rs]] と同じ「`hisui tune` / `hisui vmaf` が依存する外部 CLI バイナリ依存を排除する」という方向性の取り組みである。

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

検討の結果、以下の方針で実装する。

1. **実装するアルゴリズムは NSGA-II のみとする**
   - hisui が使うのは 2 目的 (合成時間 minimize / VMAF 平均 maximize) の多目的最適化だけなので、NSGA-II だけを実装する
   - NSGA-II の構成要素: 非劣ソート (non-dominated sorting)、混雑度距離 (crowding distance)、トーナメント選択、交叉 (SBX 等)、突然変異 (polynomial mutation 等)
   - 探索空間の各分布型 (整数・浮動小数・カテゴリカル) に対する交叉・突然変異の扱いを定める
   - optuna 既定の `NSGAIISampler` のパラメータ (population_size など) と挙動の差異を把握し、必要なら合わせる。完全一致は目的としないが、最適化品質が劣化しないことを確認する

2. **乱数の扱い (再現性は今回スコープ外)**
   - NSGA-II は乱数を使うが、再現性 (シード指定) は今回は対応しない。本当に必要になった時点で改めて検討する
   - そのため乱数ライブラリの追加は不要とし、既存コードベースと同じ `aws_lc_rs::rand::fill` を使う (例: `src/srt/inbound_endpoint.rs` の `pseudo_random_u32`)
   - NSGA-II が必要とする乱数操作 (範囲付き整数・浮動小数の生成、確率判定など) は、`aws_lc_rs::rand::fill` の上に薄いヘルパー関数を被せて実現する
   - CLI へのシード引数の追加は行わない

3. **ストレージは単一の JSON Lines ファイルとする**
   - optuna の SQLite ストレージは不要。各試行を 1 行 1 JSON オブジェクトとして追記する単一の JSON Lines ファイルで永続化する
   - ファイルパスは `<tune_working_dir>/<study_name>.jsonl` とする
   - 各行のエントリ種別は 1 種類のみ。1 トライアル完了ごとに 1 行追記する
   - 各行に含める情報:
     - `trial_number`: トライアル番号
     - `params`: そのトライアルで適用したパラメータ
     - `state`: `complete` (成功) または `fail` (失敗)
     - `elapsed_seconds` / `vmaf_mean`: 成功時のみ。失敗時は省略する
     - `study_name` はファイル名が担うので各行には含めない (冗長なため)
   - 既存の `optuna.db` (SQLite) との互換は不要 (フォーマットを変える)
   - 分散・並列最適化は非対応。一度に 1 プロセスのみがファイルを更新する前提とする

4. **ロックファイルによる多重起動防止**
   - 並列最適化非対応のため、同一スタディに対する多重起動を簡易的に防ぐ
   - プロセス起動時に `<tune_working_dir>/<study_name>.lock` (仮) の存在を確認し、存在したらエラーにする
     - エラーメッセージで「他プロセスが実行中の可能性。終了済みなら手動でこのファイルを削除すること」を示唆する
   - 存在しなければ作成し、プロセス終了時に削除する
   - 削除は `Drop` を実装したガード構造体 (RAII) で行い、途中エラー時・パニック時にも確実に消えるようにする
   - 異常終了 (クラッシュ・kill) では残存しうるが、それは手動削除で対応する前提とする (シンプルさ優先)

5. **中断・再開 (合計到達ベース)**
   - 既存の `.jsonl` があれば過去のトライアルを読み込んで続きから最適化する
   - `--trial-count` は「合計でこの件数に到達するまで回す」という意味とする
     - 既存 N 件 + `--trial-count M` のとき、新たに回すのは `M - N` 回 (全エントリ基準。成功・失敗の両方を件数に数える)
     - 既存件数がすでに `--trial-count` 以上の場合、新規トライアルは 0 回でベストトライアル表示のみを行う
   - これは optuna の `--skip-if-exists` (新規 trial を都度追加する) とは挙動が変わるが、hisui ではこちらの方が予測しやすく必ず有限回で終わるため採用する
     - optuna との挙動差は docs / コマンドラインヘルプに明記する
   - trial_number の採番は既存の最大 `trial_number + 1` から継続する (番号重複を避ける)
   - 再開時、パレートフロント計算・NSGA-II の初期集団には成功エントリ (`complete`) のみを使う。失敗エントリ (`fail`) は採番の参照にのみ含める

6. **既存インターフェースの維持とモジュール構成**
   - `SearchSpace` / `ParameterDistribution` / `Trial` / `TrialValues` / `BestTrial` といった既存の型・概念は可能な限り再利用し、`recording_subcommand_tune.rs` 側の変更を最小化する
   - `OptunaStudy` 相当の構造体 (例: `Study`) が `ask` / `tell` / `tell_fail` / `best_trials` 相当のメソッドを提供する形にする
   - 外部プロセス起動がなくなるため `check_optuna_availability()` は削除する
   - ファイル構成は、`src/optuna.rs` をディレクトリモジュール `src/tune/` に再編する (役割で分割):
     - 汎用 JSON 値型 (`JsonValue` / `JsonNumber` / `JsonObjectMemberPath` など。optuna 専用ではなくレイアウト JSON 操作用) を独立させる
     - NSGA-II 本体 (非劣ソート・混雑度距離・選択・交叉・突然変異・集団管理) を分割する
     - ストレージ (JSON Lines の読み書き・ロックファイル管理) を分割する
   - `optuna` という名称はモジュール・型・CLI ヘルプ・表示文言から取り除く (storage / study 表示なども JSON Lines パスベースに更新する)

7. **テスト方針 (AGENTS.md / shiguredo-rust 準拠)**
   - モックやスタブは使わない
   - PBT (proptest) をこの issue で新規導入する (プロジェクト初の PBT)。ワークスペースに `pbt` クレートを追加し、PBT は `pbt/tests/prop_<module>/main.rs` にサブモジュール対応で配置する
   - PBT で検証する不変条件の例:
     - 非劣ソート: rank が小さい解は、より大きい rank の解に支配されない
     - パレートフロント抽出: フロント上の任意 2 解は互いに非劣
     - 混雑度距離: 端点の距離は無限大
     - JSON Lines の保存・読み込みのラウンドトリップ
   - 同値・タイ・境界 (目的値が等しいケース) でドミナンス判定 (`<` と `<=` の取り違え等) のバグが出やすいため、PBT で重点的に検証する
   - PBT で実現できないエラーパス・境界値は単体テスト (`tests/test_<module>.rs`) で補う
   - Fuzzing (cargo-fuzz) はこの issue では見送る。NSGA-II の入力は自前生成の数値集団でありパニック耐性の優先度は低い。`.jsonl` パースのエラーパスは単体テスト + ラウンドトリップ PBT で担保する

## 完了条件

- NSGA-II の自前実装による多目的最適化が動作し、`hisui tune` が外部 `optuna` バイナリなしで実行できる
- `src/optuna.rs` の外部プロセス起動 (`Command::new("optuna")`) が削除され、モジュールが `src/tune/` に再編されている
- 試行履歴が単一の JSON Lines ファイル (`<tune_working_dir>/<study_name>.jsonl`) に永続化される
- 既存の `.jsonl` からの再開 (合計到達ベース) が動作する
- ロックファイルによる多重起動防止が動作し、ガード (`Drop`) で確実に削除される
- 既存の optuna ベース実装と比較して、最適化品質 (得られるパレートフロントの良さ) が明確に劣化しないことを確認している
- NSGA-II の主要ロジックが PBT でカバーされている (`pbt` クレートを新設)
- `--trial-count` の合計到達ベースの挙動 (optuna との差異) が docs / コマンドラインヘルプに反映されている
- README / docs/command_tune.md / docs/build.md の optuna インストール手順に関する記述が更新されている (ドキュメント自体の編集はこの issue の範囲外だが、依存変更を反映する必要がある旨を記録する)

## 備考

- 自前実装が困難 (NSGA-II の実装コストが見合わない、最適化品質が担保できない等) と判明した場合は、その理由を明記して `issues/pending/` へ移動すること
- 既存の optuna ベース実装と最適化品質を比較する際は、同一の探索空間・同一の合成対象で複数回試行し、得られるパレートフロントを比較すること (NSGA-II は乱数を使うため 1 回の比較では不十分)
- CHANGES.md には外部 `optuna` バイナリ依存の有無の変化を反映する (後方互換に関わるため種別を慎重に選ぶ)
