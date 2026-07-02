# CI でエンコーダーパラメーター tune を実行してパレートフロントを収集する

- Priority: Medium
- Created: 2026-07-02
- Completed:
- Model: Opus 4.7
- Branch: feature/add-encoder-tune-ci
- Polished:

## 目的

hisui には NSGA-II ベースの `tune` サブコマンドが既に実装されており (`src/tune.rs`, `src/sora/recording_subcommand_tune.rs`)、「合成時間 vs VMAF」の 2 目的でエンコーダーパラメーターを探索できる。しかし CI では tune を回しておらず、`DEFAULT_LAYOUT_JSON` (実体は `layout-examples/tune-libvpx-vp9.jsonc` を `include_str!` で埋め込み) の既定値は人手で更新しているのが現状。

本対応では、CI で `workflow_dispatch` で tune を手動起動し、各エンコーダーのパレートフロントを GitHub Actions アーティファクトとして残す仕組みを追加する。定期実行 (`schedule`) はしない。tune を回したいのは基本的に依存ライブラリを更新したタイミングであり、それ以外に自動で回し続ける必要がないため。

最終的な既定値の決定 (複数環境の結果を突き合わせる・パレートフロントから 1 点選ぶ) は当面人手で行う前提とし、CI で自動化するのは tune 実行と結果保存までに閉じる。人手はアーティファクトを見て選ぶだけで済むようになる。

将来的には、複数環境の結果の突き合わせやパレートフロントからの最終選択そのものを自動化する余地を残す。

## 優先度根拠

- 既定パラメーターは性能・画質に直結するが、依存ライブラリ更新のたびに手元で tune を回して結果をまとめるのは負担が大きく、追従が遅れがち。
- 一方で `tune` サブコマンドと CI インフラ (self-hosted runner を含む既存の workflow 群) は既に揃っており、`workflow_dispatch` の workflow を 1 本追加すれば最初の一歩を踏み出せる。実装コストは中程度。
- 別 issue 0005 (最新パラメーター追従) が完了した後に既定値の見直しをスムーズに開始できるようにしておく価値がある。
- 以上から Medium 妥当。

## 現状

### tune サブコマンドの実装

- `src/tune.rs`: NSGA-II 本体
- `src/tune/nsga2.rs`, `rng.rs`, `storage.rs`: 探索アルゴリズム・乱数・永続化
- `src/sora/recording_subcommand_tune.rs`: CLI エントリーポイント。`DEFAULT_LAYOUT_JSON` 定数は `layout-examples/tune-libvpx-vp9.jsonc` を `include_str!` で埋め込む形
- 対応エンコーダー: libvpx-vp8/vp9, openh264, svt-av1, video-toolbox-h264/h265, nvcodec-h264/h265/av1 (計 10)
- 外部パッケージは不要 (Rust binding のみで完結)

### 入力・設定ファイル

- `layout-examples/tune-*.jsonc` (計 10 個): 各エンコーダーの tune 用 layout
- `search-space-examples/full.jsonc`: 探索空間の既定定義
- `testdata/archive-*.mp4/webm`: 既存のテストメディア (単体テスト用途)
- ドキュメント: `docs/command_tune.md`

### CI インフラ

- `.github/workflows/ci.yml`: build / test / lint。scheduled + push / PR で実行
- ubuntu-24.04 の GitHub-hosted runner + self-hosted (NVIDIA-Video-Codec-SDK) + macOS ARM64 のジョブ構成が既に存在
- tune は現状 CI では動いていない

### 既存 issue との関係

- 0005 (Medium open): 最新エンコーダーパラメーター追従。本 issue は 0005 の完了後に最初のキックを打つのが自然 (最新のパラメーター範囲で探索できるため)。ただし必須依存ではない。

## 設計方針

### 基本方針

1. **CI で自動化するのは tune 実行と結果保存まで**: パレートフロントを JSON アーティファクトに残すところで区切り、そこから 1 点を選ぶのは人手で行う (運用手順を docs に書く)。
2. **トリガーは `workflow_dispatch` のみ**: 定期実行は導入しない。依存ライブラリ更新など人手のタイミングでキックする。
3. **既存 `ci.yml` に相乗りしない**: `.github/workflows/tune.yml` (仮称) として独立させ、tune 実行失敗が CI 全体を落とすことがないようにする。
4. **段階的にエンコーダーを増やす**: 初回スコープは GitHub-hosted ubuntu で動く libvpx-vp8/vp9, openh264, svt-av1 に絞る。GPU 系 (NVENC / VideoToolbox) は self-hosted runner に載せる段階を分ける。
5. **入力メディアは public フリー素材で妥協する**: Big Buck Bunny 等の広く使われている素材を採用する想定。既存 `testdata/archive-*` はテスト用途に温存する。素材の選定・調達方法 (LFS / ダウンロード / リポジトリ埋め込み) は issue 実装時に決める。
6. **1 回の実行時間の上限を意識する**: フル探索は数時間から数十時間になり得るため、CI 用に絞った search-space プロファイル (例: `search-space-examples/ci.jsonc`) を新設するか、`--generations` / `--population` 相当の引数で絞る。手動起動なので実行頻度に制約はないが、1 回あたりの実行時間はランナーの上限内に収める。

### 主要論点 (実装着手前に詰める必要があるもの)

