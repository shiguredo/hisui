# optuna 相当の多目的最適化を自前実装 (NSGA-II) に置き換える

- Priority: Medium
- Created: 2026-06-02
- Model: Opus 4.8
- Branch: feature/change-replace-optuna-with-builtin-nsga2
- Polished: 2026-06-08

## 目的

現状の `hisui tune` サブコマンドは、パラメータチューニングのために外部の `optuna` CLI コマンド (Python 製) に依存している。これを Rust の自前実装 (NSGA-II) に置き換える。

置き換えによって以下が得られる:

- 外部 `optuna` バイナリ (Python + optuna パッケージ) の事前インストールが不要になり、ビルド・実行環境のセットアップが単純になる
- `optuna create-study` / `ask` / `tell` / `best-trials` という CLI 実行 → 標準出力の JSON パースという間接的な処理を、ライブラリ内呼び出しに置き換えられる
- 外部プロセス起動・SQLite ファイル経由のやり取りを排除でき、挙動が hisui 内で完結する

これは [[0009-feature-change-replace-vmaf-with-vmaf-rs]] (closed) と同じ「`hisui tune` / `hisui vmaf` が依存する外部 CLI バイナリ依存を排除する」方向性の取り組みである。

## 優先度根拠

`hisui tune` は開発・チューニング用途のサブコマンドであり、エンドユーザー向けの合成機能には影響しない。そのため最優先ではない。一方で、Python + optuna という重い外部依存の排除は開発体験とポータビリティを明確に改善するため Medium とする。

## 現状

- `src/optuna.rs` が optuna CLI のラッパーを担う (`src/lib.rs:19` で `pub mod optuna;` として公開)
  - `OptunaStudy` が `check_optuna_availability` / `create_study` / `ask` / `tell` / `tell_fail` / `get_best_trials` を提供し、いずれも `Command::new("optuna")` で外部プロセスを起動する
  - ストレージは `sqlite:///<tune_working_dir>/optuna.db` (SQLite ファイル)
- `src/sora/recording_subcommand_tune.rs` が上記を駆動する (`OptunaStudy` の唯一の利用箇所)
  - `ask` → レイアウトへパラメータ適用 → 合成 + VMAF 評価 → `tell` のループを `trial_count` 回繰り返す (既定 `trial_count` は 100)
  - 各試行後に `get_best_trials()` でパレートフロントの更新を確認し、更新時のみ表示する
- 探索空間は `SearchSpace` / `ParameterDistribution` で表現される
  - `Numeric { min, max }`: `min` / `max` がともに整数なら IntDistribution、それ以外は FloatDistribution として扱う (現行 `to_optuna_distribution` の判定規則)
  - `Categorical(choices)`: CategoricalDistribution
- 探索空間例 `search-space-examples/full.jsonc` を見ると、パラメータの過半はカテゴリカル (`["vbr","cbr"]` / `[true,false]` / `[1,2,3]` 等) と整数であり、**連続浮動小数のパラメータは現状存在しない**。交叉・突然変異の設計上、整数・カテゴリカルの扱いが中心課題となる
- hisui は optuna の多くの機能のうち **2 目的 (合成時間 minimize / VMAF 平均 maximize) の多目的最適化のみ** を利用しており、分散・並列最適化、可視化、プルーニング、単目的最適化は使っていない。optuna はこの用途で既定アルゴリズムとして `NSGAIISampler` (NSGA-II) を用いる

## 設計方針

検討の結果、以下の方針で実装する。

### 1. アルゴリズム: NSGA-II のみ

参考にする一次資料は optuna の `NSGAIISampler` の既定挙動と NSGA-II の原論文 (Deb et al., 2002) とする。挙動の完全一致は目的としないが、最適化品質が劣化しないことを確認する (完了条件参照)。

#### 1-1. 世代進行モデル (ask/tell との対応)

hisui の駆動は「1 trial ずつ `ask` → 評価 → `tell`」の逐次形である。これを NSGA-II の世代構造に次のように対応させる:

