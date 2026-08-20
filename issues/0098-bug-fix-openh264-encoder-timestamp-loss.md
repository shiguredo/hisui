# OpenH264 エンコーダーがバッファリング時にタイムスタンプを失い、末尾フレームが欠落する

- Created: 2026-08-20
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-openh264-encoder-timestamp-loss
- Polished: {YYYY-MM-DD}

## 目的

`src/encoder/openh264.rs` の `Openh264Encoder` が、エンコーダーの内部バッファリングにより `None` を返した入力フレームのタイムスタンプを失い、遅れて出た出力に誤ったタイムスタンプが付与される問題と、`finish()` で末尾フレームが排出されない問題を修正する。

## 現状

`Openh264Encoder::encode` は `shiguredo_openh264::Encoder::encode` の戻りが `None` のとき `Ok(())` で戻り、その入力フレームのメタデータをどこにも残さない。出力が出たときは、その呼び出しの `frame.as_video_frame().timestamp` を付ける。

- バッファリングが起きると、遅れて出たビットストリームに「次の入力の」タイムスタンプが付与され、1 フレーム分ずれる
- 最後の入力で `None` が返った場合、そのフレームのエンコード結果は `finish()` で排出されない。`finish()` はコメントどおり「他のエンコーダーに合わせてメソッドだけ用意」しており、本体は `Ok(())` を返すだけである
- `src/encoder/libvpx.rs` の `LibvpxEncoder` と `src/encoder/svt_av1.rs` の `SvtAv1Encoder` は `input_queue` に入力を積み、出力時に先頭からタイムスタンプを取り、`inner.finish()` で残りを出す
- `src/decoder/openh264.rs` の `Openh264Decoder` も同じ FIFO と `inner.finish()` を持つ。エンコーダーだけが未対応である
- 既存テスト `openh264_sets_sample_entry_on_every_output_frame` と `openh264_sets_sample_entry_after_keyframe_request` は `sample_entry` の有無と最新値の伝播だけを見る。タイムスタンプ列は検証していない
- `shiguredo_openh264` 2026.1.0 の `Encoder::encode` は `Option<EncodedFrame>` を返す。`finish` は `Decoder` にだけあり、`Encoder` には無い。末尾排出は crate 側 API の有無を実装時に確定する必要がある

## 設計方針

- 入力フレームを `input_queue` に積み、出力が出たときに先頭の入力からタイムスタンプを取り出す方式 (`LibvpxEncoder` / `SvtAv1Encoder` と同様) に変更する
- `inner.encode` が `None` を返しても入力はキューに残す
- `last_sample_entry` の更新と、SPS / PPS 未確定時の fail-fast は維持する
- `finish()` で内部バッファに残ったフレームを排出する。crate の `Encoder` に flush 相当が無い場合は、本 issue で可能な範囲（メタデータの対応付け）と crate 側の不足を分けて完了条件に書く
- 変更対象は `src/encoder/openh264.rs`。上位の `VideoEncoder` ディスパッチは触らない

## 完了条件

- バッファリングが起きても出力フレームのタイムスタンプが入力と正しく対応する
- 内部バッファに残ったフレームを EOS / `finish()` 後に出せる。crate に Encoder flush が無ければ、その制約を issue か `CHANGES.md` に明記し、メタデータ対応は完了とする
- 既存の `sample_entry` テストが通る
- OpenH264 エンコーダーのテストで、フレーム数とタイムスタンプ列が検証されている（`OPENH264_PATH` 未設定の環境ではスキップする）
