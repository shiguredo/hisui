# fMP4 ファイルの読み込みに対応する

- Priority: Medium
- Created: 2026-05-29
- Completed:
- Model: Opus 4.7
- Branch: feature/add-fmp4-read-support
- Polished: 2026-06-03

## 目的

現状の hisui は `.mp4` 拡張子のファイルを通常の MP4 (`ftyp` + `moov` + `mdat` でサンプルテーブルが `stbl` に揃ったもの) としてのみ扱っており、fragmented MP4 (`moov` に `mvex` を含み、`moof`/`mdat` セグメントを並べた構造) を読み込めない。HLS や MPEG-DASH、Sora 以外の録画ツールが生成するメディアファイルでは fMP4 形式が広く使われており、現状ではそうしたファイルの中身を hisui で確認・取り込みできない。本対応では、まず fMP4 ファイルを読み込んでサンプル単位の情報を取り出せるようにする。

## 優先度根拠

- 現状でも MP4 (非 fragmented) と WebM の読み込みは可能なため、機能不足ではあるが業務が完全に止まる類のものではない。
- 一方、外部ツールが生成する .mp4 を inspect しようとしたときに「読めない」で詰まるケースが今後増える見込みであり、最低限の中身確認ができる状態を作っておきたい。
- 依存ライブラリ `shiguredo_mp4` 側に `Fmp4FileDemuxer` と `Mp4FileKindDetector` がすでに用意されているため、実装コストは比較的小さい。
- 以上から、High ではないが Low でもなく Medium が妥当。

## 現状

### 関連コード

- `src/types.rs:84-103` (`ContainerFormat::from_path`)
  - 拡張子 `.mp4` / `.webm` の二択。`mp4` か `fmp4` かはここでは区別しない。
- `src/mp4/reader.rs` (約 2380 行)
  - `Mp4FileReader` は `seek()` / `prev_sample()` / warm-up / 一時停止 / ループを伴う OBSWS メディア再生機能に依存した大きな構造体。内部で `ReaderState` が `Mp4FileDemuxer` を直接生成して使う。
  - 前方読みのためのユーティリティ (`ReaderState::update_audio_format` / `update_video_format` / `read_sample_data`、自由関数 `calculate_timestamps` / `initialize_mp4_demuxer` / `select_audio_track` / `select_video_track`) もこのファイルに同居している。
  - `probe_mp4_track_availability` / `probe_mp4_video_dimensions` は前方読み (`next_sample`) のみで成立する。
- `src/subcommand_inspect.rs`
  - `Mp4FileReader::new()` 経由で `Mp4FileDemuxer` を使う構成。
  - 重要: inspect は reader に decoder を設定していない (`set_audio_decoder` を呼ばない)。デコードは別 processor の `audio_decoder` / `video_decoder` が担当し、reader の責務は encoded sample を `*_encoded` トラックへ publish するだけ。よって inspect が `Mp4FileReader` から実際に使うのは前方読み (`next_sample`) のみで、warm-up / realtime / seek / loop / pause は一切使わない。
- `src/sora/recording_mp4_reader.rs` / `src/sora/recording_reader.rs`
  - Sora の録画合成パイプライン用。`Mp4VideoReader` / `Mp4AudioReader` は `Iterator` 実装で `next_sample()` のみ (前方読み、seek 不使用)。
  - `recording_reader.rs` 側は `ContainerFormat` で Mp4/Webm を分岐し、実体 reader を `enum {Mp4, Webm}` で保持する。実体 reader 内部の demuxer を差し替えれば `recording_reader.rs` 自体は無変更で済む構造。
- `src/obsws/source/file_mp4.rs` / `src/obsws/state.rs` / `src/obsws/source/mp4.rs`
  - OBSWS のメディア入力。`Mp4FileSource::create_reader` が `Mp4FileReader::new()` を `realtime: true` で生成し、再生制御 (seek / pause / loop) を使う。トラック有無・解像度判定に `probe_mp4_track_availability` / `probe_mp4_video_dimensions` を利用。

### `shiguredo_mp4` ライブラリ (2026.3.0) で利用可能な API

実コード (`Cargo.toml` は `=2026.3.0` 固定) で確認済み。

