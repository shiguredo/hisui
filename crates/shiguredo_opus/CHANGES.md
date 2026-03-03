# 変更履歴

- UPDATE
  - 後方互換がある変更
- ADD
  - 後方互換がある追加
- CHANGE
  - 後方互換のない変更
- FIX
  - バグ修正

## develop

- [FIX] Opus のシンボル衝突回避処理を安定化する
  - `libopus` の定義済みグローバルシンボルをビルド時に直接 `shiguredo_opus_` プレフィックスへ置換して衝突を回避する
