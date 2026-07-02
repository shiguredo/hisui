# tune サブコマンドで既定パラメーターを初期集団のシードとして投入できるようにする

- Priority: Medium
- Created: 2026-07-02
- Completed:
- Model: Opus 4.7
- Branch: feature/add-tune-seed-default-params
- Polished:

## 目的

hisui の `tune` サブコマンドは NSGA-II による多目的最適化を行うが、初期集団 (`POPULATION_SIZE = 20`) を一様ランダムサンプリング (`nsga2::sample_random`) だけで生成しており、「対象エンコーダーの現在の既定パラメーター」を初期個体としてシード投入する経路が無い。

そのため、履歴が空の状態から探索を始めると 20 個体分の評価が「既定値より悪い」可能性があり、CI や短時間 tune のように探索予算が限られる状況ほど無駄が大きい。既定値を初期個体として明示的に投入できるようにすれば、パレートフロントは常に「既定値以上」から始まる。

依存ライブラリの更新後に既定値見直しを行うワークフローとも噛み合う。旧既定値をシードとして投入して tune を回せば、「更新後にそのパラメータが依然としてパレートフロント上に残るか」「他の個体に支配されて更新の余地が出たか」が結果を見るだけで判断できる。

## 優先度根拠

- 依存ライブラリ更新時の既定値見直しに直接効く。手元 tune / CI tune (別 issue 0074 の想定シナリオ) の双方で効果があり、実装なしでも tune は回るので High ではないが、後回しにするほど「毎回シードを手動で埋め直す」手間が積み上がる。
- CI で tune を回す場合 (0074) は世代数・個体数を絞る前提であり、初期集団の質が結果の質に直結する。
- 実装ボリュームは中程度 (Tuner の ask 分岐 + CLI 引数 + シード JSON バリデーション + テスト) で、単独 issue として扱う価値がある。
- 以上から Medium 妥当。0074 とは実装依存はしないが、0074 と併用したときに恩恵が大きい。

## 現状

### 初期集団のサンプリング経路

- `src/tune.rs` `Tuner::ask`: 成功トライアル数が `POPULATION_SIZE` (= 20) 未満のときは `nsga2::sample_random` で一様ランダムサンプリング、以降は `nsga2::generate_child` で交叉 + 突然変異。
- `src/tune/nsga2.rs` `sample_random`: `SearchSpace` の各パラメータを一様ランダムに振る。既定値を混ぜる仕組みは無い。
- `src/tune/rng.rs`: 乱数は `aws_lc_rs` 直呼びで、シード指定は今回スコープ外の旨がコメントに明記されている (乱数シードは本 issue のスコープではない。ここでの「シード」は初期集団への既定値注入を指す)。
- 履歴 JSONL に既存の成功トライアルがある場合はそれが暗黙のシードとして機能するが、「初回起動時に既定値を確実に 1 個体投入する」経路は存在しない。

### tune サブコマンドの入出力

- `src/sora/recording_subcommand_tune.rs` `Args`: `--layout-file` / `--search-space-file` / `--name` / `--trial-count` などはあるが、シード投入用オプションは無い。
- `run_internal`: 対象 layout を読み込み、search-space から「layout template 内で `null` になっているエントリ」だけを残す (`search_space.params.retain(|path, _| matches!(path.get(&layout_template), Some(JsonValue::Null)))`)。したがって、tune が扱う探索対象パラメータは「layout template で null のパス集合」に一意に定まる。
- `Tuner::new` (`src/tune.rs`): ワーキングディレクトリ内の `<name>.jsonl` を読んで既存履歴があれば `next_trial_number` を続きから採番する。

### 既定パラメーターの実体

- hisui 内の「既定パラメーター」は `src/sora/recording_layout_encode_params.rs` の `LayoutEncodeParams::default()` として集約されている (`src/encoder.rs::default_video_encode_config_for_rpc` はこれを経由して RPC 既定値を返す)。
- `layout-examples/tune-*.jsonc` は「探索したいフィールドを `null` にした tune 用テンプレート」であって「既定パラメーターセット」ではない (固定値は書かれているが、探索対象は明示的に `null` になっている)。したがって「シードとしての既定値」は tune-*.jsonc からは取り出せない (取り出そうとすると null になる)。
- 「シード JSON をユーザーがファイルで渡す」経路にするか、「コード側 default から自動抽出する」経路にするかは設計方針で選ぶ (下記)。

### 履歴とロック

- `src/tune/storage.rs`: JSONL 追記 + ロックファイル。シード投入分もトライアル 1 件として通常通り記録すれば、既存の履歴形式・ロック仕組みには変更不要。

## 設計方針

### 基本方針