- **成功 trial (`complete`) のみ**を完了順に並べ、`population_size` 個ずつを 1 世代とみなす (世代 g = 成功 trial 列のインデックス `[g * population_size, (g + 1) * population_size)`)。失敗 trial (`fail`) は世代カウントに一切含めない (採番にのみ用いる。設計方針 5 と一致)
- `ask` 時、これまでに完了した成功 trial 数から、次に生成すべき個体の所属世代を決定する
  - 世代 0 (最初の `population_size` 個) は GA 演算を行わず、各パラメータを範囲内で一様ランダムにサンプリングして生成する (初期集団)
  - 世代 g >= 1 の個体は、**これまでに完了した全成功 trial**を親候補集団とし、非劣ソート + 混雑度距離でランク付けして上位 `population_size` 個を親世代とする。その親世代から binary トーナメント選択 (rank 優先、同 rank は混雑度距離が大きい方を優先) で 2 親を選び、交叉 + 突然変異で 1 子個体を生成する
- 親候補を「これまでの全成功 trial の累積」とすることで、優れた解が後の世代でも淘汰されず親集団に残る (大域最良が保持されるため、elitism と同等に「子が親より劣っても無条件に次世代へ進む」事態を防ぐ)。これは厳密な NSGA-II の世代交代 (直前の親世代とその子世代を結合して生存選択する) とは一致しない、系譜を保存しない制約下での簡略化である。設計方針 1 のとおり optuna との完全一致は目的とせず、品質が劣化しないことを比較で確認する (完了条件参照)
- 親世代の選抜は `ask` のたびに累積成功 trial から再計算する (世代・系譜情報を保存しない。数十〜数百 trial 規模では非劣ソートの毎回再計算コストは無視できる)。再開時 (設計方針 5) もこの再計算で世代位置・親世代が復元されるため、`.jsonl` に世代情報を持たせる必要がない

#### 1-2. ハイパーパラメータ (optuna 既定に準拠)

- `population_size = 20` (定数。CLI 引数での変更は提供しない。必要になった時点で検討する)。optuna 既定 (50) や論文の実験値 (100) より小さめにしているのは、1 試行が高コストなため限られた試行数でも GA フェーズに早く入れるようにするため
- 交叉確率 `crossover_prob = 0.9` (optuna 既定)
- 突然変異確率 `mutation_prob = 1 / パラメータ数` (optuna 既定相当)。ここでの「パラメータ数」はレイアウトの null 箇所で絞り込んだ後の `search_space.params.len()` を指す
- SBX の分布指数および polynomial mutation の分布指数は optuna 既定に合わせる

`population_size` を定数化することの帰結として、`trial_count < population_size` の場合は世代 0 が完成せず GA フェーズに入らないため、全試行が単なる一様ランダムサーチになる (1 trial = 1 回の合成 + VMAF 評価で重く、`trial_count` を数十に絞る運用があり得るため、利用者が NSGA-II の利点を得るには `trial_count >= population_size` が必要)。この旨は CLI ヘルプ / docs に反映する (完了条件参照)。

#### 1-3. 分布型ごとの交叉・突然変異

- 数値 (整数 / 浮動小数): 交叉は SBX、突然変異は polynomial mutation を用いる。整数の場合は SBX / mutation 後に最近傍へ丸め、`min` / `max` の範囲内にクランプする。整数 / 浮動小数の判別は現行 `ParameterDistribution::Numeric` の規則 (両端が整数なら整数扱い) を踏襲し、保存・レイアウト適用時も整数値は整数のまま保つ (エンコーダパラメータが整数型を期待するため)
  - レンジが極小の整数 (実例: `svt_av1_encode_params.tier` は `min 0` / `max 1`、`target_socket` は `min -1` / `max 1`) でも SBX + 丸めをそのまま適用する。両親が同値になりやすく多様性が枯渇しやすいが、`population_size` 固定 (小規模運用) の代償として許容し、特別扱いはしない。突然変異 (一様再サンプリング相当の polynomial mutation) で最低限の多様性を担保する
