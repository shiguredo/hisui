# 外部 vmaf CLI コマンドを shiguredo/vmaf-rs に置き換える

- Priority: Medium
- Created: 2026-06-02
- Completed: 2026-06-08
- Model: Opus 4.8
- Branch: feature/change-replace-vmaf-with-vmaf-rs
- Polished: 2026-06-08

## 目的

現状の VMAF 評価は外部の `vmaf` CLI コマンドに依存している。これを Rust の FFI バインディングライブラリ <https://github.com/shiguredo/vmaf-rs> (crates.io 上のクレート名は `shiguredo_vmaf`) に置き換える。

置き換えによって以下が期待できる:

- 外部 `vmaf` バイナリの事前インストール (PATH 設定) が不要になり、実行環境のセットアップが単純になる
- JSON ファイルを介した CLI 実行 → 出力ファイル読み込み → パースという間接的な処理を、ライブラリ呼び出しに置き換えられる
- 自社製ライブラリであり、依存の管理・追従がしやすい

トレードオフとして、ビルド時に libvmaf の prebuilt ダウンロードによるネットワークアクセスが発生する (設計方針 3 を参照)。

## 優先度根拠

VMAF 評価は `hisui vmaf` サブコマンドと、それをサブプロセスとして起動する `hisui tune` サブコマンド (いずれも開発・チューニング用途) で使われ、エンドユーザー向けの合成機能には影響しない。そのため最優先ではない。一方で外部バイナリ依存の排除は開発体験とポータビリティを明確に改善するため Medium とする。

## 現状

### `src/sora/recording_subcommand_vmaf.rs` (VMAF 評価の本体)

- `check_vmaf_availability()` (pub 関数): `Command::new("vmaf")` で外部バイナリの存在を確認する
- `run_vmaf_evaluation()`: `Command::new("vmaf")` を以下の固定引数で実行する
  - `--reference <ref.yuv>` `--distorted <dist.yuv>`
  - `--width <w>` `--height <h>`
  - `--output <out.json>` `--json`
  - `--pixel_format 420` `--bitdepth 8` (hisui では固定)
  - `--model` は未指定 (libvmaf のデフォルトモデルを使用)
- `parse_vmaf_output()`: 出力 JSON の `pooled_metrics.vmaf` から `min` / `max` / `mean` / `harmonic_mean` を取り出して `VmafScoreStats` に詰める
- CLI 引数 `--vmaf-output-file` で出力 JSON ファイルのパスを指定でき、実行結果サマリの `Output` JSON にも `vmaf_output_file_path` フィールドが含まれる
- 合成パイプラインでは、distorted (リミッター出力をエンコード → デコードしたもの) と reference (リミッター出力そのもの) をそれぞれ `YuvWriter` (`src/yuv.rs`) がプロセッサとして I420 連続バッファ (`frame.data`) のまま YUV ファイルへ逐次書き出し、そのファイルパスを CLI に渡している

### `src/sora/recording_subcommand_tune.rs` (影響を受ける利用側)

- `recording_subcommand_vmaf::check_vmaf_availability()?` を起動時チェックで呼んでいる
- `hisui vmaf` をサブプロセスとして実行し、引数に `--vmaf-output-file <trial_dir>/vmaf-output.json` を渡している
- 結果は `hisui vmaf` の標準出力 JSON から `vmaf_mean` / `elapsed_seconds` のみを読む (vmaf-output.json 自体は読まない)。標準出力 JSON 全体は `metrics.json` として trial ディレクトリに保存される

つまり現状は「8-bit I420、固定パラメータでの 1 リファレンス vs 1 ディストーション評価」しか行っておらず、vmaf-rs が提供する範囲 (8-bit I420 のフルリファレンス VMAF) と用途が一致する。

## 調査結果 (2026-06-08)

vmaf-rs (調査時点: `shiguredo_vmaf` 2026.1.0-canary.0、GitHub リポジトリの README と `tests/test_lib.rs` を確認) を調査した結果、hisui の用途を満たすことを確認した。

