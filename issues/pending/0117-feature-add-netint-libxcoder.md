# NETINT Quadra Libxcoder に対応する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-netint-libxcoder
- Polished:

## 目的

NETINT Quadra をハードウェアエンコーダー / デコーダーとして利用できるようにする。

## pending とした理由

- 実機と libxcoder の入手・ライセンス確認、依存追加の判断が必要。
- 対応するコーデックと、既存のエンコーダー / デコーダー interface への組み込み方を決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/encoder/` (`libvpx.rs` / `openh264.rs` / `svt_av1.rs` / `video_toolbox.rs` / `nvcodec.rs`) と `src/decoder/` (`libvpx.rs` / `openh264.rs` / `dav1d.rs` / `video_toolbox.rs` / `nvcodec.rs`) に NETINT 関連の実装は無い。
- README の優先実装が可能な機能一覧に記載があるのみ。

## 設計方針 (要検討・未確定)

- libxcoder を利用したエンコーダー / デコーダーを追加する。
- オプション feature として扱い、ビルド要件を整理する。

## 完了条件

- NETINT Quadra でエンコード / デコードができること。

(解決方法は pending のため、設計確定後に記載する。)
