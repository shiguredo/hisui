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

- [FIX] Opus の公開シンボルが他ライブラリと衝突する問題を修正する
  - `opus_` / `celt_` / `clt_` / `silk_` で始まるグローバルシンボルを `shiguredo_opus_` プレフィックスにリネームしてビルドする