1. **8-bit I420 フルリファレンス API**: あり
   - `Picture::from_i420(y: &[u8], u: &[u8], v: &[u8], width: u32, height: u32) -> Result<Self, Error>`
   - `Context::new()` でコンテキスト生成、`Model::load_builtin()` でビルトインモデル 5 種 (V061 既定 / BV063 / V061Neg / V4k061 / V4k061Neg) を読み込み、参照・劣化フレームのペアをインデックス付きで登録して評価する設計
2. **プール済みメトリクス**: 4 種すべて取得可能
   - `Context::score_pooled(&self, model: &Model, method: PoolingMethod, index_low: u32, index_high: u32) -> Result<f64, Error>`
   - `PoolingMethod` は Min / Max / Mean / HarmonicMean をサポートし、hisui が利用する 4 メトリクスと一致する
   - `index_low` / `index_high` は両端を含む閉区間で、フレームインデックスは 0 始まり (libvmaf 本体の `vmaf_score_pooled` の仕様。全フレームのプーリングは `[0, フレーム数 - 1]` を渡す)
   - フレーム単位スコアも `score_at_index()` で取得可能
3. **libvmaf 本体の入手方法**:
   - 既定: GitHub Releases から prebuilt バイナリを自動ダウンロード (通常の `cargo build` のみで可)
   - `source-build` フィーチャでソースビルドも可能 (要 Git、C/C++ コンパイラ、Meson + Ninja、NASM (x86_64 のみ)、xxd)
4. **対応プラットフォーム**: Ubuntu 24.04 / 22.04 (x86_64・arm64)、macOS 15 / 26 (arm64) でテスト済み
5. **公式 libvmaf とのスコア一致検証**: 公式 CLI とのスコア値直接比較テストは無い。`tests/test_lib.rs` は性質ベースの検証のみ (同一フレーム比較スコア ≥ 95.0、ノイズ加算フレームで < 90.0、Mean プーリングはフレーム単位スコアの平均との差 ±0.1、HarmonicMean は ±1.0)。その他に fuzz / pbt / コーデックベンチあり
   - ただし vmaf-rs は libvmaf 本体への FFI バインディングであり、評価エンジン自体は現状の `vmaf` CLI と同一

## 設計方針

1. **YUV データの受け渡し**: 既存の YUV ファイル経由を維持する (最小変更)
   - 合成パイプライン (`YuvWriter`) はそのまま使い、書き出された reference / distorted の YUV ファイルを hisui 側で読み込んでフレームごとに `Picture::from_i420` で vmaf-rs に渡す
   - 評価後も YUV ファイルは削除せず残す (同一性確認の手順で外部 CLI と突き合わせるため)
   - フレームサイズは `width * height * 3 / 2` バイト。I420 連続バッファから Y / U / V プレーンへのオフセット分割が必要。幅・高さは `usize` から `u32` への変換 (TryFrom) が必要
   - reference / distorted のフレーム数が一致しない場合はエラーとする (両者は同じリミッター出力に由来し、同数になる想定。不一致は実装バグか入力破損のため握りつぶさない)
   - メモリ上のプレーン直接渡し (パイプラインから vmaf-rs へ直結) はファイル書き出しを省略できるが、distorted (エンコード → デコード経由) と reference (リミッター出力直) は異なるパイプライン段から異なるタイミングで届くため、ペア登録には hisui 側でのフレームバッファリングが必要になり、複雑化とメモリ消費増 (1080p の生 I420 で 1 フレーム約 3 MB) を招く。本 issue ではやらない
   - 評価ループの進捗表示は実装しない (現状 CLI の stderr 進捗は失われるが、開発用途のため tracing ログで足りる)
