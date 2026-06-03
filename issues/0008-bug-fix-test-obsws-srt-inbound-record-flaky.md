# e2e-test obsws/test_output.py::test_obsws_srt_inbound_start_record_and_inspect_output がフレーキー

- Priority: Low
- Created: 2026-05-29
- Completed:
- Model: Opus 4.7
- Branch: feature/fix-test-obsws-srt-inbound-record-flaky
- Polished: 2026-05-29

## 目的

E2E テスト `obsws/test_output.py::test_obsws_srt_inbound_start_record_and_inspect_output` が CI で稀に失敗する。録画停止後に生成された MP4 ファイルを `hisui inspect` で読み出した結果、出力 JSON に `video_codec` と `video_sample_count` フィールドが含まれず、ヘルパー側の必須キー検査でアサート失敗する。ランタイムメトリクスを見る限り mp4_writer は 24 サンプル分の映像を受信・書き込みしており、トラック秒数も 0.799992 秒分計上されているため、**「ランタイム上は書き込み成功しているが、生成された MP4 を後から読むと映像トラックが読み出せない」** という構造的レースの可能性が高い。本 issue ではまず原因切り分けと観測のために起票する。

## 優先度根拠

Low とする。

- 観測例は 2026-05-29 の 1 件のみ。同 workflow の直近 13 件 (`gh run list --workflow "E2E Test" --limit 15`) はすべて成功で、連続失敗には至っていない。
- 失敗テストは録画 + inspect の E2E で、本番運用にも実害は生じうるが、現状の頻度は低くリリースを止める段階ではない。
- 1 件のログだけで構造的原因が断定できておらず、まず観測・切り分けが必要。実装を確定させるフェーズではない。
- High にしない理由: develop で連続失敗していない。観測 1 件で頻度のトレンドが立っていない。
- Medium にしない理由: 同上。観測例の蓄積が増えた段階で格上げする (エスカレーション基準参照)。

## 現状

### 失敗内容

失敗箇所: `e2e-tests/obsws/helpers.py:452` 付近 (inspect 出力の必須キー検査)

失敗時のアサートメッセージ:

```
AssertionError: inspect output missing required keys: missing_keys=['video_codec', 'video_sample_count'], output={'path': '/tmp/pytest-of-runner/pytest-0/test_obsws_srt_inbound_start_r0/obsws-record-1780035651665.mp4', 'format': 'mp4'}
```

inspect 出力に `video_codec` と `video_sample_count` の両方が欠落しており、JSON 上は `path` と `format` の 2 フィールドしか含まれていない。

### 失敗モード2 (2026-06-02 観測)

同一テストだが、モード1 (inspect の必須キー欠落) より**前段**で失敗している。

失敗箇所: `e2e-tests/obsws/test_output.py:949`

失敗時のアサートメッセージ:

```
AssertionError: record did not write video samples in time for srt_inbound
```

該当箇所はテスト本体で、StartRecord 後に ffmpeg で SRT push を開始し、`/metrics` を 0.2 秒間隔で最大 30 回 (= 約 6 秒) ポーリングして `hisui_total_video_sample_count{processor_id="output:record:mp4_writer:0"}` が正値になるのを待つループ。この待機が時間内に成立せずタイムアウトしている。

つまりモード1 が「書き込みは成立したが後から MP4 を読めない」のに対し、モード2 は「そもそも待機時間内に映像サンプルが 1 つも書き込まれなかった」という別症状。SRT inbound 経由の映像が CI 環境で約 6 秒以内に mp4_writer まで届かなかった (ffmpeg SRT push の確立遅延、SRT ハンドシェイク/接続待ち、ランナー負荷など) 可能性が高い。待機上限 (30 回 × 0.2 秒 = 6 秒) が CI 環境のばらつきに対して短い可能性も切り分け対象。

### 失敗モード1 のランタイムメトリクス (2026-05-29 run の `/metrics` ダンプより抜粋)

書き込み経路は **すべてのフェーズで映像フレームを処理している** ことが分かる:

