# CI のバイナリアップロード分離と依存ビルドキャッシュを検討する

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/improve-ci-binary-cache
- Polished:

## 目的

CI のテスト自体は短時間で終わるが、バイナリビルドとアップロードに時間がかかっている。バイナリアップロードを分離するか、依存ライブラリのビルドをキャッシュして CI 全体を短縮する。

## pending とした理由

- ワークフローの再構成 (分離かキャッシュか) とキャッシュ戦略の設計が必要。
- どのジョブを対象にし、キャッシュキーをどう設計するかを決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `.github/workflows/ci.yml` の `ubuntu-binary` は x86_64 のみ有効 (arm64 は無効化)、`macos-binary` は複数バージョンを対象に `actions/upload-artifact` で `.tar.gz` を artifact 化する。
- `rust-cache` は `ci` / `test-fdk-aac` / `test-openh264` / `test-candle` / `ubuntu-binary` / `macos-binary` で使われるが、`test-nvidia-video-codec` / `test-apple-toolbox` と `release.yml` の各ジョブでは使われていない。
- 外部 C/C++ ライブラリ (libvpx / OpenH264 / SVT-AV1 等) 専用のキャッシュステップは無く、OpenH264 は毎回ダウンロードしている。
- ML モデル (Silero VAD / Whisper tiny) のみ `actions/cache` で明示的にキャッシュしている。

## 設計方針 (要検討・未確定)

- バイナリアップロードを専用ワークフローに分離する。
- 依存ライブラリのビルド成果物をキャッシュする (対象とキーを設計する)。
- キャッシュ導入で CI の所要時間が実際に短縮されるか計測する。

## 完了条件

- バイナリビルドを含む CI の所要時間が短縮されること。
- キャッシュの不整合でテストが失敗しないこと。

(解決方法は pending のため、設計確定後に記載する。)
