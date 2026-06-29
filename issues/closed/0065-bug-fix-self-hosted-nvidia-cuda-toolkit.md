# self-hosted runner (NVIDIA-Video-Codec-SDK) に nvidia-cuda-toolkit (nvcc) を導入する

- Priority: Medium
- Created: 2026-06-25
- Completed: 2026-06-29
- Model: Opus 4.7
- Branch: feature/fix-self-hosted-nvidia-cuda-toolkit
- Polished:

## 目的

issue 0059 で `.github/workflows/ci.yml` の `test-nvidia-video-codec` ジョブに追加した `cargo check --features candle,candle-cuda -p hisui` / `cargo clippy --features candle,candle-cuda -p hisui --all-targets -- --deny warnings` の各 step が、self-hosted runner 上に `nvcc` が無いためビルドに失敗している。candle-cuda 経路の CI 検証が機能していない状態を解消するため、runner に `nvidia-cuda-toolkit` (nvcc を含む CUDA Toolkit 一式) を導入する。

公開情報の根拠:

- 失敗ジョブ: https://github.com/shiguredo/hisui/actions/runs/28141542604/job/83339780842
- candle-core 0.10.2 の cuda backend は `.cu` カーネルを `nvcc` でビルド時にコンパイルする (`candle-kernels` クレートが build script 内で `nvcc` を起動する設計)

## 優先度根拠

Medium。candle-cuda 経路を CI で検査できない状態が続くと、本系列の後続 issue (0061 / 0062 / 0064) で CUDA 側の実装が壊れても検出できない。0059 マージ後早めに対応する。利用者向け機能の停止には繋がらない (本リリースのバイナリ配布物には candle feature を含めない方針のため) のため High ではなく Medium。

## 現状

- `.github/workflows/ci.yml` の `test-nvidia-video-codec` ジョブは self-hosted runner (group: `Self`、labels: `[self-hosted, linux, x64, NVIDIA-Video-Codec-SDK]`、`timeout-minutes: 30`) で実行される。既存 step は rustup インストールと `cargo check/clippy/test --features nvcodec -p hisui` のみで、apt パッケージのインストールを CI 内で行わない方針 (runner 側で事前インストール済みである前提)
- 同 runner 上には NVIDIA Video Codec SDK (`shiguredo_nvcodec` のリンク先) は事前に配置されているが、`nvidia-cuda-toolkit` (nvcc コンパイラを含む CUDA Toolkit 一式) は配置されていない
- 0059 で追加した candle-cuda 関連の 2 step (`cargo check --features candle,candle-cuda -p hisui` / `cargo clippy --features candle,candle-cuda -p hisui --all-targets -- --deny warnings`) が nvcc 不足で失敗する
- 一方、GitHub-hosted runner で動く `ubuntu-binary` / `release.yml` / `pypi-publish.yml` の各ジョブは `shiguredo/github-actions/.github/actions/setup-cuda-toolkit@main` Composite Action を使って毎回 CUDA Toolkit をセットアップしているため、こちらは nvcc を含む構成になっている

## 設計方針

### 方針 A (本命): runner に nvidia-cuda-toolkit を事前インストール

self-hosted runner (NVIDIA-Video-Codec-SDK ラベル) の OS 側に `nvidia-cuda-toolkit` パッケージを事前インストールする。

- `apt install -y nvidia-cuda-toolkit` 等で nvcc / nvprof / libcudart-dev 等を一式入れる
- インストール後、`nvcc --version` が PATH 経由で実行できることを確認する
- 一度入れてしまえば既存の `test-nvidia-video-codec` の他 step (nvcodec 関連) には影響しない (nvcodec は cudart の動的リンクで動き、nvcc を必要としない) ため副作用は小さい

CUDA バージョンは `.github/workflows/ci.yml` の `env.CUDA_VERSION: 13.0.2` と整合させる方向で検討する (整合させない場合に candle-core 0.10.2 のビルドが通るかは要確認)。

### 方針 B (代替案): CI yaml 側で Composite Action 経由でセットアップ

`shiguredo/github-actions/.github/actions/setup-cuda-toolkit@main` を `test-nvidia-video-codec` ジョブにも適用し、CI 実行のたびに CUDA Toolkit をセットアップする。

- Composite Action 自体が self-hosted runner で動作するかは要検証 (GitHub-hosted の ubuntu 想定で書かれている可能性がある)
- 毎回セットアップに時間がかかるため、`timeout-minutes: 30` を超過する懸念
- self-hosted runner で apt-get install を CI 内で叩くパターンの前例が無く、既存方針 (runner 側で事前インストール) と整合しない

方針 A を本命とし、方針 B はバックアップとして検討する。

### CI 設定への影響

方針 A を採用する場合、`.github/workflows/ci.yml` 側の変更は不要 (runner 側のセットアップだけで完結する)。`ubuntu-binary` ジョブで使われている `setup-cuda-toolkit` Composite Action は本 issue では触らない。

方針 B を採用する場合、`test-nvidia-video-codec` ジョブに `setup-cuda-toolkit` Composite Action の呼び出し step を追加する。

### ドキュメント

runner セットアップ手順は内部運用ドキュメントに記載する。リポジトリ内の `README.md` / `docs/` には書かない方針 (利用者向け情報ではないため)。

`CHANGES.md` への追記は不要 (CI 内部の話で利用者影響なし)。

## 完了条件

- `test-nvidia-video-codec` ジョブで `cargo check --features candle,candle-cuda -p hisui` が green になる
- 同ジョブの `cargo clippy --features candle,candle-cuda -p hisui --all-targets -- --deny warnings` も green になる
- 既存 step (`cargo check/clippy/test --features nvcodec -p hisui`) が引き続き green であること (nvcc 導入で nvcodec 経路が壊れないことを確認)

## 解決方法

起票時の想定と異なり、`nvcc` は self-hosted runner に既にインストール済みであったが `PATH` が通っていなかった。さらに、candle-onnx のビルドには別途 `protoc` (`protobuf-compiler`) が必要だが、それも runner にインストールされていなかった。以下の 2 点で対応した。

- issue 0059 のブランチで `.github/workflows/ci.yml` の `test-nvidia-video-codec` ジョブに CUDA PATH を通す step を追加した (`CUDA_PATH=/usr/local/cuda` を `$GITHUB_ENV` に、`/usr/local/cuda/bin` を `$GITHUB_PATH` に設定)
- self-hosted runner 管理者に `protobuf-compiler` の事前インストールを依頼して対応した

結果として、`test-nvidia-video-codec` ジョブで `cargo check --features candle,candle-cuda -p hisui` と `cargo clippy --features candle,candle-cuda -p hisui --all-targets -- --deny warnings` の両方が green になることを CI で確認した。

「runner に `nvidia-cuda-toolkit` を新規インストールする」という起票時の方針 A は不要であった。設計方針 / 完了条件は起票時の理解で書いたものをそのまま残し、本セクションで実態を明示する。
