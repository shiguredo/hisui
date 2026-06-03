# 短時間録画で hybrid_mp4_writer の finalize が走らず映像トラックが空になる

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/fix-hybrid-writer-finalize-on-stop-record
- Polished: 2026-06-03

## 目的

StopRecord 直後に inspect すると映像トラックが読めない事象 (issues/0008) の根本原因を特定して修正する。issues/0008 のフェーズ A（観測・切り分け）で「短時間録画では StopRecord 後の MP4 が空の映像トラックになる」という実測事実が確定したため、その修正を本 issue で扱う。

flaky なテストの解消にとどまらず、短時間録画では本番運用でも映像トラックが空の MP4 が生成されうる実害があるため対応する。

なお、issues/0008 の結論セクションには本 issue で誤りと判明した機序の記述（「pending が宙に浮いて finalize 条件に到達しない」）が残っている。本 issue で真因を確定したら issues/0008 の該当記述も訂正する（issues/0008 は closed のため、訂正可否はユーザー判断とする）。

## 優先度根拠

Medium とする。

- 短時間 (約 2 秒未満) の録画では、StopRecord 後の MP4 から映像トラックが読めない（サンプル 0 の空トラックになる）実害がある。テスト固有の問題ではなく本番でも再現しうる。
- 一方で、2 秒を超える通常の録画では最低 1 フラグメントが flush 済みになり救われるため、影響範囲は短時間録画に限定される。常時発生する致命的バグではない。
- High にしない理由: 影響が短時間録画に限定され、通常の録画運用は救われている。
- Low にしない理由: 本番でも再現する実データ欠損であり、テストのフレーキー解消（issues/0008）の域を超えた実害があるため。

## 現状

### 確定している実測事実 (issues/0008 フェーズ A)

issues/0008 で対象テストを CI で 100 回繰り返し、約 3% でモード1（inspect が `video_codec` / `video_sample_count` を読めない）を再現した。詳細な ffprobe 出力とメトリクスダンプは issues/0008 の「## 解決方法 / 結論」を参照。要点は次の 3 点。

- 生成された MP4 には映像トラックの SampleEntry（`avc1`）は存在するが、サンプルがゼロ（`duration: 0`, `nb_frames` 無し）。外部ツール ffprobe でもサンプルが読めないため、inspect 側のバグではなくファイル自体が壊れている。
- StopRecord 応答後のメトリクスでも `hisui_actual_moov_box_size = 0`。この値は HybridMp4Writer では `finalize()` 内（`src/mp4/hybrid_writer.rs:534`）でのみ設定される（通常の Mp4Writer 側は `src/mp4/writer.rs:590` で設定するが本 issue の対象外）。よって **StopRecord 応答時点で `finalize()` が一度も完了していない**ことを示す。
- `hisui_total_flushed_fragment_count = 0`、`hisui_total_video_sample_count = 24`（StopRecord 前は 23、後は 24）。全サンプルが未 flush のフラグメントに滞留したまま、ディスク上のサンプルテーブルにも標準 moov にも反映されていない。録画中の recovery 用 fMP4 moov（stbl 空）だけがファイルに残った状態。

### なぜ短時間録画で顕在化するか

- 録画が約 0.8 秒と短く、`HYBRID_FRAGMENT_MAX_DURATION = 2 秒`（`src/mp4/hybrid_writer.rs:38`）の時間フラッシュに届かない。録画開始時の 1 枚以外にキーフレームが無く GOP 区切りフラッシュも起きない（`src/mp4/hybrid_writer.rs:650`）。よって録画全体が単一の未 flush フラグメントに収まり、出力の成否が `finalize()` の実行有無だけに依存する。
- 2 秒超の録画や複数キーフレームを含む録画では、最低 1 フラグメントが flush 済みになるため、仮に `finalize()` がスキップされても recovery moov + flush 済みフラグメントから映像が読める。短時間録画はこの救済が無く、`finalize()` 未実行が即「空トラック」に直結する。

### 真因（確定: 候補 B = finalize 内 Err）

2026-06-03 の観測（追加した観測点入りで CI 100 回実行、`E2E Flaky Repro`）で真因を確定した。失敗時のメトリクスは `hisui_total_finalize_started_count = 1` / `hisui_total_finalize_completed_count = 0`（finalize に到達したが完了せず）で、サーバ stderr に次の warn が出ていた。

```
[WARN] hisui::mp4::hybrid_writer - hybrid mp4 writer exited with error: Missing sample entry for first sample of Audio track
```

よって **候補 B（`finalize()` 内の `flush_fragment()` で Err が発生し `run()` が Err 終了）が真因**。候補 A・C は棄却。

具体的な発生機序:

