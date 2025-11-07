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