```
hisui_total_input_video_frame_count{processor_id="input:srt_inbound:...", processor_type="srt_inbound_endpoint"} 27
hisui_total_input_video_frame_count{processor_id="program:video_mixer",   processor_type="video_mixer"}          27
hisui_total_input_video_frame_count{processor_id="output:record:video_encoder:0", processor_type="video_encoder"} 24
hisui_total_output_video_frame_count{processor_id="output:record:video_encoder:0", processor_type="video_encoder"} 24
hisui_total_received_video_data_count{processor_id="output:record:mp4_writer:0",  processor_type="hybrid_mp4_writer"} 24
hisui_total_video_sample_count{processor_id="output:record:mp4_writer:0",         processor_type="hybrid_mp4_writer"} 24
hisui_total_output_video_keyframe_count{processor_id="output:record:video_encoder:0", processor_type="video_encoder"} 1
hisui_total_video_track_seconds{processor_id="output:record:mp4_writer:0", processor_type="hybrid_mp4_writer"} 0.799992
hisui_video_codec{processor_id="output:record:mp4_writer:0", processor_type="hybrid_mp4_writer", value="H264"} 1
hisui_total_recovery_moov_update_count{processor_id="output:record:mp4_writer:0", processor_type="hybrid_mp4_writer"} 63
```

要点:

- `hisui_video_codec` ラベル `value="H264"` で映像コーデックは特定済み。
- `hisui_total_received_video_data_count = 24` / `hisui_total_video_sample_count = 24` で mp4_writer は 24 サンプル受け取り、書き込みを行った状態。
- `hisui_total_video_track_seconds = 0.799992` で video track の duration も 0.8 秒近くまで進んだ計上がある。
- `hisui_total_recovery_moov_update_count = 63` と高頻度で moov 更新が走っており、hybrid_mp4_writer の recovery 機構が頻繁にトリガーされていた様子。

つまり **encoded video は mp4_writer まで届いて書き込み計上はされているが、最終的に inspect から見える MP4 では `moov.trak[video]` が読み出せない状態** という乖離がある。

### 観測済み失敗事例

| 発生時刻 (UTC) | ジョブ | ブランチ | 失敗箇所 | 失敗モード | run / job |
| --- | --- | --- | --- | --- | --- |
| 2026-05-29T06:16Z | E2E Test → e2e-test | develop (HEAD `902385d2`) | `helpers.py:452` | モード1: inspect 出力に `video_codec` / `video_sample_count` が欠落 | <https://github.com/shiguredo/hisui/actions/runs/26621510837> |
| 2026-06-02T06:03Z | E2E Test → e2e-test | develop (HEAD `f488f266`) | `test_output.py:949` | モード2: `hisui_total_video_sample_count` が時間内に正値にならず待機タイムアウト | <https://github.com/shiguredo/hisui/actions/runs/26801672139> |

### 仮説 (現時点では未確定)

ログだけからは構造原因を断定できないため、複数の仮説を並べておく。**いずれも検証が必要**。

- **仮説 1: hybrid_mp4_writer の finalize と inspect 実行の race**
  - hybrid_mp4_writer は moov を逐次的に再構成 (`recovery_moov_update_count`) する設計のため、StopRecord 直後の最終 moov 書き込みが完了するより前にテスト側で inspect が走ると、video trak エントリが含まれない中間状態の moov を読む可能性。
- **仮説 2: 最終 moov 構築時に video trak の SampleEntry が確定していなかった**
  - 何らかの理由 (SRT 入力の先頭が映像不在で始まった、Sample Entry の確定が遅れた) で moov に video trak エントリ自体が出力されず、結果として `video_codec` が読めない状態になった。ただし `hisui_video_codec value="H264"` が立っているのでランタイム上は確定していた可能性が高く、書き出し側のタイミング問題が本命。
- **仮説 3: hisui inspect 側の挙動 (fMP4 / hybrid 出力との相性)**
  - hisui inspect は `Mp4FileDemuxer` 経由で MP4 を読む (`src/subcommand_inspect.rs:121` の `ContainerFormat::Mp4` 分岐 → `Mp4FileReader::new`)。hybrid_mp4_writer が生成する MP4 が、inspect 側の前提と微妙に違う形 (例: moov 直後の `mdat` が空のまま終わった、`moof` 経路に振った 等) になっていて、video trak が読めない可能性。
  - 関連: issues/0001 で fMP4 読み込み未対応の指摘あり (`src/mp4/reader.rs:1627` の `initialize_mp4_demuxer` コメント)。hybrid_mp4_writer の出力が想定外に fMP4 形式に近づいた状態で inspect が読んだなら、同じ症状を引きうる。