- `append_audio_to_fragment()`（`src/mp4/hybrid_writer.rs:253-266`）は、サンプルの `sample_entry` を `sample.sample_entry.clone().or_else(|| self.last_audio_sample_entry.clone())` で決める。
- 音声トラックの**最初のサンプルが `sample_entry` 無しで到着**し、かつ過去に `sample_entry` 付きサンプルが無い（`last_audio_sample_entry` も None）場合、そのサンプルの `sample_entry` は None になる。
- flush 時（`flush_fragment()` → muxer）に、トラックの先頭サンプルには `sample_entry`（コーデック設定）が必須のため、muxer が「Missing sample entry for first sample of Audio track」で Err を返す。
- 短時間録画では flush 機会が finalize 時の 1 回だけなので、この先頭サンプルの sample_entry 欠落が即 finalize 失敗 → 空 moov に直結する。約 3% の発生率は、音声トラックの先頭サンプルが sample_entry 無しで到着する競合の頻度と整合する。

下記は確定前の検討記録（経緯として残す）。`flush_pending_video_if_ready` 等により Finish/EOS 経路では finalize に到達することが分かり、候補 A は弱いと判断していた。

#### 確定前の検討（参考）

issues/0008 の結論は「Finish RPC が入力を即座に閉じると pending フレームが宙に浮き、finalize 条件に到達しないまま強制終了される」と記述しているが、**この機序はコード読解と整合しない**。

- `Finish` ハンドラは `input_video_track_id = None` / `input_audio_track_id = None` を設定して `Ok(true)` を返す（`src/mp4/hybrid_writer.rs:905-911`）。`run()` はこれを受けて受信チャネルを閉じ、直後に `poll_output()` を呼ぶ（`src/mp4/hybrid_writer.rs:872-880`）。
- `poll_output()` の待機分岐はいずれも `input_*_track_id.is_some()` を要求するため、両トラック None なら待機せず `handle_next_audio_and_video()` に進む（`src/mp4/hybrid_writer.rs:749-768`）。
- その先頭で `flush_pending_video_if_ready()` / `flush_pending_audio_if_ready()` が走り、「track_id が None かつキュー空かつ pending あり」の条件で pending をフラグメントへ flush する（`src/mp4/hybrid_writer.rs:692-728`）。pending が掃けると `(None, None)` かつ pending 無しに到達し `finalize()` が呼ばれる（`src/mp4/hybrid_writer.rs:597-604`）。

つまり Finish RPC が writer に届いて処理される限り、pending は救済されて `finalize()` に到達するのがコードの素直な読みである。さらに、Finish RPC が届かなくても finalize には到達しうる。staged stop は encoder を terminate して writer の購読チャネル送信端を drop するため、writer は `Message::Eos` を受信し、その Eos 処理で `input_*_track_id = None` が設定される（`src/mp4/hybrid_writer.rs:931-964`、`handle_input_sample(..., None)` 経由）。track_id が None になれば pending 救済を経て `finalize()` に到達する。

したがって正常な停止経路では finalize に到達するはずであり、`actual_moov_box_size = 0`（finalize 未完了）を説明するには「finalize に到達したが完了しなかった」または「到達前に writer が終了した」経路が必要になる。本 issue の最初の作業は、この真因の特定である。現時点の候補は次の 3 つ。いずれも未検証で、複合の可能性もある。

- **候補 A: writer が `finalize()` 到達前に強制終了される。** 経路は 2 つ。(1) `finish_mp4_writer_rpc()` が `get_rpc_sender()` に失敗し続けて 500ms 後に `terminate_processor()` する（`src/obsws/coordinator/output_record.rs:377-385`）。(2) `wait_or_terminate(writer, 5 秒)` がタイムアウトし、生存していれば `terminate_and_wait` へ委譲して強制終了する（`src/obsws/coordinator/output_record.rs:344-349`、`src/obsws/coordinator/output.rs:670-686`）。ただし EOS だけでも finalize に到達するため、(1) の RPC 未達単独では症状を説明できない（EOS 経路が生きていれば finalize する）。(2) も「finalize に到達しないまま 5 秒経つ」状況を要し、上述の素直な読みと矛盾する。よって候補 A は単独では弱く、RPC 未達と EOS 未達が同時に起きる等の追加条件が要る。
- **候補 B: `finalize()` に到達したが内部処理が Err を返し、`run()` が Err 終了する。** `finalize()` は先頭で `flush_fragment()?`（`src/mp4/hybrid_writer.rs:529`）、続いて `mp4_muxer.finalize()?`（532）を実行し、**534 で初めて** `actual_moov_box_size` を設定する。529 または 532 が I/O エラー等で Err を返すと 534 に到達せず、`run()` が `?` 伝播で Err 終了する（`src/mp4/hybrid_writer.rs:1017-1020` のクロージャが Err を返す）。この場合は強制終了ではなく writer の自己終了であり、観測上は `actual_moov_box_size = 0` かつ録画中の recovery moov（stbl 空）だけが残る——実測症状と完全に一致する。なお既存の `hisui_error` gauge ではこの経路を検出できない（`set_error()` は `src/mp4/writer.rs:480` の「BUG: unexpected input stream」分岐でのみ呼ばれ、finalize の Err では立たない）。検出には観測点の追加が必要（後述）。
- **候補 C: `finalize()` 完了直前のタイムアウト。** 候補 A(2) と同じ `wait_or_terminate` だが、finalize に到達しているが処理中に 5 秒を超えるケース。`flush_fragment` は seek を伴う複数 `write_all` と recovery moov 再書き込み（free 領域への書き込み、`src/mp4/hybrid_writer.rs:508-520`）を含むが、短時間録画では I/O 量が小さく 5 秒を超える根拠は乏しい。優先度は最も低いが、観測で除外するまで残す。

