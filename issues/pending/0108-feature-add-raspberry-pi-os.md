# Raspberry Pi OS に対応する

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/add-raspberry-pi-os
- Polished:

## 目的

Raspberry Pi Zero 2 W などの省電力デバイスで hisui を動かせるようにする。

## pending とした理由

- ARM (32 bit / 64 bit) 向けのビルドと、利用可能な依存ライブラリ・コーデックの整理が必要。
- 実機でのパフォーマンス検証が必要。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- README の動作環境は Ubuntu 22.04 / 24.04 (x86_64 / arm64) と macOS。
- Raspberry Pi OS 向けのビルド手順・配布物は無い。

## 設計方針 (要検討・未確定)

- Raspberry Pi OS (armhf / arm64) 向けのビルドを用意する。
- 利用する software コーデックとハードウェア支援の範囲を決める。
- 実機で入出力と合成の動作を確認する。

## 完了条件

- Raspberry Pi OS 上で主要機能が動作すること。
- ビルド手順と動作環境がドキュメント化されていること。

(解決方法は pending のため、設計確定後に記載する。)
