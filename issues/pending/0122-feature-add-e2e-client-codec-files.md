# 各クライアント/コーデックの録画ファイルで E2E を追加する

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/add-e2e-client-codec-files
- Polished:

## 目的

合成の失敗は入力ストリームとの相性に起因することが多い。各クライアント・各コーデックの録画ファイルを一通り揃え、E2E で定期的に確認できるようにする。デコード以降の処理は入力に依存しにくいため、`inspect` 相当までの検証で十分。

## pending とした理由

- 各クライアント / コーデックのテスト用入力ファイルの収集と配置の設計が必要。
- 検証範囲を `inspect` 相当に絞る場合の期待値の持ち方を決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- Rust の E2E は `tests/e2e.rs` の inspect 検証 (`inspect_mp4_without_decode` / `inspect_mp4_with_decode` / fMP4 系 / 解像度変化系) が中心。
- obsws の Python E2E (`e2e-tests/obsws/`) は `red-320x320-h264-aac.mp4` と `beep-aac-audio.mp4` を主な入力にしている。
- `testdata/` に `archive-red-320x320-{h264,vp9,h265,av1}.mp4` や `archive-blue-640x480-*`、解像度変化ファイルがあるが、inspect テストでのみ使用されている。
- 既存 open `0007` は合成後の映像・音声の中身検証を扱う (本 issue とは範囲が異なる)。

## 設計方針 (要検討・未確定)

- クライアント / コーデックごとの入力ファイルを `testdata/` に揃え、出自を `testdata/README.md` に記録する。
- `inspect` 相当の検証を E2E として回す。
- 既存 `0007` (中身検証) との役割分担を決める。

## 完了条件

- 主要な入力パターンが E2E で検証されること。
- 入力ファイルの出自と再生成方法が記録されていること。

(解決方法は pending のため、設計確定後に記載する。)
