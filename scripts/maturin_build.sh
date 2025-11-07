#!/bin/bash
# maturin build を実行する前に Cargo.toml のバージョンを Python 互換に変換するスクリプト
#
# 使い方:
#   ./scripts/maturin_build.sh
#   ./scripts/maturin_build.sh --release

set -e

# 元のバージョンを取得
ORIGINAL_VERSION=$(grep '^version = ' Cargo.toml | cut -d'"' -f2)
echo "Original Cargo.toml version: $ORIGINAL_VERSION"

# -canary. を含む場合のみ変換が必要
if [[ "$ORIGINAL_VERSION" == *"-canary."* ]]; then
    # -canary. を -dev. に変換
    PYTHON_VERSION="${ORIGINAL_VERSION//-canary./-dev.}"
    echo "Converting to Python-compatible version: $PYTHON_VERSION"

    # macOS と Linux の両方で動作する sed コマンド
    if [[ "$OSTYPE" == "darwin"* ]]; then
        sed -i '' "s/^version = \"$ORIGINAL_VERSION\"/version = \"$PYTHON_VERSION\"/" Cargo.toml
        if [ -f Cargo.lock ]; then
            sed -i '' "s/^version = \"$ORIGINAL_VERSION\"/version = \"$PYTHON_VERSION\"/" Cargo.lock
        fi
    else
        sed -i "s/^version = \"$ORIGINAL_VERSION\"/version = \"$PYTHON_VERSION\"/" Cargo.toml
        if [ -f Cargo.lock ]; then
            sed -i "s/^version = \"$ORIGINAL_VERSION\"/version = \"$PYTHON_VERSION\"/" Cargo.lock
        fi
    fi

    echo "Updated Cargo.toml version:"
    grep '^version = ' Cargo.toml
fi

# maturin build を実行（引数をそのまま渡す）
echo "Running: maturin build $@"
uv run maturin build "$@"
RESULT=$?

if [ $RESULT -eq 0 ]; then
    echo "✅ Build completed successfully!"
else
    echo "❌ maturin build failed with exit code $RESULT"
fi

exit $RESULT