- **仮説 4: テストフローでの Stop タイミング**
  - `test_obsws_srt_inbound_start_record_and_inspect_output` 内で StopRecord → ファイル存在確認 → inspect の流れがあるが、StopRecord のレスポンスはあくまで API として受理した時点で返るのみで、mp4_writer が moov を書き終わって fsync まで完了したことを保証しない可能性。テスト側で finalize 完了を待つ仕組みが不足している可能性。

- **仮説 5 (モード2): SRT push 確立遅延に対して待機上限が短い**
  - モード2 はサンプル待機ループ (`test_output.py:937` 付近, 30 回 × 0.2 秒 = 約 6 秒) のタイムアウト。CI ランナー負荷や ffmpeg SRT push の接続確立 (SRT ハンドシェイク) が遅れると、6 秒以内に最初の映像サンプルが mp4_writer まで届かない可能性。失敗 run の `/metrics` が回収できれば `hisui_total_input_video_frame_count{processor_type="srt_inbound_endpoint"}` が 0 のままだったか (= そもそも SRT 入力が届いていない) を確認できる。

### 関連箇所 (調査着手用)

- `e2e-tests/obsws/test_output.py::test_obsws_srt_inbound_start_record_and_inspect_output` — テスト本体。
- `e2e-tests/obsws/helpers.py:452` — `video_codec` / `video_sample_count` の存在を必須キー検査するヘルパー (失敗箇所)。
- `src/mp4/hybrid_writer.rs` — hybrid_mp4_writer の本体。moov の recovery 更新ロジックと finalize の境界がここにある。
- `src/subcommand_inspect.rs` — inspect コマンドの実装。MP4 入力経路。
- `src/mp4/reader.rs:1627` 前後 — `initialize_mp4_demuxer` (fMP4 非対応の明示コメントあり)。
- `e2e-tests/obsws/test_output.py:937-951` — モード2 のサンプル待機ループ (待機上限 30 回 × 0.2 秒)。

## 設計方針

### A. 観測 + 切り分け (本対応の現フェーズ)

1 件の観測のみで原因が確定していないため、まずは **再現と切り分け** にコストを割く。実装変更は本フェーズの完了条件ではない。

#### 切り分け手順

1. CI ログ全体を保存し、本 issue にリンクとして添付する (`gh run download 26621510837` 等で可能ならアーティファクトも回収)。
2. 該当 MP4 (`obsws-record-1780035651665.mp4`) のローカル再現を試す:
   - `cargo run --release -- server` を起動し、E2E テストと同じ手順 (SRT inbound → StartRecord → SRT 入力流入 → StopRecord) をスクリプト化して 50 回繰り返し、再現可能か確認。
   - ローカルで再現できなければ workflow を `workflow_dispatch` で 20 回連続実行して CI 環境で再発を観測。
3. 失敗 MP4 を別経路で解析できる仕組みを E2E テスト側に追加する。
   - 当初は失敗時に MP4 を CI アーティファクトとして保全する案だったが、保全は「後から人手で解析する手番」と「retention 期限切れで取り逃すリスク」が残るため、より直接的な方式に変更した。
   - 実装 (2026-06-03): inspect の必須キー検査が失敗した時点で `ffprobe -v error -show_format -show_streams -of json <file>` を呼び、生 JSON を診断メッセージへそのまま含める (`e2e-tests/obsws/helpers.py` の `_probe_mp4_with_ffprobe` / `_inspect_mp4`)。これにより `moov.trak` に video stream があるかを CI ログ上で即判定でき、「ファイル自体に video trak が無い (writer 側)」のか「trak はあるが hisui inspect が読めない (inspect 側)」のかをモード1 で切り分けられる。metrics スナップショットは既に診断メッセージへ埋め込み済みのため追加対応は不要。
4. 再現できた失敗 MP4 を:
   - `hisui inspect <file>` で再実行 → 同じく `video_codec` / `video_sample_count` が欠ける状態か確認。
   - `MP4Box -info` / `ffprobe` 等の外部ツールで構造を解析し、`moov.trak` に video entry があるかを別経路で確認。これにより「mp4 ファイル自体に video trak が無い」のか、「video trak はあるが hisui inspect で読めない」のかを切り分ける。