1. **シードは「初期集団の 1 個体目」として投入する**。残り 19 個体は既存どおりランダム。探索多様性を著しく損なわないための安全策。複数個体のシード投入 (2 個以上) は将来拡張として本 issue の対象外にする。
2. **シードの供給元は「ユーザー指定の JSON ファイル」を優先案とする**。
    - 案 (a): `--seed-params <PATH>` で明示的にユーザーが渡す。
    - 案 (b): コード側の `LayoutEncodeParams::default()` から自動抽出する。
    - まずは (a) を実装する。実装が薄く、「旧既定値をシードにして更新影響を見る」ユースケースにも対応できる (旧既定値の JSON を残しておけば再現可能)。(b) はコードとの結合が強く、`JsonObjectMemberPath` 経路との互換維持コストが増す。(b) が必要になれば別 issue で追加する。
3. **シード JSON のフォーマットは search-space と同じ flat 形式 (`"path.to.param": value`) を優先候補にする**。search-space との対応が明快で、tune が既に扱っている `JsonObjectMemberPath` をそのまま使い回せる。layout と同じネスト形式にする案もあるが、そちらは "flatten してから search-space と突き合わせる" ステップが増える。実装着手時にどちらにするかを最終決定する。
4. **search-space の範囲外・型不整合はエラーで即座に弾く**。silently clamp すると「探索空間の外にシードが居座る」異常状態になり、後続の突然変異でもその範囲外に留まる可能性がある。Categorical で選択肢外の値が指定された場合も同様にエラー。
5. **search-space に含まれていないパスがシード JSON にあれば警告して無視する**。layout template で null になっていない = 固定値扱いのパスは、tune 側で触らないのが正しいので、シードに書かれていても無視するのが安全。ただし黙って捨てると意図の取り違いを検出できないので `tracing::warn!` でログを出す。
6. **既存履歴があるときはシード投入をスキップする**。`<name>.jsonl` に成功トライアルが 1 件以上ある場合は、シード指定があってもそのまま既存履歴を続ける (履歴があるということは既に初期集団が構築されている / 続きから最適化する意図であるはず)。スキップした旨は `tracing::info!` で必ず表示する (ユーザーがシード指定したのに使われなかったことを気付けるように)。
7. **`storage.rs` は変更しない**。シード個体もトライアル 1 件として通常どおり `ask` → 評価 → `tell` の流れに乗せる。JSONL 上は他のトライアルと区別せず記録する (state = complete、params = シード値)。区別が必要になれば別 issue で `source` フィールド等を検討する。

### `Tuner::ask` の分岐設計

- `Tuner` にシード params (`Option<BTreeMap<JsonObjectMemberPath, JsonValue>>`) と "シード投入済みフラグ" を持たせる。
- `ask` の先頭で「シード指定あり + 累積成功トライアルが 0 + シード未投入」の条件を満たすときだけ、シード params を返し「投入済み」に切り替える。
- それ以外は既存の分岐 (成功個体数 < POPULATION_SIZE ならランダム、以降は交叉 + 突然変異) に流す。
- 累積成功トライアル数の判定を優先することで、既存履歴を読んだ直後 (`Tuner::new` 実行後) にシードが二重投入される事故を防ぐ。

### 対象外 (別 issue に切り出す想定)

- 複数個体のシード投入 (2 個以上、あるいは「シード + そのバリアント」の投入)。
- コード側 `LayoutEncodeParams::default()` からの自動抽出 (上記案 (b))。
- 既存履歴がある場合でも強制的にシードを再投入するオプション。
- パレートフロントから最終 1 点を選ぶ自動選択。
- 乱数シード指定による探索の再現性確保 (`src/tune/rng.rs` の TODO)。

## 完了条件

- `hisui tune --seed-params <PATH>` オプションが追加され、指定時に初期集団の 1 個体目としてシード params が投入されること。
- シード JSON の値が search-space の範囲外・型不整合・Categorical の選択肢外の場合はエラーで起動時に弾かれ、エラーメッセージから該当パスと理由が読めること。
- search-space に含まれないパスがシード JSON に書かれていた場合は警告ログを出したうえで無視すること。
- 既存履歴 (`<name>.jsonl`) に成功トライアルが 1 件以上ある場合はシード投入をスキップし、スキップした旨をログ出力すること。
- 以下の観点をカバーするユニットテストが `src/tune.rs` (もしくは新規 `src/tune/seed.rs` を作る場合はそこ) に追加されること:
    - `ask` の 1 回目の返却値がシード params と一致すること。
    - 既存履歴があるときはシード指定を無視して既存経路を通ること。
    - 範囲外・型不整合を起動時 (もしくは `ask` 前) に検出できること。
    - シード JSON に search-space 外のパスが混ざっているケースが警告扱いになること (ログ出力の検証は必須ではない)。
