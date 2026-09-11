# NDI に対応する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-ndi
- Polished:

## 目的

NDI を hisui のメディア入出力として追加する。

## pending とした理由

- NDI SDK のライセンスと依存追加の可否を判断する必要がある。
- 入出力どちらを対象にするか、ハードウェア要件をどうするかを決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/` に NDI 関連の実装は無い。
- README の対応機能にも含まれていない。

## 設計方針 (要検討・未確定)

- NDI SDK を利用した入力ソースと出力を追加する。
- SDK をオプション feature として扱うか、必須依存にするかを決める。

## 完了条件

- NDI 経由のメディア入出力ができること。

(解決方法は pending のため、設計確定後に記載する。)