5. 上記結果に基づき、仮説 1〜4 のどれが該当するかを本 issue の現状セクションに追記する。

### B. テスト側で finalize 完了を待つ (仮説 4 が当たった場合)

StopRecord のレスポンスは API として受理した時点を返すのみで、mp4_writer の最終 finalize 完了は別経路で観測する必要があるかもしれない。

- 対応案: StopRecord 後に `RecordingStateChanged` 相当のイベントを待つ、または `/metrics` の `hisui_total_video_sample_count` 等が安定値に到達するのを待つ簡易ポーリングを E2E テストヘルパーに入れる。
- これは本 issue の A 案で仮説 4 が当たった場合のみ実施する。実装着手は別 issue として切り出す。

### C. hybrid_mp4_writer の finalize を同期的にする (仮説 1 が当たった場合)

`recovery_moov_update_count = 63` から、recovery moov 更新が高頻度で走っていたことが分かる。StopRecord 受理時に「これ以降は recovery moov を出さず、最終 moov を完全に書き終えてから StopRecord のレスポンスを返す」フローへの変更を検討する。

- 対応案: hybrid_mp4_writer の close フローを再点検し、最終 moov 書き込み + fsync を完了してから StopRecord のレスポンスチャネルに完了通知を流す。
- 影響範囲は大きい (recovery moov の役割と整合させる必要)。実装着手は別 issue として切り出す。

### D. inspect 側の hybrid 出力対応を強化する (仮説 3 が当たった場合)

hybrid_mp4_writer が生成する MP4 構造が、`Mp4FileDemuxer` の前提と微妙にずれている可能性がある。`Mp4FileKindDetector` を使って入力ファイルの種別 (Mp4 / FragmentedMp4) を判定し、それぞれの読み出し経路を整備する。

- 対応案: issues/0001 (fMP4 読み込み対応) に統合するか、別途 hybrid_mp4_writer の出力形式を inspect 側で正しく読めるよう調整する。
- 実装着手は本 issue の切り分け結果次第。

### スコープ外 (本 issue では扱わない)

- E2E テスト共通のリトライ機構 (個別テストにリトライ付与する pytest プラグイン導入等)。仮説確定前にリトライで誤魔化すと根本原因がさらに分かりにくくなる。
- hybrid_mp4_writer 全体の設計見直し。本 issue は flaky の解消が目的で、設計刷新は別軸。

## 完了条件

本 issue (フェーズ A) の完了条件:

- 上記「切り分け手順」を実行し、仮説 1〜4 のうちどれが該当するかを本 issue の「結論」セクションに追記する。
- 失敗時の MP4 ファイルが回収できる仕組みを E2E テストに入れる。
- 仮説が確定し対応方針が決まった場合は、対応する設計方針 (B / C / D) を別 issue として起票する。本 issue はそこまで完了したら close する。
- 仮説確定後の再発防止対応は別 issue で扱うため、本 issue の close 時点でフレーキー自体は解消していなくてよい (経過観察に移行する)。

## エスカレーション基準

- 同事象の再発を観測した時点で「観測済み失敗事例」テーブルに追記する。
- 観測 3 件超過 (= 短期間に同種失敗が頻発): Medium に格上げし、切り分けを最優先で進める。
- 観測 5 件超過: High に格上げし、設計方針 B / C / D のいずれかを暫定でも適用する判断をする。
- 単発で連続 CI 失敗が起きた場合 (= 2 回連続失敗): その時点で Medium に格上げ。

## 経過観察

「観測済み失敗事例」テーブル参照。

2026-06-02 時点: 観測 2 件 (モード1 / モード2 の別症状)。連続 CI 失敗ではなく (両 run の間は成功)、観測も 3 件未満のため Priority は Low のまま据え置き。ただし「同一テストが異なる箇所で 2 度フレーキー」となったため、切り分けの優先度は上げ、次回観測時 (= 3 件目) で Medium 格上げを検討する。

本 issue の close 後も、同事象が観測されたら CLAUDE.md「issue が実は解決してなかった場合」手順に従って `issues/closed/` → `issues/` に `git mv` で戻し、観測事例を追記する。

## 解決方法

(切り分け完了時に追記)

### 結論 (切り分け完了時に追記)

仮説の判定結果と、対応として切り出した別 issue 番号を記録する。
