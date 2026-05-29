# fMP4 ファイルの読み込みに対応する

- Priority: Medium
- Created: 2026-05-29
- Completed:
- Model: Opus 4.7
- Branch: feature/add-fmp4-read-support
- Polished: 2026-05-29

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
- `src/mp4/reader.rs`
  - `Mp4FileDemuxer::new()` を直接生成して使用。
  - `initialize_mp4_demuxer` のコメントに「NOTE: fMP4 には未対応なので、この関数完了後、demuxer はファイル読み込みを要求しない」と既に記載されている。
  - `Mp4FileReader` は `seek()` / `prev_sample()` を用いた OBSWS メディア再生機能 (warm-up / シーク / 一時停止) に依存している。
- `src/sora/recording_mp4_reader.rs`
  - Sora の録画合成パイプライン用。前方読みのみ。`Mp4FileDemuxer` を直接生成。
- `src/subcommand_inspect.rs`
  - `Mp4FileReader::new()` 経由で `Mp4FileDemuxer` を使う構成。inspect のロジック自体は前方読み (`next_sample`) のみで成立する。
- `src/obsws/source/file_mp4.rs` / `src/obsws/state.rs` / `src/obsws/source/mp4.rs`
  - OBSWS のメディア入力で `Mp4FileReader` と `probe_mp4_track_availability` / `probe_mp4_video_dimensions` を利用。

### `shiguredo_mp4` ライブラリ (2026.3.0) で利用可能な API

- `demux::Mp4FileDemuxer`: 通常 MP4 用。`tracks() / next_sample() / prev_sample() / seek() / handle_input() / required_input()` を持つ。
- `demux::Fmp4FileDemuxer`: fMP4 用。`tracks() / next_sample() / handle_input() / required_input()` のみ。`seek()` と `prev_sample()` は提供されていない。
- `demux::Mp4FileKindDetector`: ファイル先頭 (`ftyp` + `moov` まで) を incremental に読んで `Mp4` か `FragmentedMp4` かを返す。
- `demux::Sample` 構造体は両 demuxer で共通。`Fmp4FileDemuxer` の `data_offset` もファイル絶対位置に揃えられている。

### 制約

- `Fmp4FileDemuxer` には現状 `seek()` / `prev_sample()` が無いため、`Mp4FileReader` の OBSWS 連携で使っている warm-up 付きシーク・後方走査をそのまま fMP4 に拡張することはできない。
- `Fmp4FileDemuxer` の制限事項として「`tfhd` の `base_data_offset` にファイル絶対オフセットが入っている形式には未対応」と明記されている。

## 設計方針

2 段階で進める。まず段階 1 で「fMP4 を読めない」状態を解消し、段階 2 で本格的に取り込めるようにする。両者の境界を本 issue で明確に切り、段階 2 は別 issue として切り出す。

### 段階 1: inspect コマンドだけ fMP4 に対応する (本 issue の範囲)

1. `ContainerFormat` に `Fmp4` を追加するのではなく、`Mp4FileKindDetector` でファイル種別を判定するヘルパー `detect_mp4_file_kind(path) -> Result<Mp4FileKind>` を `src/mp4/` 配下に追加する。
   - 拡張子で fmp4 を区別する流派 (`.m4s` など) もあるが、`.mp4` 拡張子に fMP4 が入っていることが普通なので、拡張子ではなく実体で判定するのが筋。
   - `ContainerFormat` 自体は webm/mp4 の 2 値のままにしておく (外部 API 互換維持)。
2. `src/mp4/reader.rs` から、共通化したい「ファイル読み込み・サンプル列挙」用のごく薄い trait もしくは enum 型を切り出す。
   - 仮称 `Mp4Demuxer { Mp4(Mp4FileDemuxer), Fmp4(Fmp4FileDemuxer) }` の enum で `tracks() / next_sample()` だけを統一インターフェース化する案を第一候補とする (trait object よりも分岐が見えて読みやすい)。
   - `seek()` / `prev_sample()` は段階 1 では mp4 のみ対応とし、fmp4 のケースでは未サポート扱いとする。
3. `src/subcommand_inspect.rs` の MP4 分岐で、上記判定結果に応じて
   - `Mp4FileKind::Mp4` → 既存の `Mp4FileReader` を使う
   - `Mp4FileKind::FragmentedMp4` → 新規に追加する `Fmp4InspectReader` (仮称) で fMP4 を読み出し、`MediaPipeline` に encoded サンプルを流す
   ようにする。
4. fMP4 用 inspect reader は inspect の用途に特化した最小実装にする。
   - `Mp4FileReader` の OBSWS 連携機能 (シーク、ループ、一時停止) は持たせない。
   - `realtime: false` 相当の動作 (= 一気にサンプルを送出して EOS) のみサポートする。
   - サンプル送出後の処理 (デコード/集計) は既存の OutputPrinter / Decoder にそのまま流す。

### 段階 2: 録画合成・OBSWS への適用 (別 issue として切り出す)