- `demux::Mp4FileDemuxer`: 通常 MP4 用。`new() / required_input() / handle_input() / tracks() / next_sample() / prev_sample() / seek()` を持つ。`Clone` 実装あり。
- `demux::Fmp4FileDemuxer`: fMP4 用。`new() / required_input() / handle_input() / tracks() / next_sample()` のみ。`seek()` と `prev_sample()` は提供されていない。`Clone` 実装あり。
- `demux::Mp4FileKindDetector`: ファイル先頭 (`ftyp` + `moov` まで) を incremental に読んで判定する。`required_input() / handle_input() / file_kind() -> Result<Option<Mp4FileKind>, DemuxError>`。
- `demux::Mp4FileKind`: `Mp4` / `FragmentedMp4` の 2 値。
- `demux::{Sample, TrackInfo, Input, RequiredInput, DemuxError}`: 両 demuxer で共通。`Fmp4FileDemuxer` の `Sample::data_offset` もファイル絶対位置に揃えられている (`moof_offset + data_offset` 加算済み)。

### 制約

- `Fmp4FileDemuxer` には `seek()` / `prev_sample()` が無いため、`Mp4FileReader` の OBSWS 連携で使っている warm-up 付きシーク・後方走査をそのまま fMP4 に拡張することはできない。
- `Fmp4FileDemuxer` のドキュメントに、`tfhd` の `base_data_offset` にファイル先頭からの絶対オフセットが記録されている形式には対応していないと明記されている。

## 設計方針

fMP4 を読みたい箇所は実際には 4 系統あり、前方読みで足りるか seek を要するかで難易度が大きく異なる。これを段階の境界にする。

| 箇所 | 使う reader | seek 要否 | 段階 |
| --- | --- | --- | --- |
| inspect | `Mp4FileReader` (前方読みのみ使用) | 不要 | 段階 1 |
| 録画合成 | `Mp4VideoReader` / `Mp4AudioReader` (Iterator) | 不要 | 段階 2a |
| OBSWS probe | `probe_mp4_*` | 不要 | 段階 2b に付随 |
| OBSWS 再生 | `Mp4FileReader` (seek/warm-up 使用) | 必要 | 段階 2b |

### 段階 1: inspect コマンドの fMP4 対応 (本 issue の範囲)

inspect は前方読みしか使わないため、`Mp4FileReader` (再生制御込みの大きな構造体) から inspect を切り離し、前方読み専用の軽量 reader に寄せる。demuxer の Mp4/Fmp4 差は薄い enum で吸収し、reader 本体は 1 本にする。これにより前方読みパスの二重化を避け、`Mp4FileReader` を OBSWS 専用に純化できる。なお、composition_time_offset (B フレーム由来の CTS オフセット) を持つサンプルは、既存の前方読みパスと同様に段階 1 でも非対応とし、エラーで弾く挙動を踏襲する (B フレーム対応は将来の別 issue)。

1. ファイル種別判定ヘルパー `detect_mp4_file_kind(path) -> Result<Mp4FileKind>` を `src/mp4/` 配下に追加する。
   - `Mp4FileKindDetector` を `required_input` 駆動で incremental に動かし、ファイル先頭のみ読む (`initialize_mp4_demuxer` と同じ `File::seek` + `read_exact` パターン)。
   - 拡張子では判定しない。`.mp4` 拡張子に fMP4 が入っているのが普通であり、実体で判定するのが筋。
   - `ContainerFormat` は webm/mp4 の 2 値のまま据え置く (外部 API 互換維持)。`Fmp4` は追加しない。
2. demuxer の差を吸収する薄い `enum Mp4Demuxer { Mp4(Mp4FileDemuxer), Fmp4(Fmp4FileDemuxer) }` を追加する。
   - 前方読みに必要な `required_input()` / `handle_input()` / `tracks()` / `next_sample()` だけを統一インターフェースとして公開し、各バリアントへ match で委譲する (`next_sample` が返す `Sample<'_>` のライフタイムは enum 自身の借用に紐づく)。
   - `seek()` / `prev_sample()` は持たせない。前方読み専用の抽象とする。
3. 前方読み専用の軽量 reader (仮称 `Mp4SampleReader`) を追加する。
   - `detect_mp4_file_kind` の結果に応じて `Mp4Demuxer` を構築し、`next_sample()` を回して `AudioFrame` / `VideoFrame` を `TrackPublisher` へ送出し、EOS で終了するだけ。
   - warm-up / realtime / decoder / seek / loop / pause は一切持たせない (inspect が使っていないため不要)。
   - inspect は Mp4/Fmp4 のどちらでもこの 1 本を使う。
4. `Mp4FileReader::new()` 冒頭に `detect_mp4_file_kind` による fail-fast を追加し、fMP4 なら再生前に明示エラーで返す (例: `"Fmp4 is not supported by Mp4FileReader yet"`)。
   - inspect は段階 1 で軽量 reader に移行済みのため、この fail-fast が守るのは OBSWS / 録画合成経路のみになる。