2. **モデル**: vmaf-rs のビルトイン既定モデル (V061) を使う
   - 現状の `vmaf` CLI も `--model` 未指定 (= libvmaf デフォルトの vmaf_v0.6.1) なので一致する想定だが、後述の同一性確認で実スコアの一致をもって担保する
   - モデルデータの所在: `Model::load_builtin()` が読むモデルは libvmaf 本体に埋め込まれている (libvmaf のビルド時に meson が `xxd -i` で各モデル JSON を C 配列化してコンパイルする。`built_in_models` オプションは既定で有効)。vmaf-rs は libvmaf を静的リンクするため、最終的に hisui のバイナリにモデルごと含まれる。prebuilt の `libvmaf.a` にも埋め込み済み
   - そのため実行時にモデルファイルの配置やネットワークアクセスは不要で、ユーザーが追加で用意するものはない
   - バイナリに含まれるモデルサイズ (生 JSON、libvmaf v3.1.0 の固定小数点版): V061 約 19 KB / V061Neg 約 19 KB / V4k061 約 22 KB / V4k061Neg 約 23 KB / BV063 約 400 KB。`load_builtin` の 5 モデルすべてが埋め込まれるため合計でおよそ 480 KB がバイナリに乗る (hisui が使うのは V061 のみだが未使用分も含まれる)
3. **libvmaf 本体の入手方法**: 既定の prebuilt 自動ダウンロードを採用する
   - これによりビルド時にネットワークアクセスが発生する。CI やオフラインビルドで問題になった場合は `source-build` フィーチャへの切り替えを検討する (その判断は実装時の CI 結果で行う)
4. **依存追加**: `Cargo.toml` の実態 (「バージョンは厳密一致で指定している」のコメントどおり全依存が `=x.y.z` 指定) に合わせ、`shiguredo_vmaf = "=2026.1.0-canary.0"` を厳密一致で追加する
   - 実装着手時に crates.io で canary が進んでいないか確認し、進んでいれば差分を確認のうえ最新を採用する
   - canary 版の採用は `shiguredo_m3u8` / `shiguredo_rtmp` / `sora_sdk` 等で既に常態であり問題ない
   - 用途コメント (VMAF 評価) を明記する

## 後方互換性の扱い

出力 JSON ファイル (vmaf-output.json) はライブラリ化により生成されなくなるため、以下を削除する (後方互換のない変更):

- `hisui vmaf` の CLI 引数 `--vmaf-output-file`
- 実行結果サマリ `Output` JSON の `vmaf_output_file_path` フィールド
- `recording_subcommand_tune.rs` がサブプロセス起動時に渡している `--vmaf-output-file` 引数

削除して問題ない根拠:

- `hisui vmaf` / `hisui tune` は開発・チューニング用途のサブコマンドである
- tune は標準出力 JSON の `vmaf_mean` / `elapsed_seconds` しか読んでおらず、vmaf-output.json に依存していない
- スコア 4 種 (`vmaf_min` / `vmaf_max` / `vmaf_mean` / `vmaf_harmonic_mean`) は標準出力 JSON に引き続き含まれるため、情報としては失われない

付随して、tune が保存する `metrics.json` (標準出力 JSON のコピー) からも `vmaf_output_file_path` が消えるが、tune の読み取りには影響しない。

このため CHANGES.md の種別は `[CHANGE]` とし、ブランチは `feature/change-replace-vmaf-with-vmaf-rs` とする。エントリには「外部 `vmaf` バイナリが不要になる」「ビルド時に libvmaf の prebuilt ダウンロードが発生する」「`--vmaf-output-file` が削除される」を含めること。

## 変更対象

- `Cargo.toml`: `shiguredo_vmaf` を追加する
- `src/yuv.rs`: YUV ファイルを読み込んでフレームごとの Y / U / V プレーンに分割する読み込み側 (`YuvReader` 等) を追加する (書き込み側 `YuvWriter` と同居させる)
- `src/sora/recording_subcommand_vmaf.rs`:
  - `check_vmaf_availability()` を削除する (ライブラリ化によりバイナリ存在チェック自体が不要になる)
  - `run_vmaf_evaluation()` / `parse_vmaf_output()` を vmaf-rs ベースの実装 (YUV ファイル読み込み → フレーム分割 → `Picture::from_i420` → ペア登録 → `score_pooled` を 4 つの `PoolingMethod` で呼び出し → `VmafScoreStats`) に置き換える
  - 「後方互換性の扱い」に記載の `--vmaf-output-file` 引数と `Output::vmaf_output_file_path` を削除する
  - 不要になった `use std::process::{Command, Stdio}` を掃除する