- `Mp4FileReader` の fMP4 対応 (OBSWS メディア再生では `seek()` / `prev_sample()` を fmp4 でも使いたいケースが出てくる) は、`shiguredo_mp4` 側で `Fmp4FileDemuxer` に `seek()` 等が追加されてから検討する。
- 段階 1 完了時点では、`Mp4FileReader::new()` が fMP4 ファイルを開いた際に「fMP4 は未対応」エラーを早期に返すように `Mp4FileKindDetector` で fail-fast する。OBSWS / 録画合成側で fMP4 を誤って渡された場合の挙動を明確化する。
- 段階 2 の issue にて、`recording_mp4_reader.rs` が fMP4 を扱う場合の方針 (前方読みのみで合成可能か、sample_entry のキャッシュをどう扱うか) も併せて整理する。

## 完了条件

- `cargo test` がすべて成功すること。
- inspect コマンドで fMP4 ファイル (`testdata/` に新規追加するサンプル) に対して、トラック情報・サンプル数・タイムスタンプが、対応する通常 MP4 と整合的に出力されること。
- `Mp4FileReader::new()` に fMP4 ファイルが渡された場合、デコードや再生を試みる前に「fMP4 is not supported by Mp4FileReader yet」相当の明示エラーで失敗すること (OBSWS / 録画合成側で不可解な挙動にならないこと)。
- CHANGES.md の `## develop` に `[ADD] inspect コマンドが fMP4 ファイルの読み込みに対応する` を追記すること。
- 段階 2 の issue ファイル (`Mp4FileReader` 側の fMP4 対応) を `issues/` 配下に作成しておくこと。

## 解決方法

### 実装ステップ

1. `src/mp4/` に `detect_mp4_file_kind` を実装し、`Mp4FileKindDetector` 駆動でファイル先頭部分を都度読み込んで判定する。
   - 既存の `initialize_mp4_demuxer` と同じ「`required_input` を満たすデータを `File::seek` + `read_exact` で供給する」パターンで書く。
2. `src/mp4/reader.rs` の `Mp4FileReader::new()` 先頭で `detect_mp4_file_kind` を呼び出し、fMP4 ならば即エラーで返す (`"Fmp4 is not supported by Mp4FileReader yet"` のようなメッセージ)。
3. `src/mp4/` に `inspect_fmp4_reader.rs` を新規追加し、`Fmp4FileDemuxer` を用いて `next_sample()` を回し、`AudioFrame` / `VideoFrame` を `TrackPublisher` に流す最小 reader を実装する。
   - `Mp4FileReader` の `handle_audio_sample` / `handle_video_sample` 相当のコードのうち、warm-up / realtime / pending_seek を除いた部分を切り出す。
   - 共通化できるユーティリティ (`update_audio_format`, `update_video_format`, `read_sample_data`, `calculate_timestamps`) は `pub(crate)` で reader.rs から共有する。
4. `src/subcommand_inspect.rs` の `setup_pipeline` 内、`ContainerFormat::Mp4` のブランチで `detect_mp4_file_kind` の結果に応じて新旧 reader を切り替える。
5. `testdata/` に fMP4 のサンプルファイル (映像のみ・音声のみ・両方の最低 1 ファイル) を追加する。
   - サンプルは hisui 自身で生成できない場合、外部ツール (e.g. `ffmpeg -movflags +frag_keyframe+empty_moov+default_base_is_moof`) で作成する手順を `testdata/` の README か issue 内に記録する。
6. 単体テストを追加する。
   - `tests/test_mp4_reader.rs` 相当のファイルに `detect_mp4_file_kind` のテスト (通常 MP4 / fMP4 / 不正バイナリ)。
   - inspect の E2E テストとして、fMP4 ファイルを `cargo run -- inspect` した出力が想定の JSON 形になることを検証する (既存の E2E テスト枠組みに合わせる)。
7. 段階 2 用の issue ファイル (`Mp4FileReader` の fMP4 対応) を `issues/0002-feature-add-mp4-reader-fmp4-support.md` 相当で作成し、根拠と保留理由を書いておく。

### 共通化の判断

- inspect の用途では `next_sample()` のみで足りるため、最小限の trait/enum で十分に共通化できる。`Mp4FileReader` 全体の共通化は段階 2 に回す。
- `data_offset` がライブラリ側でファイル絶対位置に揃えられている (`Fmp4FileDemuxer` 側で `moof_offset + sample.data_offset` を加算済み) ため、データ読み出しロジックは MP4 / fMP4 で完全に共有できる。
- `Sample` 構造体・`SampleEntry` も両 demuxer で共通なので、サンプル受け取り後の format 判定や TrackPublisher 送出ロジックも共有できる。

### リスク・留意事項

- `Fmp4FileDemuxer` は「`tfhd.base_data_offset` がファイル絶対オフセットの形式」未対応。実ユーザーが持ち込むファイルで踏む可能性があり、踏んだ場合は明示的にエラーを返す。
- inspect の JSON 出力は段階 1 では mp4 と fmp4 で区別しない (`"format": "mp4"`)。区別したい要望が出てきたら別途検討する。
- fMP4 ファイルの `track.duration` は init segment 由来 (実値ではないことがある)。inspect の `audio_duration_us` / `video_duration_us` は既存実装どおりサンプル累積で算出するため影響しない見込みだが、最後のサンプルの duration が取れない既知の挙動は維持される。
