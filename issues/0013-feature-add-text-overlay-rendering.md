# 合成映像へのテキスト (字幕) 描画に対応する

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/add-text-overlay-rendering
- Polished:

## 目的

合成映像にテキストをオーバーレイ描画できる汎用基盤を提供する。想定される応用としては、ラベル・タイムスタンプ・任意テキストの表示や、別 issue 0012 (candle / Whisper 文字起こし) の結果を字幕 (transcription) として映像に重ねて表示することなどがある。本 issue は描画プリミティブの提供までを範囲とし、字幕特有の関心事 (縁取り・背景帯などの字幕スタイル一式、0012 結果との時刻同期) は本 issue では扱わない。

## 優先度根拠

- テキスト描画はラベル・タイムスタンプ・字幕など複数応用の前提となる汎用基盤で、単体で動作確認・マージが可能。
- ただし業務を止めている課題ではない。
- 以上から Medium。

## 現状

- hisui の映像合成は I420 (YUV) 上で行う:
  - `src/video/canvas.rs` の `I420Canvas` (`new` / `draw_frame_clipped` / `into_data`)。
  - `src/mixer/video.rs` の `VideoRealtimeMixer` と `compose_frame` / `draw_frame_clipped` / `blend_component` で、I420A レイヤをブレンドして I420 を出力する。
  - 録画合成側は `src/sora/recording_video_mixer.rs`。
  - 色空間変換・リサイズは shiguredo_libyuv を使用する。
- テキスト描画機能は無い。グリフをラスタライズして映像へ重ねる手段が存在しない。

## 設計方針

1. 描画ライブラリ
   - shiguredo/raden (https://github.com/shiguredo/raden) を採用する。Cranelift JIT ベースの CPU-only な 2D ベクターグラフィックスライブラリで、`fill_text(x, y, &Font, text)` でテキストを描画できる (全グリフを 1 つの Path に結合して fill_path で一括描画)。CPU のみで動くため、GPU の無い CI 環境とも相性が良い (hisui の合成も CPU ベース)。
   - リスク (要管理): raden は公式 README で「実験的プロジェクトであり、API や内部実装は予告なく大幅変更されうる」と明記されている。依存バージョンを厳密固定 (hisui 方針) し、API 変更時の追従コストを織り込む。
2. 描画結果の合成への取り込み
   - raden の描画出力は RGBA 系 (Rgba32)。hisui の合成は I420 / I420A。透明背景の RGBA バッファへテキストを描画 → shiguredo_libyuv で I420A (アルファ付き) へ変換 → 既存の `VideoRealtimeMixer` のレイヤ合成 (`compose_frame` / `blend_component`) に 1 レイヤとして渡す、という流れが既存構造と整合しやすい。
3. フォント
   - フォントファイルの同梱 or 指定方法を決める。日本語字幕を想定するなら CJK 対応フォントが必要 (ライセンスにも留意)。
4. 描画スタイル
   - 文字色・位置・サイズ・フォント指定の API 形を決める。
   - 縁取り・背景帯などの字幕特有スタイル、および 0012 結果との時刻同期は本 issue では扱わない (字幕としての応用は別途整理する)。
5. スコープ
   - リアルタイム (OBSWS) と録画合成の両経路で使えるようにするか、まず録画合成に限定するかを決める。

## 完了条件

- 合成映像の指定位置に、指定したスタイル (文字色・サイズ・フォント) でテキストを描画できること。
- 0012 等の他 issue に依存せず、本 issue 単独で動作確認・マージできること。
- CHANGES.md の `## develop` に該当エントリを追記すること。

## 解決方法

- raden で RGBA へ描画 → I420A へ変換 → 既存レイヤ合成へ組み込む形で実装する。
- 詳細スコープ (リアルタイム対応の要否、フォント同梱方針) は `/polish-issue` での磨き上げ時に確定する。
