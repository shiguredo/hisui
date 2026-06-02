# vmaf ライブラリを shiguredo/vmaf-rs に置き換えを検討する

- Priority: Medium
- Created: 2026-06-02
- Model: Opus 4.8
- Branch: feature/refactor-replace-vmaf-with-vmaf-rs

## 目的

現状の VMAF 評価は外部の `vmaf` CLI コマンドに依存している。これを Rust の FFI バインディングライブラリ <https://github.com/shiguredo/vmaf-rs> に置き換えられないかを検討する。

置き換えによって以下が期待できる:

- 外部 `vmaf` バイナリの事前インストール (PATH 設定) が不要になり、ビルド・実行環境のセットアップが単純になる
- JSON ファイルを介した CLI 実行 → 出力ファイル読み込み → パースという間接的な処理を、ライブラリ呼び出しに置き換えられる
- 自社製ライブラリであり、依存の管理・追従がしやすい

## 優先度根拠

VMAF 評価は `hisui vmaf` サブコマンド (開発・チューニング用途) でのみ使われ、エンドユーザー向けの合成機能には影響しない。そのため最優先ではない。一方で外部バイナリ依存の排除は開発体験とポータビリティを明確に改善するため Medium とする。

## 現状

- `src/sora/recording_subcommand_vmaf.rs` が VMAF 評価を担う
- `check_vmaf_availability()` (682 行付近): `Command::new("vmaf")` で外部バイナリの存在を確認する
- `run_vmaf_evaluation()` (701 行付近): `Command::new("vmaf")` を以下の固定引数で実行する
  - `--reference <ref.yuv>` `--distorted <dist.yuv>`
  - `--width <w>` `--height <h>`
  - `--output <out.json>` `--json`
  - `--pixel_format 420` `--bitdepth 8` (hisui では固定)
- `parse_vmaf_output()` (743 行付近): 出力 JSON の `pooled_metrics.vmaf` から `min` / `max` / `mean` / `harmonic_mean` を取り出して `VmafScoreStats` に詰める

つまり現状は「8-bit I420、固定パラメータでの 1 リファレンス vs 1 ディストーション評価」しか行っておらず、vmaf-rs が提供する範囲 (8-bit I420 のフルリファレンス VMAF) と用途が一致する。

## 設計方針

検討・実装にあたって確認・対応すべき点:

1. **vmaf-rs の API が hisui の用途を満たすか確認する**
   - vmaf-rs は 8-bit I420 のフルリファレンス VMAF に対応している (README より)。hisui も `--pixel_format 420` `--bitdepth 8` 固定なので前提は合致する
   - 現状 hisui は `min` / `max` / `mean` / `harmonic_mean` の 4 つのプール済みメトリクスを利用している。vmaf-rs がこれら 4 種のプール値を取得できるか確認する。取得できない (単一スコアのみ等) 場合は、フレーム単位スコアから hisui 側で集計するか、vmaf-rs 側に機能追加を依頼するかを判断する
2. **YUV データの受け渡し方法を整理する**
   - 現状は合成結果を YUV ファイルに書き出してから CLI に渡している (`YuvWriter` 利用)。vmaf-rs は `Picture::from_i420(&y, &u, &v, width, height)` でメモリ上のプレーンを直接受け取れるため、ファイル経由を省略できる可能性がある。ただし最小変更で進めるなら、まずは既存の YUV ファイルを読み込んで vmaf-rs に渡す形でも良い
3. **libvmaf 本体の入手方法を決める**
   - vmaf-rs はデフォルトで GitHub Releases から prebuilt バイナリを取得してビルドし、`source-build` フィーチャでソースビルドも可能。hisui のビルド・CI 環境でどちらを使うかを決める
4. **依存追加の方針 (AGENTS.md 準拠)**
   - 依存はマイナーバージョンまで指定する
   - 依存ライブラリの用途をコメントで明記する

## 結果の同一性確認 (重要)

置き換え前後で VMAF スコアが劇的に変わらないことを確認する必要がある。ただし:

- **vmaf-rs 側のテストで、libvmaf 公式 (= 現状 hisui が使う `vmaf` CLI と同じエンジン) と同等のスコアが出ることが十分に担保されているなら、hisui 側での確認は不要とする。**
- まず vmaf-rs のテスト・検証内容を調査し、結果の妥当性 (公式 libvmaf との一致) が担保されているかを確認する
- 担保されていない場合のみ、hisui 側で以下を行う:
  - 同一の reference / distorted YUV に対し、旧 (`vmaf` CLI) と新 (vmaf-rs) で `min` / `max` / `mean` / `harmonic_mean` を比較する
  - 許容誤差を定めて (例: 浮動小数演算の差異程度に収まること)、それを超えないことを確認する

## 完了条件

- vmaf-rs が hisui の用途 (8-bit I420、4 種プールメトリクス) を満たすかの調査結果が明確になっている
- 満たす場合: `recording_subcommand_vmaf.rs` の VMAF 評価部分が vmaf-rs ベースに置き換わり、外部 `vmaf` バイナリへの依存 (`Command::new("vmaf")`) が削除されている
- 結果の同一性が (vmaf-rs 側または hisui 側のいずれかで) 担保されている
- 満たさない場合: 何が不足しているかと、vmaf-rs 側への要望 or 置き換え見送りの判断が issue に記録されている

## 備考

- 置き換えが難しい (vmaf-rs の機能不足、ビルド環境の制約など) と判明した場合は、その理由を明記して `issues/pending/` へ移動する
- CHANGES.md には外部 `vmaf` バイナリ依存の有無の変化を反映する (後方互換に関わるため種別を慎重に選ぶ)