- `docs/command_tune.md` に `--seed-params` の使い方と、「旧既定値をシードとして投入して更新影響を見る」推奨ワークフローが記載されていること。
- `CHANGES.md` の `## develop` に `[ADD] hisui tune サブコマンドに --seed-params オプションを追加する` を追記すること。

## 解決方法

### 実装ステップ

1. **シード JSON のパース・検査ロジック追加**:
    - `SearchSpace` に対してシード JSON を検査するユーティリティ関数を追加する。Numeric なら `min <= value <= max`、Categorical なら選択肢に含まれること、型 (整数 / 浮動小数 / bool / 文字列) が `ParameterDistribution` と一致することをチェックする。
    - 範囲外・型不整合は具体的なパスと理由を含むエラーで返す。search-space に無いパスは警告扱い。
    - フォーマット (flat か nested か) は着手時に決定 (方針候補は上記の通り)。

2. **`Tuner` の拡張**:
    - シード params (`Option<BTreeMap<JsonObjectMemberPath, JsonValue>>`) を保持できるようにする (`Tuner::new` の引数として受け取るか、`Tuner::set_seed_params` として後付けするかは実装時に判断)。
    - `ask` の先頭で「累積成功トライアル 0 + シード未投入」の条件下でのみシード params を返す分岐を追加する。
    - シード投入済みフラグは `Tuner` の状態として持ち、`ask` 呼び出し内でフラグを立てる。
    - `Tuner::new` の既存挙動 (履歴読み込み・`next_trial_number` 採番) は変更しない。

3. **CLI (`src/sora/recording_subcommand_tune.rs`) への統合**:
    - `Args` に `--seed-params <PATH>` を追加する (`noargs::opt("seed-params").ty("PATH")` 相当)。デフォルトは未指定。
    - `run_internal` でパース + 検査 (Step 1) を呼び出し、結果を `Tuner` に渡す。
    - シード指定時に search-space の絞り込み (layout の null 判定) を先に済ませてから検査することで、絞り込みで探索対象外になったパスを警告扱いにできる。
    - 開始時に表示する INFO セクションに「seed params file」の行を追加する。

4. **既存履歴によるスキップ判定**:
    - `Tuner` 側で「累積成功トライアル 0」を判定基準にする。`tell_fail` 経由の失敗トライアルは「初期集団が確立されていない」状態とみなしてシード投入を許容する (シード自体は失敗しない前提の値のはず)。この判定基準を Tuner のドキュメントコメントに明記する。
    - スキップ時はログ (`tracing::info!`) で「skip seed injection because history already has successful trials」の旨を出す。

5. **ユニットテスト**:
    - `src/tune.rs` (もしくは分離するなら新規モジュール) にテストを追加する。ワーキングディレクトリは `tempfile::TempDir` などで作り、実 JSONL を書いて挙動を検証する (モック・スタブは AGENTS.md 規約で禁止)。
    - テストのログメッセージは日本語 (`AGENTS.md` 準拠)。
    - コメントは日本語。テストの意図が読み手に伝わるようにする。

6. **ドキュメント整備**:
    - `docs/command_tune.md` に `--seed-params` の項を追加する。「旧既定値を JSON として保存しておき、依存更新後に tune を回すときに投入するワークフロー」を推奨手順として書く。
    - シード JSON のフォーマット例と、範囲外・型不整合時のエラー挙動を明記する。

7. **`CHANGES.md` の `## develop` 追記**:
    - `[ADD] hisui tune サブコマンドに --seed-params オプションを追加する`。

### リスク・留意点

- シードが局所解の近傍にあると、そこから抜けにくくなる恐れ。ただし残り 19 個体はランダム初期化のままなので影響は限定的。将来的にシード N 個体 (N > 1) の投入を許すときは、多様性維持の設計を再検討する。
- シード JSON のフォーマット (flat vs nested) は決めきりで進める。両対応は実装コストと利用者側の混乱を招く。
- `crate::json` の解析器が jsonc (コメント可) に対応しているかは実装着手時に確認する。対応していれば `--seed-params` にも jsonc を許すのが自然。対応していなければ純 JSON に限定する。
- `layout-examples/tune-*.jsonc` は探索対象を `null` にした「テンプレート」であり、これをそのままシードとして渡すと `null` を検出してエラーになる。この旨をドキュメントに明記する (誤用しやすいポイント)。

### 将来の発展

- 複数個体のシード投入 (シード + そのバリアント自動生成、複数の既定値候補の投入)。
- コード側 `LayoutEncodeParams::default()` からの自動抽出 (`--seed-from-code` 的なオプション)。
- 「シード個体を必ずパレートフロントに残す」制約付き NSGA-II。
- 依存ライブラリ更新前後で自動的にシード投入 + tune を回し、更新影響レポートを生成する CI ワークフロー (0074 との連携)。