- カテゴリカル: SBX は適用できない。交叉は uniform crossover (各親のどちらかの選択肢を一様に選ぶ)、突然変異は確率的に選択肢集合から一様再サンプリングする (optuna の categorical 扱いに準拠)

#### 1-4. ドミナンス判定と混雑度距離

- 2 目的の最適化方向は「`elapsed_seconds` は minimize、`vmaf_mean` は maximize」である。ドミナンス判定では内部で方向を統一して扱う (例: `vmaf_mean` を符号反転して両目的を minimize に揃える)。`<` と `<=` の取り違え (タイ・同値の扱い) はバグの温床なので慎重に実装し、PBT で検証する (設計方針 7)
- 混雑度距離は各目的軸を min-max 正規化してから計算する。2 目的のスケール差 (`elapsed_seconds` は秒オーダー、`vmaf_mean` は 0〜100) が選択圧に影響しないようにする。各フロントの端点の距離は無限大とする

### 2. 乱数 (再現性は今回スコープ外)

- NSGA-II は乱数を使うが、再現性 (シード指定) は今回は対応しない。本当に必要になった時点で改めて検討する
- 乱数ライブラリの追加は不要とし、既存コードベースと同じ `aws_lc_rs::rand::fill` を使う (`src/srt/inbound_endpoint.rs` の `pseudo_random_u32` が既存パターン)
- NSGA-II が必要とする乱数操作 (範囲付き整数・浮動小数の生成、確率判定、選択肢の一様選択など) は、`aws_lc_rs::rand::fill` の上に薄いヘルパー関数を被せて実現する。範囲付き整数では素朴な剰余によるモジュロバイアスを避ける (rejection sampling 等を用いる)。負の下限を含む閉区間 `[min, max]` (実例: `intra_period_length` の `min -1`、`cdef_level` の `min -1`) を扱える符号付き整数生成とする (`max - min` のレンジで生成して `min` を加える形)
- CLI へのシード引数の追加は行わない

### 3. ストレージ (単一 JSON Lines ファイル)

- 各試行を 1 行 1 JSON オブジェクトとして追記する単一の JSON Lines ファイルで永続化する
- ファイルパスは `<tune_working_dir>/<study_name>.jsonl` とする
- エントリ種別は 1 種類のみ。1 トライアル完了ごとに 1 行追記する
- 各行に含める情報:
  - `trial_number`: トライアル番号
  - `params`: そのトライアルで適用したパラメータ。キーは `JsonObjectMemberPath` の文字列表現 (ドット結合)、値は `JsonValue`。整数値は整数として保存する (1-3 参照)
  - `state`: `complete` (成功) または `fail` (失敗)
  - `elapsed_seconds` / `vmaf_mean`: 成功時のみ。失敗時は省略する
  - 世代・系譜情報は保存しない (1-1 参照)。`study_name` はファイル名が担うので各行には含めない
- 既存の `optuna.db` (SQLite) との互換は不要 (フォーマットを変える。後方互換破壊として CHANGES.md に記す)
- 分散・並列最適化は非対応。一度に 1 プロセスのみがファイルを更新する前提とする

### 4. ロックファイルによる多重起動防止

- 同一スタディに対する多重起動を簡易的に防ぐ
- ロックファイルパスは `<tune_working_dir>/<study_name>.lock` とする
- 作成は `OpenOptions::new().create_new(true)` でアトミックに行い、起動時に既存ならエラーにする (存在確認 → 作成の TOCTOU を避ける)
- エラーメッセージで「他プロセスが実行中の可能性。終了済みなら手動でこのファイルを削除すること。`.jsonl` は残るので `.lock` を削除すれば再開できる」旨を案内する
- 正常終了時は削除する。削除は `Drop` を実装したガード構造体 (RAII) で行い、途中エラー時にも確実に消えるようにする
- ただし `Drop` は `std::process::exit` やシグナル (SIGINT / SIGTERM) では走らない。`hisui tune` は長時間実行で Ctrl-C 中断が常套のため、中断時はロックが残り次回起動時に手動削除が必要になる。この代償を許容する (シグナルハンドリングは今回スコープ外。残存ロックは上記エラーメッセージの案内で対処する)

