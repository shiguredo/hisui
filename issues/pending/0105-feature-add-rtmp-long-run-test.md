# ffmpeg / OBS で RTMP の長時間配信テストを追加する

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/add-rtmp-long-run-test
- Polished:

## 目的

RTMP のタイムスタンプはミリ秒を 32 bit で表現し、24 bit の範囲を超えると拡張タイムスタンプに切り替わる。この境界付近の挙動は仕様から読み取りにくい部分があり、実際に拡張タイムスタンプが使われる長時間配信で確認する。

## pending とした理由

- 長時間 (分〜時間オーダー) のテストは CI の常時実行に向かず、実行環境と運用方法 (定期実行・手動実行) の検討が必要。
- 送信側を ffmpeg / OBS のどちらで担うか、確認するメトリクスを何にするかを決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/timestamp/mapper.rs` の `TimestampMapper` / `WrappingTimestampNormalizer` が 32 bit wrap を補正し、`normalizer_handles_32bit_wrap` などのテストを持つ。ただしこれは mapper 単体のテストで、RTMP 経路の結合テストではない。
- RTMP の Rust 統合テスト (`tests/rtmp_inbound_endpoint_tests.rs` / `tests/rtmp_outbound_endpoint_tests.rs` / `tests/rtmp_publisher_tests.rs`) は `new` のバリデーション中心で、`run()` を長時間動かしていない。
- `e2e-tests/obsws/` の RTMP テストは ffmpeg を相手にした数十秒オーダー。
- `src/timestamp/mapper.rs` のコメントに「RTMP (32 bit / 1 kHz) はおよそ 584 年」とあるが、2^32 ミリ秒は約 49.7 日であり、長時間配信の観点では実態と乖離している。

## 設計方針 (要検討・未確定)

- ffmpeg / OBS を送信側にした長時間テストを、手動または定期実行ワークフローとして追加する。
- 24 bit 境界を跨ぐタイムスタンプで送受信が破綻しないことを、ログまたはメトリクスで確認する。
- タイムスタンプのコメントの誤りも合わせて確認する。

## 完了条件

- 24 bit を超えるタイムスタンプで RTMP の送受信が破綻しないことを確認できること。
- 長時間テストの実行方法がドキュメント化されていること。

(解決方法は pending のため、設計確定後に記載する。)
