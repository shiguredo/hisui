# Scripts

このディレクトリには、開発とビルドを支援するためのスクリプトが含まれています。

## maturin_develop.sh

### 目的

ローカル開発環境で `uv run maturin develop` を実行するためのラッパースクリプト

### 説明

Cargo.toml のバージョンが `-canary.X` 形式を使用している場合、Python/maturin との互換性の問題が発生します。このスクリプトは、**`-canary.X` がある場合のみ**、一時的にバージョンを `-dev.X` 形式に変換してから `uv run maturin develop` を実行し、完了後に元のバージョンに戻します。通常のバージョン（例: `2025.3.0`）の場合は変換せずにそのまま実行します。

### 使い方

```bash
# 通常の開発ビルド
./scripts/maturin_develop.sh

# リリースモードでビルド
./scripts/maturin_develop.sh --release
```

### 動作の流れ

1. Cargo.toml から現在のバージョンを読み取る
2. **バージョンに `-canary.` が含まれている場合のみ**:
   - Cargo.toml をバックアップ
   - `-canary.` を `-dev.` に置換
   - `uv run maturin develop` を実行
   - 元のバージョンに復元
3. **バージョンに `-canary.` が含まれていない場合**:
   - 変換せずにそのまま `uv run maturin develop` を実行

## maturin_build.sh

### 目的

GitHub Actions などの CI 環境で `maturin build` を実行するためのラッパースクリプト

### 説明

**`-canary.X` がある場合のみ**、Cargo.toml のバージョンを Python/maturin 互換の形式に変換してから `maturin build` を実行します。通常のバージョン（例: `2025.3.0`）の場合は変換せずにそのまま実行します。このスクリプトは元のファイルを復元しません（CI 環境では必要ないため）。

### 使い方

```bash
# 通常のビルド
./scripts/maturin_build.sh

# リリースモードでビルド
./scripts/maturin_build.sh --release
```

### 動作の流れ

1. Cargo.toml から現在のバージョンを読み取る
2. **バージョンに `-canary.` が含まれている場合のみ**:
   - `-canary.` を `-dev.` に置換
3. `uv run maturin build` を実行（変換の有無に関わらず）

**GitHub Actions での使用例**:

```yaml
- name: Build wheel with Maturin
  run: ./scripts/maturin_build.sh --release
```

## download_ml_models.py

### 目的

Hugging Face から ML 推論用モデルファイル（Whisper / Silero VAD 等）を取得するためのスクリプト

### 説明

`candle` feature 配下の ML 推論機能を動かすには、`whisper-tiny` / `silero-vad` などのモデルファイルを Hugging Face から取得して `--dest` で指定したディレクトリに配置する必要があります。本スクリプトは標準ライブラリのみ（追加の Python 依存なし）で次を行います。

- ターゲット名（`whisper-tiny` / `silero-vad` 等）ごとに HF の `resolve/main` URL から複数ファイルを取得
- ファイルごとに SHA256 を検証（期待値はスクリプト先頭の `TARGETS` dict に埋め込み済み）
- 既に保存済みで SHA256 が一致しているファイルは skip
- ダウンロード中は `.tmp` 一時ファイルに書き込み、完了後に atomic rename
- 自前のリトライ・バックオフは持たない（開発者が手で叩く想定。失敗時は再実行で続きから取れる）

**HF 側でモデルが更新された場合**: CI が SHA256 mismatch で落ちて気付くので、最新ファイルを手元で再取得して `sha256sum` で値を取り直し、`TARGETS` dict を更新する PR を出すこと。`expected_sha256` を空文字に戻して 1 回実行すると検証スキップで再取得され、`sha256sum` で値が取れます。

**新規ターゲットを追加する場合**: `TARGETS` dict に `<モデル種別>[-<サイズ/バリアント>]` のケバブケースのキーで `FileSpec` のリストを追加してください。保存先は `<dest>/<target_key>/<file_in_repo>` 規約で算出されます。

### 使い方

```bash
# 既定の取得対象
uv run scripts/download_ml_models.py --dest ml-models/ whisper-tiny silero-vad

# whisper-tiny だけ取得
uv run scripts/download_ml_models.py --dest ml-models/ whisper-tiny
```

`--dest` は必須でデフォルト値を持ちません。`ml-models/` は `.gitignore` 済みのパスです。

### 動作の流れ

1. CLI 引数を解析（`--dest` 必須、ターゲット名は `TARGETS` dict のキーから選択）
2. `--dest` の存在と書き込み権限を確認
3. ターゲット内の各 `FileSpec` について URL を組み立て、HTTP GET
4. SHA256 を検証（期待値が空ならスキップして warn 出力）、mismatch なら即終了（HF 側更新が原因なら再取得しても無意味）
5. `.tmp` ファイルから本来のパスへ `os.replace` で atomic rename

終了コード:

- 0 = 成功
- 1 = 予期しない exception（ネットワーク失敗含む）
- 2 = CLI 引数エラー
- 4 = SHA256 mismatch
- 5 = `--dest` ディレクトリが書き込み不可

## バージョン形式について

### Cargo (Rust) のバージョン形式

- 例: `2025.3.0-canary.0`
- SemVer 準拠
- プレリリースは `-` で区切る

### Python のバージョン形式

- 例: `2025.3.0-dev.0`
- PEP 440 準拠
- maturin は -dev.0 を自動で .dev0 に変換する

### 変換ルール

- `-canary.X` -> `-dev.X`
- この変換により、Cargo と Python の両方で有効なバージョン形式を維持

## 注意事項

- これらのスクリプトは macOS と Linux の両方で動作します
- `uv` と `maturin` がインストールされている必要があります
- スクリプトは Cargo.toml がプロジェクトルートにあることを前提としています
