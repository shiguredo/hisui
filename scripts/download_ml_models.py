#!/usr/bin/env python3
"""Hugging Face から ML モデルを取得するスクリプト。

利用例:
    uv run scripts/download_ml_models.py --dest ml-models/ whisper-tiny silero-vad

ターゲット名はファイル先頭の TARGETS dict のキーを指定する。
"""

from __future__ import annotations

import argparse
import hashlib
import os
import sys
import urllib.request
from pathlib import Path
from typing import NamedTuple


class FileSpec(NamedTuple):
    """ターゲット 1 つに含まれる個別ファイルの仕様。

    Attributes:
        hf_repo: Hugging Face リポジトリ名 (例: ``openai/whisper-tiny``)
        file_in_repo: リポジトリ内のファイルパス (例: ``model.safetensors``)
        expected_sha256: 期待される SHA256 値。空文字なら検証をスキップする
    """

    hf_repo: str
    file_in_repo: str
    expected_sha256: str


# 取得対象モデルの定義。
# キーは「<モデル種別>[-<サイズ/バリアント>]」のケバブケース。
# 保存先パスは <dest>/<target_key>/<file_in_repo> の規約で算出する。
TARGETS: dict[str, list[FileSpec]] = {
    "whisper-tiny": [
        FileSpec(
            "openai/whisper-tiny",
            "config.json",
            "ffdccec4f3211f4c63310f2b7098f309fe70f3952cedc5e4d11e43f5b2379b98",
        ),
        FileSpec(
            "openai/whisper-tiny",
            "tokenizer.json",
            "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566",
        ),
        FileSpec(
            "openai/whisper-tiny",
            "model.safetensors",
            "7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395",
        ),
    ],
    "silero-vad": [
        FileSpec(
            "onnx-community/silero-vad",
            "onnx/model.onnx",
            "a4a068cd6cf1ea8355b84327595838ca748ec29a25bc91fc82e6c299ccdc5808",
        ),
    ],
}

HF_BASE_URL = "https://huggingface.co"
HTTP_TIMEOUT_SECONDS = 60


# 終了コード規約 (Python 既定の 1 / argparse 既定の 2 は予約として README で説明する)
EXIT_SUCCESS = 0
EXIT_SHA256_MISMATCH = 4
EXIT_DIR_NOT_WRITABLE = 5


def build_url(spec: FileSpec) -> str:
    """HF の resolve URL を組み立てる。"""
    return f"{HF_BASE_URL}/{spec.hf_repo}/resolve/main/{spec.file_in_repo}"


def compute_sha256(path: Path) -> str:
    """ファイルの SHA256 値を算出する。"""
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download_to(url: str, dest_tmp: Path) -> None:
    """単発ダウンロード。失敗時は urllib の例外をそのまま投げる。

    開発者が手で実行する想定のため自前のリトライ・バックオフは持たない。
    失敗したら再実行で続きから取れる (取得済みファイルは SHA256 で skip される)。
    """
    with (
        urllib.request.urlopen(url, timeout=HTTP_TIMEOUT_SECONDS) as response,
        dest_tmp.open("wb") as out,
    ):
        while True:
            chunk = response.read(1024 * 1024)
            if not chunk:
                break
            out.write(chunk)


def fetch_one(spec: FileSpec, dest_dir: Path, target_key: str) -> None:
    """ターゲット 1 ファイルを取得する。既存ファイルの skip 判定も行う。"""
    url = build_url(spec)
    dest_path = dest_dir / target_key / spec.file_in_repo
    dest_path.parent.mkdir(parents=True, exist_ok=True)

    # 既存ファイルの SHA256 が期待値と一致するなら skip する。
    if dest_path.exists() and spec.expected_sha256:
        if compute_sha256(dest_path) == spec.expected_sha256:
            print(f"  skip (already up-to-date): {dest_path}")
            return

    dest_tmp = dest_path.with_suffix(dest_path.suffix + ".tmp")
    print(f"  downloading {url}")
    try:
        download_to(url, dest_tmp)
    except BaseException:
        dest_tmp.unlink(missing_ok=True)
        raise

    # SHA256 期待値が空ならスキップ (初回ハッシュ取得用)、
    # mismatch なら .tmp を片付けて即終了する (HF 側更新が原因なら再取得しても無意味)。
    if spec.expected_sha256:
        actual = compute_sha256(dest_tmp)
        if actual != spec.expected_sha256:
            print(
                f"  SHA256 mismatch (expected={spec.expected_sha256}, actual={actual})",
                file=sys.stderr,
            )
            dest_tmp.unlink(missing_ok=True)
            sys.exit(EXIT_SHA256_MISMATCH)
    else:
        actual = compute_sha256(dest_tmp)
        print(
            f"  WARN: expected_sha256 is empty, skipping verification (actual sha256={actual})",
            file=sys.stderr,
        )

    # Windows 互換のために os.replace を使う (POSIX rename は既存上書きするが
    # Windows では失敗するため)。
    os.replace(dest_tmp, dest_path)
    print(f"  saved: {dest_path}")


def ensure_writable(dest: Path) -> None:
    """--dest が書き込み可能か起動時に確認する。不可なら終了コード 5 で終了。"""
    try:
        dest.mkdir(parents=True, exist_ok=True)
    except OSError as err:
        print(f"failed to create --dest directory: {err}", file=sys.stderr)
        sys.exit(EXIT_DIR_NOT_WRITABLE)
    if not os.access(dest, os.W_OK):
        print(f"--dest directory is not writable: {dest}", file=sys.stderr)
        sys.exit(EXIT_DIR_NOT_WRITABLE)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Download ML models from Hugging Face for hisui.",
    )
    parser.add_argument(
        "--dest",
        type=Path,
        required=True,
        help="destination directory (required)",
    )
    parser.add_argument(
        "targets",
        nargs="+",
        choices=sorted(TARGETS.keys()),
        help="one or more target names",
    )
    args = parser.parse_args()

    ensure_writable(args.dest)

    for target_key in args.targets:
        print(f"target: {target_key}")
        for spec in TARGETS[target_key]:
            fetch_one(spec, args.dest, target_key)

    return EXIT_SUCCESS


if __name__ == "__main__":
    sys.exit(main())
