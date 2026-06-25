# candle feature と ML モデル取得スクリプトを追加する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-candle-feature
- Polished: 2026-06-24

## 目的

ML 推論機能 (Whisper 文字起こしと Silero VAD) の基盤として candle (Rust 製 ML 推論フレームワーク) を hisui のオプション依存として追加し、device 自動検出の骨格 (`src/ml/{mod,device}.rs`) と ML モデル取得スクリプト (`scripts/download_ml_models.py`) を整備する。本 issue は親 (索引) issue 0012 系列の最初の層であり、`scripts/download_ml_models.py` は 0064 (YOLO) からも参照される基盤となる (依存関係グラフは 0012 を参照)。

## 優先度根拠

Medium。0061 / 0062 / 0064 すべての前提となる単独クリティカルパスのため、本系列内では最優先で着手する。本 issue 単独では推論機能を提供せず、利用者から見える変更は「candle 系オプション依存の追加」「`scripts/download_ml_models.py` の追加」の 2 点に限られる。

## 現状

- hisui には ML 推論機能がなく `src/ml/` ディレクトリは存在しない
- `Cargo.toml` の `[features]` セクションは `default = ["player"]` / `fdk-aac` / `nvcodec` / `player` のみで candle 系を持たない
- `.gitignore` には `*.safetensors` の 1 行のみで、`.onnx` / `tokenizer.json` / `config.json` 等のモデル付随ファイルは除外されていない
- `.github/workflows/ci.yml` には `test-fdk-aac` / `test-openh264` / `test-fuzz` (一時無効) が並び、`slack_notify.needs` で連結されている
- ブランチ `feature/try-candle` (PR #246、ヘッドコミット `6a84c829`) で candle 系依存・`src/ml/{mod,device,yolo}.rs`・`src/ml/audio/`・`scripts/download_ml_models.sh`・`src/subcommand_ml.rs` (マイク入力前提) が実装済みだが develop には未統合

## 設計方針

### Cargo.toml への candle feature 追加

`[features]` セクションの並びは「`default` を最上段に残し、それ以外を alphabetical 順に並べる」既存慣習に従う。結果として `candle` / `candle-cuda` / `candle-metal` の 3 段は `default` の直下・`fdk-aac` の前に入る:

```
candle = ["dep:candle-core", "dep:candle-nn", "dep:candle-transformers", "dep:candle-onnx", "dep:tokenizers"]
candle-cuda = ["candle", "candle-core/cuda"]
candle-metal = ["candle", "candle-core/metal"]
```

`dep:` プレフィックスは **既存 feature (`fdk-aac = ["shiguredo_fdk_aac"]` 等) では使われていない** 流儀を本 issue で意図的に変更する。`dep:` を付けると同名 feature の自動生成 (`cargo build --features candle-core` のような誤起動経路) が抑止される。候補 feature 名が 5 件に増えるため、誤起動経路を増やさない方が安全という判断。既存 feature を `dep:` 付きに揃え直すのは本 issue では行わない (副作用を持ち込まない)。

`[dependencies]` セクションには `raw_player` (107 行目) の直下に空行を挟み、グループコメント `# candle (ML 推論)` を 1 行入れた上で次の 5 つの optional 依存を順に書く (Cargo.toml 51 行目の既存方針「通常の依存は突然挙動が変わることが内容に、バージョンは厳密一致で指定している」 (原文ママ、`内容に` は `ないように` の typo) に従い `=` で固定する):

```toml
# candle (ML 推論)
candle-core         = { version = "=0.10.2", optional = true }  # テンソル計算
candle-nn           = { version = "=0.10.2", optional = true }  # ニューラルネット building block
candle-transformers = { version = "=0.10.2", optional = true }  # Whisper 用 transformer 実装
candle-onnx         = { version = "=0.10.2", optional = true }  # Silero VAD 用 ONNX ローダー
tokenizers          = { version = "=0.22.0", default-features = false, features = ["onig"], optional = true }  # Whisper 用 tokenizer.json パーサ
```

各候補バージョンの根拠:

- candle 0.10.2 は本 issue 着手時点で candle-onnx / candle-transformers が同期している最新安定版で、PR #246 で動作確認済み
- tokenizers 0.22.0 は candle-transformers 0.10.2 が要求するバージョン
- tokenizers の `default-features = false` で `http` / `progressbar` / `cli` を切り、Whisper の `tokenizer.json` パースに必要な `onig` のみ有効化する。`onig` を選ぶ理由は PR #246 で `default-features = false, features = ["onig"]` の構成で動作実績があるため。他の正規表現エンジン候補との比較は本 issue では行わない

各 candle crate には `default-features` 指定を入れず、各 crate のデフォルト feature をそのまま利用する (`candle-core` のデフォルトには CUDA / Metal バックエンドが含まれないことは確認済み)。

candle 系は `[workspace.dependencies]` には入れず `[dependencies]` 直書きにする。`examples/*` で candle を使わず、`pbt` も `hisui = { path = ".." }` 経由で hisui crate 越しに利用するため、workspace 共有の意義がない。0061 で pbt が candle を直接 import する設計に変えた場合は別途 workspace 化を検討する。

実装着手時に candle-core 0.10.2 / candle-onnx 0.10.2 / tokenizers 0.22.0 の `rust-version` (MSRV) が hisui の `rust-version = "1.95"` 以下であることを各 crate の `Cargo.toml` または crates.io ページで確認する。MSRV が 1.95 を超えていた場合は本 issue では hisui の rust-version を上げて対応する (採用を見送る方向には倒さない)。あわせて 5 crate が yank されていないことを `cargo search` で確認する。

### システム依存

- `protoc` (candle-onnx のビルドに必須、Ubuntu は apt の `protobuf-compiler`、macOS は `brew install protobuf`)
- `build-essential` (tokenizers の `onig` feature の Oniguruma ビルドに必要、既存 `test-fdk-aac` / `test-openh264` ジョブの apt パッケージ列で充足)
- CUDA toolkit (`candle-cuda` を有効化したビルドのみ。本 issue では `test-nvidia-video-codec` self-hosted ジョブで `cargo check --features candle,candle-cuda -p hisui` と `cargo clippy --features candle,candle-cuda -p hisui --all-targets -- --deny warnings` を回し、ビルド・型・リント検査までを確認する。テストは CUDA runtime を直接叩かないため後続 issue で再検討する)

リリース配布バイナリ (`.github/workflows/ci.yml` の `ubuntu-binary` / `macos-binary`、`release.yml` 全般、`Dockerfile`、`pypi-publish.yml`) には本 issue では candle feature を **含めない**。利用者向け CLI (`hisui -x transcribe`) が完成する 0063 のマージ時に再検討する。

### src/lib.rs / src/ml/mod.rs

- `src/lib.rs` の `pub mod` 群は概ね alphabetical 順 (一部 `rtmp` / `rtsp` / `s3` が `srt` の後に置かれるなど揺らぎあり) で並んでいる。`#[cfg(feature = "candle")] pub mod ml;` は alphabetical 位置である `pub mod metrics;` の **直下**・`pub mod mixer;` の **直前** に挿入する
- `src/ml/mod.rs` を新規作成し、内容は次の 1 行のみとする:

```rust
pub mod device;
```

後続の 0061 が `pub mod audio;` を、0064 が `pub mod yolo;` および `MlModel` enum を追加することで `mod.rs` が成長していく。

### src/ml/device.rs

device 自動検出ロジックを実装する。公開 API は次のとおり:

```rust
use candle_core::Device;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlDevice {
    Cpu,
    Cuda,
    Metal,
}

impl MlDevice {
    /// 有効化されている feature とランタイム条件に基づいて最適な device を選ぶ。
    /// 初期化に失敗した場合は CPU にフォールバックする。
    pub fn auto() -> Self { /* ... */ }

    /// candle_core::Device に変換する。
    pub fn to_candle_device(self) -> candle_core::Result<Device> { /* ... */ }
}
```

`auto()` の内部実装は次の優先順位で `Device::new_cuda(0)` / `Device::new_metal(0)` を試行する。`Err` は warn ログに記録して破棄し、最終的に `Self` を返す:

1. `cfg(feature = "candle-cuda")` 成立時に `Device::new_cuda(0)` → `Ok` なら `MlDevice::Cuda`
2. `cfg(feature = "candle-metal")` 成立時に `Device::new_metal(0)` → `Ok` なら `MlDevice::Metal`
3. それ以外 → `MlDevice::Cpu`

`Device::cuda_if_available` のような暗黙フォールバック API ではなく明示的に `is_ok()` で判定する理由は、Metal 側に同形 API (`metal_if_available`) が存在せず両バックエンドで判定形を揃えたいため・フォールバック発生を warn ログで明示したいため。

`MlDevice::Cuda` / `Metal` バリアントは feature ゲートを付けず常に enum に存在させる (ゲートすると enum マッチが feature によって変わって扱いが煩雑になるため)。CUDA / Metal 試行で `Err` を返した場合は warn ログを出して `MlDevice::Cpu` に落ちる。

両 feature が同時に enable されている特殊ビルド (実環境では起こらない) では CUDA が先に試される。両 feature 同時 enable のサポートは保証しない (CUDA は macOS では使えず、Metal は macOS 専用のため通常同時 enable しない)。

ログ出力:

- device 選択結果は `tracing::info!` で `"ML device auto-detected: <variant>"` (variant は `Cpu` / `Cuda` / `Metal`)
- `candle-cuda` または `candle-metal` feature が有効なのに当該 GPU device 初期化に失敗してフォールバックした場合は `tracing::warn!("requested {} device unavailable, falling back to CPU: {}", backend, err)` (backend は `cuda` / `metal`、err は元エラーメッセージ)

`src/main.rs` 既定の `logger::init` は WARN 閾値で初期化され、info ログは `--verbose` 指定時 (DEBUG 閾値) のみ表示される。本 issue では device 検出結果は `info!` で残す (常時表示の要否は後続 issue で再検討する)。

`crate::Error` への `From<candle_core::Error>` 実装は本 issue では追加しない。`to_candle_device` は `candle_core::Result<Device>` をそのまま返し、最初に必要になった呼び出し側 (0061 / 0062 / 0064 のいずれか先着) が `src/error.rs` に `#[cfg(feature = "candle")] impl From<candle_core::Error> for crate::Error` を導入する。後発はそれを流用する。

`src/ml/device.rs` の単体テストは同ファイル内の `#[cfg(test)] mod tests` に置く。テストでは `.unwrap()` / `is_ok()` で `candle_core::Error` を扱う (現状 hisui のテスト関数は `crate::Result<()>` 戻り値型を使わないため `?` 演算子は不要)。テスト項目:

- `MlDevice::Cpu.to_candle_device()` が `Ok` を返す (CPU は常に成功するはず)
- `MlDevice::auto()` の戻り値に対して `to_candle_device()` を呼ぶと `Ok` を返す (CI 環境では `candle` のみ有効化なので `auto()` は `MlDevice::Cpu` を返す)
- `#[cfg(feature = "candle-metal")] #[test] fn metal_device_works()` で `MlDevice::Metal.to_candle_device().is_ok()` を確認する (test-apple-toolbox 経由で実行される)
- `#[cfg(feature = "candle-cuda")] #[test] fn cuda_device_works()` で `MlDevice::Cuda.to_candle_device().is_ok()` を確認する (test-nvidia-video-codec 経由で実行される)

### PoC との差分

`feature/try-candle` ブランチ (PR #246) の `src/ml/device.rs` を本 issue へ移植する際に、API シグネチャは次の点で変更されている (= コピーではなく書き直し):

- enum を `Cpu, Metal(usize), Cuda(usize)` のタプルバリアントから `Cpu, Cuda, Metal` のユニットバリアントへ簡素化 (multi-GPU の必要性が出た時点で破壊的変更で戻す)
- `to_candle_device(&self)` を `to_candle_device(self)` に変更 (`Copy` 型のため value 取得で `match` パターンを簡潔にする)
- `auto()` の試行順序を Metal → CUDA → CPU から CUDA → Metal → CPU に変更

### scripts/download_ml_models.py

Python 3.10 以降の標準ライブラリのみ (`urllib.request` / `hashlib` / `argparse` / `pathlib` / `sys`) で実装する。`pyproject.toml` には何も追加しない (依存ゼロ)。`requests` / `httpx` 等のサードパーティ HTTP クライアントは導入しない。CI では既存 maturin 系スクリプトと揃えて `uv run` で起動する。

CLI:

```
uv run scripts/download_ml_models.py --dest <DIR> <TARGET> [<TARGET> ...]
```

- 位置引数 `TARGET` (1 個以上必須): `whisper-tiny` / `silero-vad`
- `--dest <DIR>` (必須): 保存先ディレクトリ。デフォルトを持たない (親 issue 0012 で確定した「`--model-dir <path>` 必須・デフォルトパスを持たない」方針に揃える)
- 環境変数は導入しない

後続の 0061 / 0064 issue 本文で `ml-models/` というパスが登場するが、これは「**呼び出し時の慣例パス**」であり `download_ml_models.py` のデフォルト値ではない。利用側は `--dest ml-models/` を都度明示する。

ターゲット定義はスクリプト先頭の dict として保持する。**このスクリプトは拡張可能な基盤として作る**: 0064 (YOLO) が `yolo` キーで `yolov8s.safetensors` / `yolov8s-pose.safetensors` を追加することを想定している。本 issue では `whisper-tiny` / `silero-vad` の 2 キーのみ定義する。ターゲット名の命名規約は「`<モデル種別>[-<サイズ/バリアント>]` のケバブケース」(例: `whisper-tiny` / `whisper-small` / `silero-vad` / `yolo-v8s`)。

各エントリは `typing.NamedTuple` で型付けする (4 要素タプルでインデックスアクセスすると `entry[3]` の意図が読みづらいため):

```python
class FileSpec(NamedTuple):
    hf_repo: str          # 例: "openai/whisper-tiny"
    file_in_repo: str     # 例: "model.safetensors"
    expected_sha256: str  # 空文字なら検証スキップ (初回ハッシュ取得用)

TARGETS = {
    "whisper-tiny": [
        FileSpec("openai/whisper-tiny", "config.json",       "<sha256>"),
        FileSpec("openai/whisper-tiny", "tokenizer.json",    "<sha256>"),
        FileSpec("openai/whisper-tiny", "model.safetensors", "<sha256>"),
    ],
    "silero-vad": [
        FileSpec("onnx-community/silero-vad", "onnx/model.onnx", "<sha256>"),
    ],
}
```

URL は `https://huggingface.co/{hf_repo}/resolve/main/{file_in_repo}` で組み立てる。保存先パスは `<dest>/<target_key>/<file_in_repo>` の規約で算出する (`silero-vad` キーで `onnx/model.onnx` を取得する場合の保存先は `<dest>/silero-vad/onnx/model.onnx`)。dest_subpath をエントリに持たせない (キーと subpath プレフィックスの二重定義を回避する設計)。

HTTP User-Agent は Hugging Face で `Python-urllib/3.x` の既定が 403 / 429 を返されることがあるため `urllib.request.Request` 経由で `"hisui-download/<hisui-version>"` を設定する (バージョン文字列は `pyproject.toml` から動的取得しなくてよい。`"hisui-download/2026.1"` 等のリテラル固定で OK)。

エラー処理:

- 4xx (`429` 以外) は即時終了コード 3
- `429 Too Many Requests` / 5xx / connection error は最大 3 回リトライ (指数バックオフ 2 / 4 / 8 秒)。3 回失敗で終了コード 3
- SHA256 期待値が空文字 (プレースホルダ) の場合は検証をスキップし、標準エラー出力に warn メッセージを出す (本 issue 実装時の初回ハッシュ取得サイクル用)
- ダウンロード成功後 SHA256 mismatch の場合は、もう 1 ラウンド「ダウンロード (最大 3 回 HTTP リトライ内包) → SHA256 検証」を試行。2 ラウンド目でも mismatch なら終了コード 4
- 既存ファイルがある場合: サイズと SHA256 期待値が一致すれば skip、それ以外は再取得
- ダウンロード中は `<保存先パス>.tmp` に書き、完了後に `os.replace` で atomic rename (Windows 互換)。中断時に残った `.tmp` は次回実行で上書きされる (resume はしない)
- `--dest` ディレクトリが書き込み不可なら起動時に終了コード 5

終了コード規約:

- 0 = success
- 1 = 予期しない exception (Python の uncaught exception 既定)
- 2 = CLI 引数エラー (argparse 既定)
- 3 = ネットワーク失敗
- 4 = SHA256 mismatch
- 5 = ディレクトリパーミッション失敗

進捗表示・proxy 対応は本 issue では行わない (proxy は `urllib.request` の既定挙動 = 環境変数 `HTTP_PROXY` / `HTTPS_PROXY` の自動読込みで対応可能なため追加実装不要)。

SHA256 期待値の初期取得手順と HF 側更新時の対処手順は `scripts/README.md` の `download_ml_models.py` セクションに記載する (実装ステップは「## 解決方法」末尾の「実装ステップ目安」を参照)。スクリプトの単体テストは本 issue 範囲では追加しない (HF を実叩きしないと意味のあるテストにならない)。動作担保は完了条件側の「実装者ローカル実行 + PR コメント報告」で行う。

### scripts/README.md 更新

`scripts/download_ml_models.py` のセクションを既存 `maturin_develop.sh` / `maturin_build.sh` と同じ 4 節構成 (`### 目的` / `### 説明` / `### 使い方` / `### 動作の流れ`) で追加する。SHA256 期待値の初期取得手順と HF 側更新時の対処手順は `### 説明` 節に書く。`### 動作の流れ` は「引数解析 → ターゲット展開 → HTTP GET (リトライ) → SHA256 検証 → atomic rename」程度で簡潔に。

### .gitignore 更新

`/ml-models/` ディレクトリ全体を `.gitignore` に追加する (現状の `*.safetensors` 1 行では `.onnx` / `tokenizer.json` / `config.json` を捕捉できない)。既存の `*.safetensors` 行は残す。

### .github/workflows/ci.yml への test-candle job 追加

本 issue 範囲では実モデルを使わず、ビルドと最小単体テストの確認のみを行う。`actions/cache@v4` を用いた ML モデルキャッシュ・`uv` セットアップ・`uv run scripts/download_ml_models.py` 呼び出しは本 issue では追加しない。これらは 0061 (Silero VAD) / 0062 (Whisper) で test-candle ジョブに段階的に積み増す。

`test-openh264` ジョブの直後・`test-fuzz` ジョブの直前に次のジョブを追加する。初回ビルド (キャッシュ無し) で 20 分超過リスクがあるため `timeout-minutes: 30` を設定:

```yaml
test-candle:
  # 本 job は candle feature 有効ビルドの妥当性 (protoc / onig / candle 依存解決) と
  # 最小 device テストを検証する。0061 で actions/cache@v4 + uv + 実モデル取得を、
  # 0062 で実推論テストを順次積み増す。
  runs-on: ubuntu-24.04
  timeout-minutes: 30
  steps:
    - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
    - run: rustup update stable
    - name: Install packages to build external dependencies
      run: |
        sudo apt-get update
        sudo apt-get install -y meson ninja-build nasm yasm build-essential autoconf automake libtool pkg-config cmake libx11-dev libpulse-dev protobuf-compiler
    - uses: shiguredo/github-actions/.github/actions/rust-cache@main
      with:
        os: ubuntu-24.04
        toolchain: stable
    - run: cargo clippy --features candle --all-targets -p hisui -- --deny warnings
    - run: cargo test --features candle -p hisui
```

apt パッケージ列は既存 `test-fdk-aac` / `test-openh264` ジョブと同等の理由で必要 (`cargo clippy --features candle --all-targets -p hisui` はデフォルト feature の `player` = `raw_player` も同時に有効化するため、その依存ビルドに必要な meson / nasm / libpulse-dev 等が要る)。末尾に `protobuf-compiler` を追加して candle-onnx 用の `protoc` を入れる。


`slack_notify` ジョブの `needs` リストに `test-candle` を `test-openh264` の直後・`test-fuzz` の直前に挿入する (実コードは block-style リストなので 1 行追加する):

```yaml
slack_notify:
  needs:
    - ci
    - test-nvidia-video-codec
    - test-apple-toolbox
    - test-fdk-aac
    - test-openh264
    - test-candle    # ← この行を追加
    - test-fuzz
    - ubuntu-binary
    - macos-binary
```

`ubuntu-binary` / `macos-binary` ジョブの `needs` には `test-candle` を追加しない。

#### test-apple-toolbox への candle-metal ビルド追加

既存 `test-apple-toolbox` ジョブ (self-hosted macOS ARM64) に macOS の Metal 経路を CI 検証する step を追加する。挿入位置は既存 `cargo test --workspace` step (現状の最後) の **直後**。既存の `Install packages to build external dependencies` step の `brew install` 行末に `protobuf` を追記する形にして `brew update` の重複を避ける:

```yaml
- run: cargo test --features candle,candle-metal -p hisui
```

`-p hisui` 指定は candle 系 feature が hisui crate のみが持つためで、`--workspace` にすると pbt / examples の不要なビルドが走る。これにより `MlDevice::Metal` バリアントテスト (`#[cfg(feature = "candle-metal")]` ゲート) が CI で実行される。candle 5 crate の初回ビルドで時間が伸びるため、既存 `timeout-minutes: 15` を `30` に引き上げる。

本 issue 段階ではモデル不要だが、後続 0061 (Silero VAD) で `uv run scripts/download_ml_models.py --dest ml-models/ silero-vad` を、0062 (Whisper) で `whisper-tiny` 取得と実推論テストを test-apple-toolbox 内に積み増す予定。

#### test-nvidia-video-codec への candle-cuda check 追加

既存 `test-nvidia-video-codec` ジョブ (self-hosted CUDA 環境) に CUDA 経路の構文・型検査 step を追加する。挿入位置は既存 `cargo check --features nvcodec -p hisui` step の **直後**:

```yaml
- run: cargo check --features candle,candle-cuda -p hisui
- run: cargo clippy --features candle,candle-cuda -p hisui --all-targets -- --deny warnings
```

self-hosted runner に `protobuf-compiler` を事前インストール済みとする運用とし、CI yaml には apt install step を入れない (現状の test-nvidia-video-codec も apt-get を一切叩かない方針を踏襲)。本 issue マージ前に runner 管理者へ `apt-get install -y protobuf-compiler` の事前実施を依頼する旨を PR 本文に書く。candle 系初回ビルドで時間が伸びるため、既存 `timeout-minutes: 15` を `30` に引き上げる。

本 issue 段階では `cargo check` + `cargo clippy` までだが、後続 0062 (Whisper) / 0064 (YOLO) で必要なら実推論テスト (`cargo test --features candle,candle-cuda -p hisui`) を test-nvidia-video-codec に積み増す検討となる。


### pbt / examples / Python バインディングへの影響

`pbt` クレートには本 issue では candle feature を入れない (0061 で PBT 追加時に pbt 側 `Cargo.toml` に feature を伝播させる)。完了条件の `cargo check --workspace --no-default-features` が green であれば本 issue では追加対応不要 (pbt 側の hisui 参照の `default-features = false` 指定要否はこの完了条件で担保する)。

`examples/*` クレートには candle 関連の example を追加しない。Python バインディング (`pyproject.toml` / `python/`) には変更を加えない。

### CHANGES.md エントリ

本 issue の `[ADD]` エントリは hisui 慣習 (新しいものを上に積む) に従い、`## develop` 配下の既存 `[ADD]` セクション先頭 (= 既存 `[ADD]` エントリ群の直前、現時点の例では `- [ADD] obsws 経由でリアルタイム合成映像に...` の直前) に挿入する:

```
- [ADD] オプション依存として candle 系ライブラリ (candle-core 0.10.2 / candle-nn 0.10.2 / candle-transformers 0.10.2 / candle-onnx 0.10.2 / tokenizers 0.22.0) を追加する
  - `candle` / `candle-cuda` / `candle-metal` feature 配下で有効化する
  - candle-onnx のビルドに `protoc` (Ubuntu の `protobuf-compiler` 等) が必要になる
  - 本リリースのバイナリ配布物には含めない (将来の利用者向けサブコマンドが揃うタイミングで再検討する)
  - @sile
- [ADD] ML モデル取得スクリプト `scripts/download_ml_models.py` を追加する
  - Hugging Face から `whisper-tiny` / `silero-vad` のモデルを取得する標準ライブラリのみの Python スクリプト
  - 起動: `uv run scripts/download_ml_models.py --dest <DIR> <TARGET> [<TARGET> ...]`
  - @sile
```

candle 5 crate を 1 エントリにまとめる根拠は「`candle` feature 1 つを有効化すると 5 crate 全部入る一体の機能セット」のため (既存「依存ライブラリに 1 つ」パターンとは利用者にとっての関心単位が異なる)。`scripts/README.md` / `.gitignore` 更新と `test-candle` job 追加は `CHANGES.md` には載せない (`.md` 変更および利用者非可視のため)。

## 完了条件

- `Cargo.toml` に `candle` / `candle-cuda` / `candle-metal` の 3 feature と 5 つの optional 依存が追加されている
- `src/lib.rs` に `#[cfg(feature = "candle")] pub mod ml;` が追加されている
- `src/ml/mod.rs` (`pub mod device;` の 1 行) と `src/ml/device.rs` (`MlDevice` enum + `auto()` / `to_candle_device()` + 単体テスト 2 件 + Metal/CUDA バリアントテスト) が新規追加されている
- `scripts/download_ml_models.py` が新規追加され、`uv run scripts/download_ml_models.py --dest /tmp/ml-models whisper-tiny silero-vad` でモデルファイル一式が取得できることを実装者ローカルで確認し、生成ファイル一覧と SHA256 を PR コメントで報告している
- `scripts/README.md` に `download_ml_models.py` セクションが既存スクリプトと同じ 4 節構成 (目的 / 説明 / 使い方 / 動作の流れ) で追加されている (SHA256 更新手順は「説明」節に含む)
- `.gitignore` に `/ml-models/` が追加されている
- `.github/workflows/ci.yml` に `test-candle` job が追加され、`slack_notify.needs` にも追加され、`test-apple-toolbox` に candle-metal ビルド step、`test-nvidia-video-codec` に candle-cuda check step が追加されている (両ジョブの `timeout-minutes` も 30 に引き上げ済み)
- 次の実装者ローカル / CI コマンドがすべて green (workspace 系は default feature で実行されるため `src/ml/` は含まれない。`src/ml/` の検査は最後の 2 つ `--features candle -p hisui` 系で担保する):
  - `cargo fmt --all --check`
  - `cargo check --workspace --no-default-features`
  - `cargo clippy --workspace --no-default-features -- --deny warnings`
  - `cargo check --workspace`
  - `cargo clippy --workspace --all-targets -- --deny warnings`
  - `cargo test --workspace`
  - `cargo clippy --features candle --all-targets -p hisui -- --deny warnings`
  - `cargo test --features candle -p hisui`
- `cargo tree -d --features candle -p hisui` で重複依存が発生していないことを確認する。重複があった場合は (a) workspace 共通化や feature 揃えで本 issue 範囲内で解消できるなら解消、(b) candle 系内部の不可避な重複であれば PR 説明にバージョン別 crate を列挙して許容、を判断する
- `CHANGES.md` に上記 2 エントリが `shiguredo-changelog` 規約準拠の書式 (担当者行 2 文字インデント) で追記済み

## 解決方法

`feature/try-candle` ブランチ (PR #246) を参照元とし、次の 4 系統を本 issue 仕様に合わせて移植・書き直しする (`.sh` は `.py` で新規実装、`device.rs` は API 簡素化のため `feature/try-candle` から書き直し):

- `Cargo.toml` の features 3 段と optional 依存 5 件
- `src/ml/mod.rs` (PoC では `pub mod audio; pub mod device; pub mod yolo;` だが本 issue では `pub mod device;` のみ)
- `src/ml/device.rs` (本 issue の API シグネチャに合わせて書き直し、「PoC との差分」節参照)
- `scripts/download_ml_models.sh` → `scripts/download_ml_models.py` への書き直し

他の依存バージョン (tokio / rustls / raw_player / shiguredo_s3 など) は develop の現状を維持し PoC から引きずらない。

PoC から **取り込まない** もの (各取り込み先 issue を併記):

- `src/ml/yolo.rs` および `src/ml/mod.rs` への `pub mod yolo;` 追加・`MlModel` enum 追加 (0064 で取り込む)
- `src/ml/audio/` 配下および `src/ml/mod.rs` への `pub mod audio;` 追加 (0061 / 0062 で取り込む)
- `src/subcommand_ml.rs` (PoC のマイク入力サブコマンド、0064 / 0061 / 0062 で再設計)
- `src/main.rs` への subcommand 追加 (本系列では 0063 で対応)
- `From<candle_core::Error> for crate::Error` impl (0061 / 0062 / 0064 の先着で追加。挿入位置は `src/error.rs` の既存 `#[cfg(feature = "nvcodec")]` impl 直下・`#[cfg(test)] mod tests` の直前)

### 後続 issue 側に必要な追記 (本 issue マージ後の宿題)

本 issue の責務分担を成立させるため、以下を後続 issue 本文 (polish) で追記する必要がある。本 issue のレビュー / マージ時に後続 issue 担当者へ周知する:

- 0061: `src/ml/mod.rs` への `pub mod audio;` 1 行追加、`From<candle_core::Error> for crate::Error` 先着導入 (0062 / 0064 がまだ着手前なら本 issue で導入)
- 0062: 0061 で `From<candle_core::Error>` 未導入なら本 issue で導入
- 0064: `src/ml/mod.rs` への `pub mod yolo;` 1 行追加、`MlModel` enum 追加、`From<candle_core::Error>` 未導入なら本 issue で導入

### 想定 commit 構成

`shiguredo-git` 規約 (1 コミット = 1 論理単位、`{SEQ} {変更内容}` 形式) に従い、次の 4 commit で構成する:

1. `0059 candle feature と device 検出骨格を追加する` (Cargo.toml / src/lib.rs / src/ml/{mod,device}.rs + tests)
2. `0059 ML モデル取得スクリプトを追加する` (scripts/download_ml_models.py / scripts/README.md / .gitignore)
3. `0059 CI に test-candle ジョブと Metal/CUDA ビルド検証を追加する` (.github/workflows/ci.yml)
4. `0059 CHANGES.md に candle 依存追加と download_ml_models.py のエントリを追加する` (CHANGES.md)

### 実装ステップ目安

1. 事前確認: candle-core / candle-onnx 0.10.2 と tokenizers 0.22.0 の `rust-version` (MSRV) が `1.95` 以下であることと、yank されていないことを `cargo search` および各 crate の `Cargo.toml` で確認
2. `Cargo.toml` に features と optional 依存 5 件を追加
3. `src/lib.rs` に `#[cfg(feature = "candle")] pub mod ml;` を追加 (この時点で `ml` モジュール未作成のためコンパイル不可、cfg gate 忘れを早期検出)
4. `src/ml/mod.rs` を新規作成 (`pub mod device;` 1 行)
5. `src/ml/device.rs` を新規作成 (`MlDevice` enum + `auto()` + `to_candle_device()` + tests)
6. `scripts/download_ml_models.py` を新規作成 (`TARGETS` の `expected_sha256` を空文字で初期化)
7. ローカルで `uv run scripts/download_ml_models.py --dest /tmp/ml-models whisper-tiny silero-vad` を実行し、取得ファイルの SHA256 を `sha256sum` で取得して `TARGETS` dict に埋め込み、再実行して mismatch しないことを確認
8. `scripts/README.md` 追記、`.gitignore` 更新
9. `.github/workflows/ci.yml` 更新 (test-candle ジョブ追加、slack_notify.needs 追加、test-apple-toolbox と test-nvidia-video-codec への step 追加、両 self-hosted ジョブの timeout-minutes を 30 に引き上げ)
10. すべての完了条件コマンドを green に (test-apple-toolbox / test-nvidia-video-codec への変更は self-hosted ジョブのため CI 結果は PR push 後に初検証となる旨を想定しておく)
11. `CHANGES.md` に 2 エントリを追加
12. 上記「想定 commit 構成」の 4 commit に分けて push、PR 作成 (self-hosted ジョブの事前準備 [test-nvidia-video-codec への `protobuf-compiler` 事前インストール依頼] を PR 本文に明記)