- `src/sora/recording_subcommand_tune.rs`:
  - `check_vmaf_availability()` 呼び出しを削除する
  - サブプロセス起動時の `--vmaf-output-file` 引数を削除する
- `CHANGES.md`: 上記の `[CHANGE]` エントリを追加する

エラーメッセージ・ログは規約どおり英語とする。

## テスト戦略

現状 vmaf 関連の自動テストは存在しない。また、リポジトリには `pbt/` クレートも `proptest` 依存も未整備であり、PBT 基盤の新設は本 issue のスコープ外とする (AGENTS.md の PBT 方針はあるが、基盤整備は別 issue で扱うべき規模のため)。本 issue では単体テストで対応する:

- YUV フレーム分割 (I420 バッファ → Y/U/V プレーン分割) の正常系とエラーパス (ファイルサイズがフレーム境界に合わない、0 フレーム、reference / distorted のフレーム数不一致) を、CLAUDE.md の単体テスト命名規約に従い `tests/test_yuv.rs` で検証する (既存の `tests/*_tests.rs` はレガシーであり踏襲しない)
- 正常系の書き出し → 読み込みのラウンドトリップは本来 PBT 対象だが、PBT 基盤が未整備のため暫定的に単体テストで担保し、基盤整備の別 issue で PBT へ移管する
- VMAF スコア計算自体の正しさは vmaf-rs 側のテストに委ね、hisui 側では再検証しない

## 結果の同一性確認

vmaf-rs 側に公式 CLI とのスコア値直接比較テストが無いため (調査結果 5)、置き換え時に一度だけ手元で確認する:

1. 新実装の `hisui vmaf` を実行する (reference.yuv / distorted.yuv はファイルとして残る)
2. 残った同一の YUV ファイルに対し、外部 `vmaf` CLI (手元にインストールしたもの) を現状実装と同じ引数で実行する
3. 両者の `min` / `max` / `mean` / `harmonic_mean` を比較する
   - 同一エンジン (libvmaf) のため原則一致を期待し、浮動小数点演算順序やビルド差を見込んだ上限として許容誤差 ±0.1 とする
   - 超えた場合は原因 (モデル不一致等) を調査して issue に記録する
4. 確認結果 (使用した入力、両者のスコア、差分) を本 issue に記録してから close する

### 確認結果 (2026-06-08)

vp9-webm の録画 (合成出力 320x240、30 フレーム) で生成した同一の reference.yuv / distorted.yuv に対し、新実装 (vmaf-rs) と外部 `vmaf` CLI 3.0.0 を同一引数で実行して比較した。

| メトリクス | 新 (vmaf-rs) | 旧 (vmaf CLI 3.0.0) |
|---|---|---|
| min | 92.60782130867904 | 92.607821 |
| max | 97.34159608197254 | 97.341596 |
| mean | 94.171165973261 | 94.171166 |
| harmonic_mean | 94.16339665306538 | 94.163397 |

差分はいずれも 1e-6 未満 (CLI 側 JSON は小数 6 桁に丸められるため、見かけ上の差はその丸めによるもの)。許容誤差 ±0.1 を大きく下回り、同一エンジン (libvmaf) として実質完全一致であることを確認した。

## 完了条件

- `recording_subcommand_vmaf.rs` の VMAF 評価が vmaf-rs ベースに置き換わり、`Command::new("vmaf")` と `check_vmaf_availability()` が削除されている
- `recording_subcommand_tune.rs` から外部 `vmaf` バイナリへの依存 (可用性チェック、`--vmaf-output-file` 渡し) が削除されている
- 結果の同一性確認 (上記) が完了し、結果が issue に記録されている
- テスト戦略に沿ったテストが追加されている
- CHANGES.md に `[CHANGE]` エントリが追加されている

## 実装中に発見・修正した既存バグ

実装後に `hisui vmaf` / `hisui tune` を実機で動作確認した際、compose 段で完全にデッドロックする既存バグを発見し、本ブランチで併せて修正した。