5. 前方読みの共通ロジック (sample_entry からの format 判定、sample data 読み込み、timestamp 計算、トラック選択) を `pub(crate)` の関数/型として切り出し、`Mp4FileReader` と `Mp4SampleReader` で共有する。
   - `select_audio_track` / `select_video_track` は `Mp4Demuxer` を受け取れるように一般化する。

### 段階 2a: 録画合成 (recording_reader) の fMP4 対応 (別 issue)

- `Mp4VideoReader` / `Mp4AudioReader` は前方読み (Iterator) のみで seek を使わないため、内部 demuxer を段階 1 の `Mp4Demuxer` enum に差し替えれば `recording_reader.rs` は無変更で対応できる見込み。
- ただし Sora 録画合成の入力に外部 fMP4 が来るユースケースが現状あるかは要検討 (Sora 自身は通常 MP4/WebM を生成する)。需要が確認できてから着手する。
- 段階 1 の `Mp4Demuxer` を基盤として再利用するため、段階 1 完了後に最小コストで着手できる。

### 段階 2b: OBSWS メディア再生 (Mp4FileReader) の fMP4 対応 (別 issue)

- `Mp4FileReader` の OBSWS 再生は `seek()` / `prev_sample()` / warm-up に依存するため、`shiguredo_mp4` 側で `Fmp4FileDemuxer` に `seek()` 等が追加されてから着手する。それまでは段階 1 の fail-fast を維持する。
- `probe_mp4_track_availability` / `probe_mp4_video_dimensions` 自体は前方読みのみで fMP4 対応可能だが、再生段で fail-fast する以上、probe だけ先に対応しても OBSWS では結局再生できず中途半端になる。probe の fMP4 対応は段階 2b でまとめて扱う。

## 完了条件

- `cargo test` がすべて成功すること。
- inspect コマンドで fMP4 ファイル (`testdata/` に新規追加するサンプル) に対して、トラック情報・サンプル数・タイムスタンプが、対応する通常 MP4 と整合的に出力されること。
- inspect の既存 MP4 経路に回帰がないこと (軽量 reader への移行後も、既存 MP4 ファイルの inspect 出力が変わらないこと)。
- `Mp4FileReader::new()` に fMP4 ファイルが渡された場合、デコードや再生を試みる前に明示エラーで失敗すること (OBSWS / 録画合成側で不可解な挙動にならないこと)。
- CHANGES.md の `## develop` に `[ADD] inspect コマンドが fMP4 ファイルの読み込みに対応する` を追記すること。
- 段階 2a (録画合成) と段階 2b (OBSWS 再生) の issue ファイルを `issues/` 配下に作成しておくこと (`issues/SEQUENCE` を更新すること)。

## 解決方法

### 実装ステップ

1. `src/mp4/` に種別判定モジュール (仮 `file_kind.rs`) を追加し、`detect_mp4_file_kind` を実装する。`Mp4FileKindDetector` を `required_input` 駆動でファイル先頭だけ読み込んで判定する。`src/mp4.rs` に `pub mod file_kind;` を追加する。
2. `src/mp4/` に demuxer 抽象モジュール (仮 `demuxer.rs`) を追加し、`enum Mp4Demuxer { Mp4, Fmp4 }` と `required_input` / `handle_input` / `tracks` / `next_sample` の委譲を実装する。Mp4/Fmp4 それぞれの構築用コンストラクタを用意する。
3. `src/mp4/reader.rs` の前方読み共通ロジックを `pub(crate)` 化して再利用可能にする。
   - sample_entry → 音声/映像 format 変換 (現 `ReaderState::update_audio_format` / `update_video_format`)。
   - sample data 読み込み (現 `ReaderState::read_sample_data` 相当を `File` ベースの関数へ)。
   - `calculate_timestamps` (既存自由関数)。
   - `select_audio_track` / `select_video_track` を `Mp4Demuxer` 対応に一般化。
   - 切り出し先は reader.rs に残して `pub(crate)` 公開するか、demuxer モジュール等の共通モジュールへ移すかは実装時に判断する。
4. `src/mp4/` に前方読み専用 reader (仮 `Mp4SampleReader`) を追加する。`detect_mp4_file_kind` → `Mp4Demuxer` 構築 → `next_sample()` ループ → `AudioFrame` / `VideoFrame` を `TrackPublisher` へ送出 → EOS、という最小実装にする。
   - メモリを圧迫しないよう、`Mp4FileReader::TrackSender` 相当の ack バックプレッシャー (`MAX_NOACKED_COUNT` ごとに `send_syn` の ack を待つ) を持たせる。一気に全サンプルを送出しないこと。
   - composition_time_offset を持つサンプルは既存踏襲でエラーにする。
