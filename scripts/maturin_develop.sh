#!/bin/bash
# maturin develop を実行する前に Cargo.toml のバージョンを Python 互換に変換するスクリプト
#
# 使い方:
#   ./scripts/maturin_develop.sh
#   ./scripts/maturin_develop.sh --release

set -e

# 元のバージョンを取得
ORIGINAL_VERSION=$(grep '^version = ' Cargo.toml | cut -d'"' -f2)
echo "Original Cargo.toml version: $ORIGINAL_VERSION"

# -canary. を含む場合のみ変換が必要
if [[ "$ORIGINAL_VERSION" == *"-canary."* ]]; then
    # バックアップを作成
    cp Cargo.toml Cargo.toml.bak
    if [ -f Cargo.lock ]; then
        cp Cargo.lock Cargo.lock.bak
    fi

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

    # maturin develop を実行（引数をそのまま渡す）
    echo "Running: maturin develop $@"
    maturin develop "$@"
    RESULT=$?

    # 元のバージョンに戻す
    echo "Restoring original version: $ORIGINAL_VERSION"
    mv Cargo.toml.bak Cargo.toml
    if [ -f Cargo.lock.bak ]; then
        mv Cargo.lock.bak Cargo.lock
    fi

else
    # 変換不要な場合はそのまま実行
    echo "No conversion needed, running maturin develop directly"
    uv run maturin develop "$@"
    RESULT=$?
fi

if [ $RESULT -eq 0 ]; then
    echo "✅ Development environment ready!"
else
    echo "❌ maturin develop failed with exit code $RESULT"
fi

exit $RESULT