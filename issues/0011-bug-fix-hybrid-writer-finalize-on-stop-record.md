# 短時間録画で hybrid_mp4_writer の finalize が走らず映像トラックが空になる

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/fix-hybrid-writer-finalize-on-stop-record
- Polished:

## 目的

StopRecord 直後に inspect すると映像トラックが読めない事象 (issues/0008) の根本原因を修正する。issues/0008 のフェーズ A（観測・切り分け）で原因が「hybrid_mp4_writer の finalize と StopRecord/EOS 処理の競合」と確定したため、その修正を本 issue で扱う。

flaky なテストの解消にとどまらず、短時間録画では本番運用でも映像トラックが空の MP4 が生成されうる実害があるため対応する。

## 優先度根拠

Medium とする。

- 短時間 (約 2 秒未満) の録画では、StopRecord 後の MP4 から映像トラックが読めない（サンプル 0 の空トラックになる）実害がある。テスト固有の問題ではなく本番でも再現しうる。
- 一方で、2 秒を超える通常の録画では最低 1 フラグメントが flush 済みになり救われるため、影響範囲は短時間録画に限定される。常時発生する致命的バグではない。
- High にしない理由: 影響が短時間録画に限定され、通常の録画運用は救われている。
- Low にしない理由: 本番でも再現する実データ欠損であり、テストのフレーキー解消（issues/0008）の域を超えた実害があるため。

## 現状

issues/0008 の「## 解決方法 / 結論」に切り分け結果の詳細がある。要点のみ再掲する。

### 再現

issues/0008 で対象テストを CI で 100 回繰り返し、約 3% でモード1（inspect が `video_codec` / `video_sample_count` を読めない）を再現した。失敗時の最終 MP4 を ffprobe で解析すると、video trak は存在する（`codec_name: h264`, `avc1`, 1920x1080）が、サンプルがゼロ（`duration: 0`, `nb_frames` 無し）。`nb_streams: 1` で音声トラックすら無い。

### メカニズム

1. 録画が約 0.8 秒と短く、`HYBRID_FRAGMENT_MAX_DURATION = 2 秒`（`src/mp4/hybrid_writer.rs:38`）の時間フラッシュに届かない。録画開始時の 1 枚以外にキーフレームが無く GOP 区切りフラッシュも起きない（`src/mp4/hybrid_writer.rs:650`）。よって全サンプルが未フラッシュの単一フラグメントに滞留する（`total_flushed_fragment_count = 0`、`unflushed_video_sample_count = 23`）。
2. finalize は入力キューが空かつ pending 無しのときだけ走る（`src/mp4/hybrid_writer.rs:597-606`）。最後の 1 フレームは常に `pending_video_frame` として次フレーム待ちで保持される設計（`src/mp4/hybrid_writer.rs:637-657`）。
3. StopRecord の staged stop は writer に Finish RPC を送り、入力トラックを即座に閉じる（`Finish` ハンドラが `input_video_track_id = None`、`src/mp4/hybrid_writer.rs:905-911`）。終端 EOS / pending を読み切る前に入力を閉じる競合は `src/obsws/coordinator/output_record.rs:316-324` の NOTE に既知として明記されている。pending が宙に浮くと finalize 条件に到達せず、`wait_or_terminate` の 5 秒タイムアウトで強制終了 → finalize 未実行のまま、録画中の空 stbl recovery moov だけがファイルに残る。

決定的な裏付け: StopRecord 応答後のメトリクスでも `hisui_actual_moov_box_size = 0`（finalize は `src/mp4/hybrid_writer.rs:534` でこの値を設定する）。つまり StopRecord 応答時点で finalize が一度も走っていない。

### 関連箇所

- `src/mp4/hybrid_writer.rs:527` — `finalize()`。flush_fragment → mp4_muxer.finalize → 標準 moov 書き出し。
- `src/mp4/hybrid_writer.rs:590-626` — `handle_next_audio_and_video()`。finalize の発火条件（pending 残存時は finalize しない）。
- `src/mp4/hybrid_writer.rs:888-914` — `handle_rpc_message()`。Finish RPC で入力トラックを即座に閉じている箇所。
- `src/obsws/coordinator/output_record.rs:310-388` — `stop_processors_staged_record()` / `finish_mp4_writer_rpc()`。既知レースの NOTE。

## 設計方針

finalize が「pending を含む全サンプルを書き切ってから」確実に完了することを保証する。検討する方向（実装時に精査する）:

- Finish RPC を受けたら、入力を閉じる前に終端処理（pending フレームの flush + 残フラグメントの flush）を行ってから finalize する経路にする。Finish の応答は finalize 完了後に返す。
- もしくは EOS を正しく読み切ってから finalize に進むよう、staged stop 側の順序（encoder terminate → EOS 伝播待ち → Finish）を見直す。`output_record.rs:316-329` の既存 NOTE（末尾欠損レース）と整合させること。
- recovery moov の役割（異常終了時の回復）を壊さないこと。正常な StopRecord では必ず標準 moov へ finalize されることを保証する。

回帰検知用のメトリクス追加も本 issue のスコープに含める:

- finalize 完了を示すカウンタ（例 `hisui_total_finalize_count`）。強制終了で finalize がスキップされたことを観測可能にする。
- writer が `wait_or_terminate` のタイムアウト経路（強制終了）を通ったかどうかのフラグまたはログ。

## 完了条件

- 短時間（2 秒未満）録画でも StopRecord 後の MP4 に映像サンプルが正しく含まれること。
- issues/0008 の対象テスト `obsws/test_output.py::test_obsws_srt_inbound_start_record_and_inspect_output` を CI で多数回（100 回程度）繰り返してもモード1 が再発しないこと。
- finalize 完了を確認できるメトリクスが追加されていること。
- PBT / 単体テストで、pending を含む全サンプルが finalize 後の moov に反映されることを検証すること。

## 解決方法

（実装時に追記）