5. `src/mp4/reader.rs` の `Mp4FileReader::new()` 冒頭に `detect_mp4_file_kind` を呼ぶ fail-fast を追加する (fMP4 で即エラー)。
6. `src/subcommand_inspect.rs` の `ContainerFormat::Mp4` ブランチを `Mp4SampleReader` を使う形に置き換え、Mp4/Fmp4 のどちらもこの軽量 reader で処理する。`Mp4FileReader` への依存を除去する。
7. `testdata/` に fMP4 サンプル (映像のみ・音声のみ・両方の最低 1 ファイル) を追加する。
   - 既存の対応する通常 MP4 と同内容で生成し、inspect 出力の整合を検証できるようにする。
   - `Fmp4FileDemuxer` が非対応の `tfhd.base_data_offset` 絶対オフセット形式を避ける生成指定 (例: `ffmpeg -movflags +frag_keyframe+empty_moov+default_base_is_moof`) を用い、生成手順を `testdata/` の README か本 issue に記録する。
   - 段階 1 は composition_time_offset 非対応のため、B フレームを含まないサンプル列になるよう生成する (例: `-bf 0` を指定する)。
8. テストを追加する。
   - `detect_mp4_file_kind` の単体テスト (通常 MP4 / fMP4 / 不正バイナリ)。命名規則に従い `tests/test_<module>.rs` に置く。
   - `Mp4FileReader::new()` に fMP4 を渡した場合の fail-fast エラーの単体テスト。
   - inspect の E2E テストとして、fMP4 ファイルを inspect した出力が、対応する通常 MP4 と整合的な JSON 形になることを検証する (既存の E2E テスト枠組みに合わせる)。
9. 段階 2a (録画合成) と段階 2b (OBSWS 再生) の issue ファイルを作成し、根拠と保留理由を書いておく (`issues/SEQUENCE` を参照・更新)。

### 共通化の判断

- inspect の前方読みは「encoded sample を読んで publish するだけ」で decoder すら持たないため、`Mp4FileReader` から切り出す前方読みロジックはごく小さい。軽量 reader 1 本化のコストは低い。
- demuxer の差は `Mp4Demuxer` enum で吸収し、reader 本体を 1 本にすることで前方読みパスの二重化を避ける。trait object ではなく enum を採用し、分岐を読みやすく保つ。
- `data_offset` がライブラリ側でファイル絶対位置に揃えられている (`Fmp4FileDemuxer` 側で `moof_offset + data_offset` 加算済み) ため、データ読み出しロジックは Mp4/Fmp4 で完全に共有できる。
- `Sample` / `SampleEntry` も両 demuxer で共通なので、サンプル受け取り後の format 判定や `TrackPublisher` 送出ロジックも共有できる。
- `Mp4Demuxer` は段階 2a の録画合成展開でも再利用できる。段階 1 で薄く作っておくことで、段階 2a が最小コストになる。

### リスク・留意事項

- composition_time_offset (B フレーム由来の CTS オフセット) を持つサンプルは、既存の `Mp4FileReader` / `recording_mp4_reader` と同様に段階 1 でも非対応とし、エラーで弾く。配信用 fMP4 は B フレームを使うことが多く、外部ファイルではこの制約に当たりやすい。B フレームありの fMP4 まで inspect できるようにする対応は、前方読みパス全体に関わる横断的な課題のため、将来の別 issue として切り出す。
- inspect を `Mp4SampleReader` へ移行する際、既存 MP4 経路の inspect 出力が変わらないことを担保する (inspect は前方読みのみ使用のため出力は不変の想定だが、テストで確認する)。
- `Fmp4FileDemuxer` は `tfhd.base_data_offset` がファイル絶対オフセットの形式に非対応。testdata はこの形式を避けて生成する。実ユーザーが該当ファイルを持ち込んだ場合はライブラリが返すエラーをそのまま inspect のエラーとして表示する。
- inspect の JSON 出力は段階 1 では mp4 と fmp4 を区別しない (`"format": "mp4"`)。区別したい要望が出てきたら別途検討する。
- fMP4 ファイルの `track.duration` は init segment 由来 (実値ではないことがある)。inspect の `audio_duration_us` / `video_duration_us` は既存実装どおりサンプル累積で算出するため影響しない見込みだが、最後のサンプルの duration が取れない既知の挙動は維持される。
