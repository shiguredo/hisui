# 合成映像へのテキスト (字幕) 描画に対応する

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/add-text-overlay-rendering
- Polished:

## 目的

合成映像にテキストをオーバーレイ描画できるようにする。主用途は、別 issue 0012 (candle / Whisper 文字起こし) の結果を字幕 (transcription) として映像に重ねて表示すること。将来的にはラベルやタイムスタンプ等の表示にも使える。

## 優先度根拠

- 文字起こし字幕という具体的なユースケースがあり、0012 と組み合わせて価値が出る。
- ただし 0012 への依存が大きく、単体では効果が限定的。
- 業務を止めている課題ではないため Medium。

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
4. 字幕としての制御
   - 文字色・縁取り・背景帯・位置・サイズの指定方法。
   - 0012 の文字起こし結果には時刻情報が付くため、字幕の表示タイミングを合成のタイムスタンプと同期させる方法。
5. スコープ
   - リアルタイム (OBSWS) と録画合成の両経路で使えるようにするか、まず録画合成に限定するかを決める。

## 完了条件

- 合成映像の指定位置に、指定したテキストを描画できること。
- 0012 の文字起こし結果を字幕として重畳できること (0012 完了後)。
- CHANGES.md の `## develop` に該当エントリを追記すること。

## 解決方法

- raden で RGBA へ描画 → I420A へ変換 → 既存レイヤ合成へ組み込む形で実装する。
- 詳細スコープ (リアルタイム対応の要否、フォント同梱方針、字幕タイミング同期) は `/polish-issue` での磨き上げ時に確定する。