### 5. 中断・再開 (合計到達ベース)

- 既存の `.jsonl` があれば過去のトライアルを読み込んで続きから最適化する
- `--trial-count` は「合計でこの件数に到達するまで回す」という意味とする
  - 既存 N 件 + `--trial-count M` のとき、新たに回すのは `M - N` 回 (全エントリ基準。成功・失敗の両方を件数に数える)
  - 既存件数がすでに `--trial-count` 以上の場合、新規トライアルは 0 回でベストトライアル表示のみを行う
- これは optuna の `--skip-if-exists` (新規 trial を都度追加する) とは挙動が変わるが、hisui ではこちらの方が予測しやすく必ず有限回で終わるため採用する。この挙動差は docs / コマンドラインヘルプに明記する
- trial_number の採番は既存の最大 `trial_number + 1` から継続する (番号重複を避ける)
- 再開時、パレートフロント計算・NSGA-II の親集団・世代位置の再計算には成功エントリ (`complete`) のみを使う。失敗エントリ (`fail`) は採番の参照にのみ含める
- 失敗が頻発すると成功 trial が `population_size` に満たないまま `trial_count` を消費し、GA フェーズに入れず全試行がランダムサーチになる可能性がある。この挙動 (失敗は世代を満たさない) を許容する。完了条件の品質比較もこの前提で行う

### 6. 既存インターフェースの維持とモジュール構成

- `SearchSpace` / `ParameterDistribution` / `Trial` / `TrialValues` / `BestTrial` といった既存の型・概念は可能な限り再利用し、`recording_subcommand_tune.rs` 側の変更を最小化する
- `OptunaStudy` 相当の構造体 (例: `Study`) が `ask` / `tell` / `tell_fail` / `best_trials` 相当のメソッドを提供する形にする
  - `best_trials` 相当は、現行 `get_best_trials` の差分検出挙動 (前回のパレートフロントを保持し、`BestTrial` の `PartialEq` で比較して「更新があったか」を `bool` で返す) を維持する。呼び出し側 `display_best_trials_if_updated` がこの `bool` で「更新時のみ表示」を制御しているため
- `Trial.params` (現状 `BTreeMap<JsonObjectMemberPath, JsonValue>`) と `BestTrial.params` (現状 `BTreeMap<String, JsonValue>`) のキー型の非対称は、optuna best-trials JSON のフラットキー由来である。自前化に伴い **内部・保存・表示すべて `JsonObjectMemberPath` キーに統一する** (`BestTrial.params` も `JsonObjectMemberPath` キーに変更)。`recording_subcommand_tune.rs` の表示ループは `Display` 経由なので影響は軽微
  - パスにドットを含むキーは現行 `JsonObjectMemberPath::FromStr` (単純な `split('.')`) が非対応である点は既存制約として踏襲する (レイアウトのキーにドットを含まない前提)
- 2 目的固定 (`TrialValues { elapsed_seconds, vmaf_mean }`、`BestTrial.values` の `[f64; 2]` マッピング) はそのまま維持する。NSGA-II は 2 目的前提で実装してよい
- optuna 固有の要素は削除する:
  - `check_optuna_availability` / `create_study` (外部プロセス起動)
  - `to_optuna_search_space` / `to_optuna_distribution` (optuna 形式 JSON 出力)
  - `Trial` / `SearchSpace` の optuna ask 出力パース用 `TryFrom<RawJsonValue>` (ask が自前化されるため。ただし `SearchSpace` を探索空間ファイルから読む `TryFrom` は維持する)
  - `ParameterDistribution::DisplayJson` は保存・表示に流用できるか実装時に判断する