A(2) と C はいずれも `wait_or_terminate` のタイムアウト強制終了だが、「finalize 未到達（A(2)）」か「finalize 処理中（C）」かで区別する。後述の観測点（finalize 開始・完了・Err の区別）と強制終了経路通過の組み合わせで弁別できる。

### 関連箇所

- `src/mp4/hybrid_writer.rs:527-588` — `finalize()`。flush_fragment → mp4_muxer.finalize → 標準 moov 書き出し。`actual_moov_box_size` を設定する唯一の箇所（534）。
- `src/mp4/hybrid_writer.rs:590-626` — `handle_next_audio_and_video()`。finalize の発火条件（pending 残存時は finalize しない）。
- `src/mp4/hybrid_writer.rs:692-728` — `flush_pending_audio_if_ready()` / `flush_pending_video_if_ready()`。track クローズ後に pending を救済する中核。issues/0008 の機序記述が見落としていた箇所。
- `src/mp4/hybrid_writer.rs:872-886` — `run()` の Finish 処理後に `poll_output()` を呼ぶ箇所。
- `src/mp4/hybrid_writer.rs:888-914` — `handle_rpc_message()`。Finish RPC で入力トラックを閉じ、finalize 前に即応答する箇所。
- `src/mp4/hybrid_writer.rs:931-964` — `handle_audio_message()` / `handle_video_message()`。Eos 受信で `input_*_track_id = None` を設定する箇所（既存の `add_received_*_eos` メトリクスもここ）。
- `src/obsws/coordinator/output_record.rs:310-388` — `stop_processors_staged_record()` / `finish_mp4_writer_rpc()`。強制終了経路（377-385）と既知レースの NOTE（315-329）。
- `src/obsws/coordinator/output.rs:670-686` — `wait_or_terminate()`。タイムアウトで強制終了する経路。戻り値は強制終了・自然終了のどちらでも `Ok(())` を返すため、呼び出し側の戻り値だけでは経路を区別できない点に注意。
- `src/mp4/writer.rs` — `Mp4WriterStats`。メトリクス追加のパターン（フィールド・コンストラクタ・`add_*`・getter・`stats.counter()` 登録）と `hisui_error` フラグ（error gauge）。

## 設計方針

### 第 1 段階: 真因の特定

修正方針を確定する前に、finalize が完了しない経路（候補 A / B / C / それ以外・複合）を観測で確定する。観測は次の 4 軸を弁別できることを要件とする。

- **finalize の開始・完了・Err の区別**: `finalize()` の開始時・完了時・Err 終了時にそれぞれ観測点を置く。完了カウンタを追加する（命名は実装時に既存メトリクス命名規則へ合わせる。例: 登録名 `total_finalize_count` → 観測名 `hisui_total_finalize_count`、counter 型。`src/mp4/writer.rs` の `Mp4WriterStats` にフィールド・コンストラクタ・`add_*`・getter を追加し、`finalize()` 末尾 `src/mp4/hybrid_writer.rs:585` 付近で増やす）。開始の観測（開始カウンタまたは tracing ログ）も入れ、「finalize に到達したが完了しなかった（候補 B/C）」と「finalize に到達しなかった（候補 A）」を区別する。
- **finalize の Err 捕捉（候補 B）**: 既存の `hisui_error` gauge は finalize の Err では立たない（`set_error()` は `src/mp4/writer.rs:480` の「BUG: unexpected input stream」分岐でのみ呼ばれる）。候補 B を検出するには、`run()` を起動するクロージャ（`src/mp4/hybrid_writer.rs:1017-1020`）で `run()` の Err を捕捉して warn ログまたは専用カウンタに記録する観測点を追加する。
- **EOS 受信の観測**: 既存の `add_received_*_eos`（`src/mp4/hybrid_writer.rs:932,959`）の値を再現時に確認し、writer が EOS を受信したか（= EOS 経路で finalize に到達できる状態だったか）を切り分ける。新規追加は不要で、メトリクスダンプに含める運用で足りる。
- **強制終了経路の観測（候補 A）**: `wait_or_terminate()`（`src/obsws/coordinator/output.rs:670-686`）自身は terminate せず、タイムアウト後に生存していれば `terminate_and_wait`（同 685）へ委譲する。よって戻り値では経路を区別できない。実際に強制終了する `terminate_and_wait` 呼び出し直前（685）と、`finish_mp4_writer_rpc()` の `terminate_processor` 経路（`src/obsws/coordinator/output_record.rs:377-385`）に直接 warn ログを仕込む。