- 症状: `# Compose for VMAF` 出力直後に全 tokio ワーカーが park し、CPU 0% でハングする。develop ブランチ (本 issue の変更前) でも再現したため、vmaf-rs 置き換えとは無関係な既存バグと確認した
- 原因: `setup_vmaf_pipeline` の reader→decoder のトラック配線ミス。`VideoReader::run` は自身の processor_id 名のトラックに publish するが、decoder には独立採番した別のトラック ID を subscribe させていたため、両者が繋がらずデータが流れていなかった (compose 側は reader の processor_id からトラック ID を導出しており正しい)
- 修正: vmaf 側も `TrackId::new(reader_processor_id.get())` で reader の processor_id からトラック ID を導出するよう変更。さらにレビューを受けて、パイプライン内の全トラック ID を compose と同じく生成元プロセッサの processor_id 由来に統一し、独立採番のヘルパー (`next_track_id` / `next_track_number`) を削除した
- 未リリース部分のバグ修正のため CHANGES.md への `[FIX]` 追記は不要と判断した

## 動作確認結果

`hisui vmaf` 単体および `hisui tune` 経由 (vmaf-rs 実装) で実機動作を確認した。

- `hisui vmaf` (vp9-webm, 30 フレーム): 正常完了し VMAF スコアを出力 (mean 94.17 等)
- `hisui tune` (2 トライアル, 20 フレーム): vmaf サブプロセスが正常完了し、パレートフロントまで出力
- これにより compose → YUV → vmaf-rs 評価の経路が end-to-end で動作することを確認した
- 旧 `vmaf` CLI とのスコア同一性も確認済み (詳細は「結果の同一性確認」の確認結果を参照)

## 解決方法

- `Cargo.toml` に `shiguredo_vmaf = "=2026.1.0-canary.0"` を追加した
- `src/yuv.rs` に I420 YUV ファイルをフレーム単位で読み込み Y/U/V プレーンに分割する `YuvReader` / `YuvFrame` を追加した
- `src/sora/recording_subcommand_vmaf.rs` の VMAF 評価を vmaf-rs ベースに置き換えた (`Context` 構築 → `Model::load_builtin(V061)` → フレームごとに `Picture::from_i420` でペア登録 → flush → `score_pooled` を 4 種の `PoolingMethod` で算出)。`check_vmaf_availability()` / `parse_vmaf_output()` / 外部 `vmaf` コマンド実行と `--vmaf-output-file` / `Output::vmaf_output_file_path` を削除した
- `src/sora/recording_subcommand_tune.rs` から `check_vmaf_availability()` 呼び出しと `--vmaf-output-file` 渡しを削除した
- `CHANGES.md` に `[CHANGE]` エントリを追加し、`docs/` から外部 `vmaf` 依存前提の記述を整理した
- 実機確認中に発見した compose パイプラインのトラック配線デッドロック (既存バグ) を修正した (上記「実装中に発見・修正した既存バグ」参照)
- テスト: `tests/test_yuv.rs` で `YuvReader` のフレーム分割・空ファイル・端数バイト・フレーム数差を検証。`tests/e2e.rs` に `hisui vmaf` の e2e テストを追加し、compose → vmaf-rs 評価の全経路を実走で検証 (配線デッドロックの回帰も兼ねる)
- 旧 `vmaf` CLI とのスコア同一性を確認済み (「結果の同一性確認」参照)

### 対応を見送った項目 (レビュー指摘より)

- `run_vmaf_evaluation` のフレーム数不一致・0 フレームのエラーパスの自動テスト化は見送った。該当ロジックは libvmaf 依存の関数内にあり切り出しコストに見合わず、e2e テストと手動確認で実経路は担保できているため
- `YuvWriter` の単体テストと `read_frame` の EINTR 再試行は見送った。前者は e2e テストで実経路をカバー、後者は通常ファイル I/O では発生がまれでエラー時も安全に停止するため

## 関連

- [[0010-feature-refactor-replace-optuna-with-builtin-nsga2]]: `hisui tune` の外部 `optuna` CLI 依存を排除する取り組み。本 issue と合わせて完了すると tune / vmaf サブコマンドの外部コマンド依存が無くなる
