# Python SDK (hisui_sdk) を追加する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-python-sdk
- Polished:

## 目的

Python から hisui を制御し、文字起こしや統計取得、メディア入出力の操作を簡単に行えるようにする。

## pending とした理由

- 現状の Python パッケージは `subprocess` で CLI を呼ぶ薄いラッパーであり、SDK として何を提供するか (JSON-RPC 経由 / WebRTC 経由 / 単なる CLI ラッパー拡張) の設計が必要。
- Media Pipeline のフック (別 pending issue) との関係を整理する必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `python/hisui/__init__.py` の `Hisui` / `HisuiError` が `subprocess` で `hisui` バイナリを呼び、`inspect` / `list_codecs` を提供する。
- `pyproject.toml` は `build-backend = "maturin"`、`bindings = "bin"` で、パッケージ名は `hisui`。`hisui_sdk` という名前のパッケージは無い。
- `.github/workflows/pypi-publish.yml` と `pytest.yml` が存在する。

## 設計方針 (要検討・未確定)

- JSON-RPC 2.0 または WebRTC 経由で hisui とやりとりする SDK を設計する。
- 文字起こし結果や統計を Python から扱えるようにする。
- 既存の CLI ラッパーとの関係 (拡張か別パッケージか) を決める。

## 完了条件

- Python から入出力や統計を操作できること。

(解決方法は pending のため、設計確定後に記載する。)