- ファイル構成は `src/optuna.rs` をディレクトリモジュール `src/tune/` に再編する (役割で分割):
  - 汎用 JSON 値型 (`JsonValue` / `JsonNumber` / `JsonObjectMemberPath` など。optuna 専用ではなくレイアウト JSON 操作用) を独立させる。これらは所有権付きの値型であり、既存の `src/json.rs` (パース/シリアライズ関数群) とは別物である。`src/json.rs` への統合か `src/tune/` 配下のサブモジュール化かは実装時に判断する
  - NSGA-II 本体 (非劣ソート・混雑度距離・選択・交叉・突然変異・世代管理) を分割する
  - ストレージ (JSON Lines の読み書き・ロックファイル管理) を分割する
- `src/lib.rs:19` の `pub mod optuna;` を `pub mod tune;` (ディレクトリモジュール) に変更し、`recording_subcommand_tune.rs:8` の `use crate::optuna::{...}` を修正する。`optuna` を参照するのはこの 2 ファイルのみ
- `recording_subcommand_tune.rs` 側で `optuna` 名称・SQLite 前提が残る箇所を更新する:
  - `storage_url` の `sqlite:///...optuna.db` を `.jsonl` パスへ
  - `run_internal` 冒頭の `check_optuna_availability()?` 呼び出しを削除
  - 表示文言 (`optuna storage:` / `optuna study name:` / `optuna trial count:` / `CREATE OPTUNA STUDY` / `OPTUNA TRIAL` 等) と CLI ヘルプ (`tune` サブコマンドの doc「Optuna を用いた…」、`study-name` の doc「Optuna の study 名」) から optuna 名称を除去する
- 実装のログメッセージは英語、コメントは日本語とする (CLAUDE.md 規約)。現行 `optuna.rs` の英語ログ・日本語コメントを踏襲する

### 7. テスト方針 (CLAUDE.md / shiguredo-rust 準拠)

- モックやスタブは使わない
- PBT (proptest) をこの issue で新規導入する (プロジェクト初の PBT)。closed [[0009-feature-change-replace-vmaf-with-vmaf-rs]] が「PBT 基盤の新設は別 issue 規模」として見送った件を、本 issue が兼ねて導入する判断とする
  - ワークスペースに `pbt` クレートを新設する: `pbt/Cargo.toml` を作成し、`Cargo.toml` の `[workspace] members` に追加する。`proptest` 依存はマイナーバージョンまで指定する (例 `proptest = "1.x"`)。hisui 本体ロジックを参照するため `pbt` は hisui を path 依存で参照する (examples と同じ構成)
  - PBT 対象の関数 (非劣ソート・混雑度距離・パレートフロント抽出・JSON Lines シリアライズ/デシリアライズ) は、別クレート `pbt` から到達できるよう `pub` で公開する
  - PBT は `pbt/tests/prop_tune/main.rs` にサブモジュール対応で配置する
- PBT で検証する不変条件の例:
  - 非劣ソート: rank が小さい解は、より大きい rank の解に支配されない
  - パレートフロント抽出: フロント上の任意 2 解は互いに非劣
  - 混雑度距離: 各フロントの端点の距離は無限大
  - JSON Lines の保存・読み込みのラウンドトリップ (整数値が整数のまま保たれることを含む)
- 同値・タイ・境界 (目的値が等しいケース) でドミナンス判定 (`<` と `<=` の取り違え、min/max 方向の取り違え) のバグが出やすいため、PBT で重点的に検証する
- PBT で実現できないエラーパス・境界値は単体テスト (`tests/test_tune.rs`、長くなる場合はファイル内 `mod` で分割) で補う
  - 現状 `tune` / `optuna` の自動テストは存在しない (`tests/` に該当なし、`e2e-tests/` は Python ユーティリティで無関係)。自前化に伴い単体テスト・PBT を新規追加する価値が高い
- Fuzzing (cargo-fuzz) はこの issue では見送る。NSGA-II の入力は自前生成の数値集団でありパニック耐性の優先度は低い。`.jsonl` パースのエラーパスは単体テスト + ラウンドトリップ PBT で担保する

## エッジケース

実装・テストで考慮する:

