# hisui ライブラリ依存の examples/sumomo バイナリを追加する

- Created: 2026-08-03
- Completed: {YYYY-MM-DD}
- Branch: feature/add-sumomo-example
- Polished: {YYYY-MM-DD}

## 目的

hisui を Media Processing Tool としてライブラリ利用できることを示す最初の実証として、
sora-cpp-sdk の sumomo（momo の sora モードを模したサンプル）相当を
`examples/sumomo` バイナリとして追加する。

既存 examples（`examples/sora_publish` / `examples/camera_sora_grid` 等）は
`hisui` クレートに依存せず、起動済み `hisui server` に OBSWS で接続するクライアントである。
これでは「hisui をライブラリとしてアプリを組む」利用形態を示せない。
`examples/sumomo/Cargo.toml` で `hisui = { path = "../.." }` を依存に持ち、
`MediaPipeline` 上でデバイス入力と Sora 送受信を直接組み立てるバイナリが必要である。

## 前提

- hisui のライブラリ化（埋め込み可能な公開 API 境界）は本 issue の前提であり、全体設計は別途進める
- 本 issue では sumomo が要求する最小公開面だけを依存として扱う
- momo フル機能（Ayame / 内蔵 P2P シグナリング / DataChannel シリアル等）は対象外

## 現状

- `src/lib.rs` は存在するが、安定した埋め込み向け公開 API としては設計されていない
- Sora リアルタイム I/O の核は次のとおり `pub(crate)` であり、クレート外の example から直接使えない
  - `src/lib.rs` の `sora_publisher` / `sora_source` モジュール
  - `src/sora_publisher.rs` の `SoraPublisher` / `create_processor`
  - `src/sora_source.rs` の `SoraSubscriber`（RecvOnly）と関連イベント型
- デバイス入力は `src/obsws/source/video_device.rs` の `VideoDeviceSource` と
  `src/obsws/source/audio_device.rs` の `AudioDeviceSource` にあり、
  現状は `obsws` 経路（`ObswsSourceRequest` 経由）での利用が主である
- Sora 送信の実配線例は `src/obsws/coordinator/output_sora.rs` が
  `SoraPublisher` を `create_processor` で起動している
- workspace の examples はすべて `hisui` 非依存（`Cargo.toml` の `workspace.members` 参照）
- momo / sumomo 連携を扱う open issue は存在しない

## 設計方針

### 成果物の形

- `examples/sumomo/` を新規追加し、ルート `Cargo.toml` の `workspace.members` に登録する
- `examples/sumomo/Cargo.toml` は少なくとも次を満たす

```toml
[package]
name = "sumomo"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
hisui = { path = "../.." }
```

- 既存 OBSWS クライアント型 examples とは系統を分け、`hisui server` / OBSWS を起動せずに動作させる
- 参照原型は sora-cpp-sdk の `examples/sumomo`（CLI の必須セットと role の意味）

### CLI（初版）

C++ sumomo の必須オプションに寄せる。

- `--signaling-url`（必須）
- `--channel-id`（必須）
- `--role`（必須）: `sendonly` / `recvonly` / `sendrecv`

初版の必須完了は **sendonly（カメラ / マイク → Sora）** とする。
`recvonly` / `sendrecv` は同 issue 内で実装できる範囲で進めるが、
完了判定の必須条件には含めない（後述）。

対象外（後続 issue）:

- Ayame / momo 型 P2P / DataChannel シリアル
- `--use-sdl` 相当のローカルプレビュー（hisui に SDL 表示経路なし）
- C++ sumomo の全オプション網羅（コーデック実装指定、spotlight 細目、証明書等）

### パイプライン組立

sumomo は `hisui` 公開 API 経由でおおよそ次を直接組み立てる。

```text
sendonly:
  VideoDeviceSource / AudioDeviceSource
    -> (必要なら) encoder 系 processor
    -> SoraPublisher (SendOnly)
```

- `MediaPipeline` / `MediaPipelineHandle` / `TrackPublisher` 等は既に `src/lib.rs` で re-export 済み
- `SoraPublisher` / `SoraSubscriber`（および `create_processor` 相当）を example から呼べる公開境界にする
- `VideoDeviceSource` / `AudioDeviceSource` を obsws 専用に閉じず、example から組み立て可能にする
  （モジュール移動・re-export・薄い公開ラッパのいずれでもよい。ライブラリ化全体の方針に従う）
- `SoraSubscriber` は現状 RecvOnly と coordinator 向け `SoraSourceEvent` 前提のため、
  recvonly / sendrecv を同 issue で扱う場合は example 側でイベントを消費できる形にするか、
  埋め込み向けの最小 API を足す

### 公開範囲の原則

- ライブラリ化全体の API 凍結・semver・crate 分割は本 issue では行わない
- sumomo がコンパイル・動作するために必要なシンボルだけを公開する
- obsws プロトコルや coordinator 内部型を example に漏らさない

## 完了条件

- `examples/sumomo` が `hisui = { path = "../.." }` 依存でビルドできる
- `cargo run -p sumomo -- --signaling-url <URL> --channel-id <ID> --role sendonly` で
  実 Sora に接続し、カメラまたはマイク由来のメディアを送信できる
- 実行時に `hisui server` / OBSWS を起動しない
- Ayame / P2P / SDL / シリアルを実装していない（意図的に対象外）

任意（必須ではない）:

- `--role recvonly` または `sendrecv` で受信経路が動く

## 解決方法

1. hisui ライブラリ側で sumomo が使う最小公開面を切る
   - `src/lib.rs`: `sora_publisher` / `sora_source`（または同等の公開モジュール）を crate 外から利用可能にする
   - `SoraPublisher` / `create_processor`、必要なら `SoraSubscriber` と関連型
   - `VideoDeviceSource` / `AudioDeviceSource`（または同等のデバイス source）を example から利用可能にする
2. `examples/sumomo/` を追加する（`Cargo.toml` / `src/main.rs`）
3. ルート `Cargo.toml` の `workspace.members` に `examples/sumomo` を追加する
4. CLI を noargs で定義し、role に応じてパイプラインを組み立てて起動する
5. sendonly 経路を実 Sora で手動確認する（モック / スタブは使わない）
6. 必要なら examples 向けの短い README を `examples/sumomo/` に置く
7. 利用者から見える追加であれば `CHANGES.md` に `[ADD]` を記載する

## テスト方針

- モックやスタブは使わない
- 自動テストは「`examples/sumomo` が hisui 依存でビルドできること」を最低限とする
- Sora への実接続確認は手動（実デバイスと実シグナリング URL が必要なため）

## 対象外・後続

- hisui ライブラリ化の全体設計・安定 API ドキュメント
- momo 相当（Ayame / P2P / シリアル / 広範 HWA CLI）の移植
- 既存 OBSWS クライアント型 examples の書き換えや削除
- SDL プレビュー