1. **入力メディア**: 素材選定 (Big Buck Bunny 等の候補比較)、ライセンス、尺・解像度・元コーデック、リポジトリ格納方法。
2. **対象コーデック**: 初回に含めるものと後回しにするもの。GPU 系を self-hosted runner に載せる時期。
3. **1 回あたりの実行時間予算**: 世代数・個体数・入力尺のトレードオフ。ランナー上限に対する余裕。
4. **成果物の残し方**: パレートフロント JSON、可視化画像 (matplotlib 等)、CSV。Actions アーティファクトの保管期間。将来の GitHub Pages 化余地。
5. **失敗時の扱い**: tune 実行失敗は通知のみに留める。CI 全体の Required チェックには含めない。
6. **実行環境と使用ライブラリバージョンの記録**:
    - 実行環境: OS / CPU / (GPU) の情報を結果に添える。複数環境の結果を突き合わせる際のキーとして必須。
    - エンコーダー・デコーダーライブラリのバージョン: hisui の統計 (stats) 出力に既に含まれている可能性があるので、まず `src/stats*` 相当の実装を確認する。既にあれば結果 JSON にそのままマージする方針で行き、無ければ workflow 側で `cargo tree` 等から取得して添える。
    - workflow 側の環境情報 (`uname -a`, `lscpu` / `sysctl -n machdep.cpu.brand_string` 等) は生の値をアーティファクトに保存する。

### 対象外 (別 issue で扱う)

- パレートフロントからの 1 点自動選択 (重み付きスコア、Knee-point 検出等)
- 複数環境の結果の自動突き合わせ
- `DEFAULT_LAYOUT_JSON` を CI 経由で自動更新する仕組み (本 issue では人手で反映)
- 定期実行 (`schedule`) の導入 (必要になった時点で別 issue で検討)

## 完了条件

- `.github/workflows/tune.yml` (仮称) が追加され、`workflow_dispatch` で手動起動できて動くこと。
- 少なくとも 1 コーデック (libvpx-vp9 または svt-av1 を想定) の tune 結果 (パレートフロント JSON) が Actions アーティファクトとして残ること。
- 実行環境情報 (OS / CPU) とエンコーダー・デコーダーライブラリのバージョン情報が結果アーティファクトに含まれていること。
- CI 用に絞った search-space プロファイルを新設した場合はリポジトリに含まれていること。
- アーティファクトから既定値を人手で選ぶ運用手順が `docs/command_tune.md` (または新規 md) に追記されていること。
- CHANGES.md の `## develop` に `[ADD] CI でエンコーダーパラメーター tune を実行する workflow を追加する` を追記。

## 解決方法

### 実装ステップ

1. **入力メディアの選定**:
    - Big Buck Bunny や Sintel などの public フリー素材から 1 〜 2 本を選定。ライセンス条項・尺・解像度・元コーデックを確認。
    - リポジトリ内格納か CI 実行時ダウンロードかを決定 (サイズと再現性のトレードオフ)。

2. **hisui 側の tune 出力の再確認**:
    - `src/tune.rs` の結果出力形式・保存先を確認。
    - stats 出力 (`src/stats*` 相当) を確認し、ライブラリバージョン情報が既に含まれているかを判定。無ければ workflow 側で補完する方針を決める。

3. **CI 用 search-space プロファイルの追加 (必要なら)**:
    - `search-space-examples/full.jsonc` を短縮した CI 用プロファイル (例: `search-space-examples/ci.jsonc`) を追加。世代数・個体数・入力尺を CI 実行時間に収まるように絞る。

4. **`.github/workflows/tune.yml` の追加**:
    - trigger: `workflow_dispatch` のみ。
    - 初回はジョブを ubuntu-24.04 の GitHub-hosted runner に限定し、libvpx-vp8/vp9, openh264, svt-av1 を対象とする。
    - ステップ: hisui のビルド、入力メディアの用意、tune 実行、実行環境情報 (`uname -a`, `lscpu`) と `cargo tree` 出力の収集、パレートフロント JSON + 環境情報を 1 つのアーティファクトにまとめる。
    - 失敗しても CI 全体の Required チェックに影響しないようジョブを分離する。

5. **ドキュメント整備**:
    - `docs/command_tune.md` (または新設 md) に「CI アーティファクトからデフォルト値を選ぶ運用手順」を追記。パレートフロントの読み方、選定基準の考え方 (バランス点 / 品質優先 / 速度優先) を書く。
    - workflow の手動起動手順も併記する。

6. **後続作業 (別 issue に切り出す想定)**:
    - GPU 系 (NVENC / VideoToolbox) を self-hosted runner に載せる。
    - 複数環境結果の自動突き合わせ。
    - パレートフロントからの自動選択。
    - 必要になった時点で定期実行 (`schedule`) を導入する。

### リスク・留意点

- 1 回の CI 実行時間が予想以上に長くなるリスク。世代数を絞りすぎると探索の意味が薄れるので、絞り方は要調整。初回は「まず動く」を優先し、次回以降で世代数・個体数を調整する運用を想定。
- 入力メディア 1 本のみだと現実ワークロードと乖離する恐れ。当面は 1 〜 2 本で始め、必要に応じて増やす。
- self-hosted runner での tune 実行はコスト (電力・時間) を消費する。GPU 系対応は 1 回の実行時間を確認してから決める。
- 依存 crate のバージョン更新で tune 結果の意味が変わるため、結果アーティファクトには使用ライブラリバージョンを必ず含めて、後で突き合わせ可能にする。

### 将来の発展

- 複数環境 (ubuntu / macOS / GPU 有無) の結果を突き合わせて共通して良いパラメーターを抽出する仕組み。
- パレートフロントから 1 点を選ぶ自動選択 (重み付きスコア、Knee-point 検出等)。
- 世代・時間軸での品質推移を GitHub Pages で可視化。
- workflow_dispatch から特定 PR に対して tune を回し、その PR の影響で結果がどう変わるかを比較する運用。
- 必要になった時点で定期実行 (`schedule`) を導入する。