- 空探索空間: 既に `recording_subcommand_tune.rs` で `search_space.params.is_empty()` をエラーにしている。新実装でも維持する
- カテゴリカルの選択肢が 1 個 (`nvcodec_av1_encode_params.profile: ["main"]` が実例): 交叉・突然変異が無意味なため固定値として扱う
- `min == max`: サンプリング・SBX が破綻せず常にその値を返すこと
- `min > max`: 不正入力としてバリデーションエラーにする
- レンジが極小の整数 (`max - min` が 1〜2): 多様性枯渇するが許容する (設計方針 1-3)
- `.jsonl` の破損行 (クラッシュ時に最終行が途中で切れる等): パース失敗時の挙動を決める (破損行をスキップして読めるところまで再開するか、エラーで停止するか)。単体テストで挙動を固定する

## 完了条件

- NSGA-II の自前実装による 2 目的多目的最適化が動作し、`hisui tune` が外部 `optuna` バイナリなしで実行できる (`Command::new("optuna")` が全削除されている)
- `src/optuna.rs` が `src/tune/` ディレクトリモジュールに再編され、`pub mod optuna;` / `use crate::optuna::` の参照が更新されている
- 試行履歴が単一の JSON Lines ファイルに永続化され、既存 `.jsonl` からの再開 (合計到達ベース) が動作する
- ロックファイルによる多重起動防止が動作し、正常終了時にガード (`Drop`) で削除される
- `pbt` クレートが新設され、NSGA-II の主要ロジック (非劣ソート・混雑度距離・パレート抽出) と JSON Lines ラウンドトリップが PBT でカバーされている
- 既存の optuna ベース実装と比較して最適化品質が明確に劣化しないことを、定量指標で確認している (備考の比較手順参照)
- optuna 名称・SQLite 前提・`--trial-count` の挙動変更が CLI ヘルプおよび関連 docs (README / docs/command_tune.md / docs/build.md) に反映されている

## 備考

- 自前実装が困難 (NSGA-II の実装コストが見合わない、最適化品質が担保できない等) と判明した場合は、その理由を明記して `issues/pending/` へ移動すること
- 最適化品質の比較手順 (達成判定を再現可能にするため具体化する):
  - 同一の探索空間・同一の合成対象で、optuna ベース実装と自前実装それぞれを `trial_count >= population_size` で複数回 (N >= 5) 試行する
  - 各実行で得られるパレートフロントをハイパーボリューム指標で評価し、N 回の平均で比較する (NSGA-II は乱数を使うため 1 回の比較では不十分)
  - ハイパーボリュームの基準点 (nadir) は、比較対象の全 trial における各目的の最悪値で固定する (基準点依存で値が変わるため両実装で共通化する)
  - ハイパーボリューム計算と比較は使い捨てのスクリプトで行い、製品コード (`src/`) には含めない
  - optuna ベース実装は本 issue で削除されるため、比較は削除前のコミットで行うか、比較時のみ optuna を手動インストールして実施する (恒久的な optuna 依存は復活させない)
- CHANGES.md には以下を `[CHANGE]` (後方互換のない変更) として記載する (手本: closed 0009 の VMAF 置き換えエントリ):
  - VMAF パラメータチューニングの最適化エンジンを外部 `optuna` バイナリから自前 NSGA-II 実装に変更する (Python + optuna の事前インストールが不要になる)
  - 試行履歴ストレージを SQLite (`optuna.db`) から JSON Lines (`<study_name>.jsonl`) に変更する (既存 `optuna.db` は引き継げない)
  - `--trial-count` の意味を「追加試行回数」から「合計到達回数」に変更する
- 関連 issue [[0029-feature-refactor-generic-vmaf-tune]] (open): `vmaf` / `tune` サブコマンドを Sora 録画前提から汎用化し、`tune` サブコマンド本体を `src/sora/` から移動する可能性がある。本 issue が新設する `src/tune/` (最適化エンジン) と、0029 が移動する `tune` サブコマンド本体は名前空間が衝突しうる。番号順では本 issue (0010) を先に着手するため `src/tune/` を最適化エンジン用に確保するが、0029 着手時にモジュール配置 (サブコマンド本体の置き場所・命名) を相互調整すること