以上の 4 軸を入れた上で、後述の一時ワークフローで再現させ、候補 A / B / C のいずれか（または複合）を確定する。第 1 段階の観測で候補のいずれにも当てはまらない場合は、finalize 直前までの tracing ログを追加して再観測する（観測が空振りした際の出口）。

### 第 2 段階: 修正

確定した真因に応じて修正する。真因確定前に方針を断定しない。修正の方向（候補別）:

- 候補 A: 強制終了経路でも finalize を実行する手段（例: terminate 前に同期 finalize を促す）を検討する。
- 候補 B: Err の発生箇所（`flush_fragment` か `mp4_muxer.finalize` か）と原因（I/O・状態不整合）を特定し根本を直す。finalize 失敗時もファイルが可能な限り回復可能になるようにする。
- 候補 C: Finish の応答を `finalize()` 完了後に返すか、`wait_or_terminate` のタイムアウトを finalize 完了まで延長する。

いずれの場合も次を満たすこと:

- StopRecord の応答性を優先する既存方針（`src/obsws/coordinator/output_record.rs:324` の NOTE）とのトレードオフを明示し、応答が finalize 完了まで待つ設計に変えるなら遅延が許容範囲かを評価する。
- recovery moov の役割（異常終了時の回復）を壊さない。正常な StopRecord では必ず標準 moov へ finalize されることを保証する。生成 MP4 のレイアウトに後方互換上の変化が生じないことを確認する。
- 第 1 段階の観測で真因が確定し、第 2 段階の修正が大規模（設計変更を伴う）になると判明した場合は、第 2 段階を別 issue に切り出す。第 1 段階（観測点の追加と真因の確定）だけで本 issue を一旦区切ることを許容する。

## 完了条件

第 1 段階（真因特定）の完了条件:

- 候補 A / B / C のいずれか（または複合）を観測で特定し、本 issue の「現状」へ確定内容を反映する。
- finalize の開始・完了・Err と強制終了経路を弁別できる観測点（メトリクス / ログ）が追加されていること。このうち恒久的に残すもの（`total_finalize_count` 等の正規メトリクス）と、調査用に一時追加して撤去するもの（finalize 直前の詳細 tracing ログ等）を区別して扱う。

第 2 段階（修正）の完了条件（第 2 段階を別 issue に切り出す場合は、その issue の完了条件とする）:

- 短時間（2 秒未満）録画でも StopRecord 後の MP4 に映像サンプルが正しく含まれること。
- issues/0008 の対象テスト `obsws/test_output.py::test_obsws_srt_inbound_start_record_and_inspect_output` を CI で繰り返してもモード1 が再発しないこと。再現率が約 3% のため、検証は十分な試行回数で行う。検証には issues/0008 で用いた 10 シャード × 10 回 = 100 回相当のワークフローを一時的に再追加し、検証後に削除する。この一時ワークフローの追加・削除は最終差分に残さない（同一ブランチ内なら追加と削除を別コミットにし、マージ時に最終差分から消えることを確認する）。
- テストを役割分担に沿って追加する（CLAUDE.md の方針）:
  - 単体テスト: writer 単体で「track クローズ → pending 救済 → finalize」の同期経路を検証する（回帰防止用であり、非同期の staged stop 競合そのものの再現ではない点に注意）。現状 hybrid_writer の既存テスト（`src/mp4/hybrid_writer.rs` の `#[cfg(test)] mod tests`）は内部状態しか見ておらず、finalize 後の MP4 を再デコードして検証するテストが無い。書き出したファイルを `Mp4FileDemuxer::next_sample()`（`shiguredo_mp4` 由来。`src/mp4/reader.rs` で利用、inspect も同経路で計数）でトラックごとにサンプル数を数えて検証する単体テストを新設する。
  - PBT: 任意のサンプル列を入力して finalize 後の moov に全サンプルが反映されること（ラウンドトリップ）を検証できる範囲があれば追加する。staged stop の非同期競合自体は PBT の対象外。
- `CHANGES.md` の `## develop` に `[FIX]` エントリを追記する（CLAUDE.md の変更履歴規約）。

## 解決方法

（実装時に追記）
