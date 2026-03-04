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

- [UPDATE] cmake の代わりに shiguredo_cmake を利用する
- [FIX] Opus のシンボル衝突回避処理を安定化する
  - 静的ライブラリの `libopus` の定義済みグローバルシンボルに `shiguredo_opus_` プレフィックスを付与して衝突を回避する
