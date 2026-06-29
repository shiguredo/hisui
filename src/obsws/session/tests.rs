//! `src/obsws/session.rs` から `#[cfg(test)] #[path = "session/tests.rs"]` で読み込まれるテストエントリポイント。
//!
//! 各機能群のテストはサブモジュールに分かれている (`mod` 宣言はアルファベット順):
//!
//! - `common`: テスト共通ヘルパー (`pub(super)` で公開)
//! - `input`: Input の作成 / 削除 / 設定変更 / 名前変更
//! - `lifecycle`: identify / sleep / broadcast / authentication / RPC version
//! - `output_create`: HisuiCreateOutput / SetOutputSettings / SetRecordDirectory
//! - `output_hls_dash`: HLS / DASH 出力経路
//! - `output_misc_lifecycle`: 上記いずれにも属さない Output 系混在群
//! - `output_player`: `#[cfg(feature = "player")]` でモジュール宣言ごとゲートされた player 系
//! - `output_record`: StartRecord / StopRecord
//! - `output_stream`: StartStream / StopStream
//! - `persistent_data`: SetPersistentData / GetPersistentData
//! - `request_batch`: RequestBatch の haltOnFailure 挙動
//! - `scene`: Scene の作成 / 切替 / 削除
//! - `scene_item`: SceneItem の作成 / 削除 / プロパティ更新 / 再インデックス
//! - `stream_service`: handle_get_stream_service_settings
//! - `text_overlay`: TextOverlay 機能の 4 メソッドと専用ヘルパー
#[path = "tests/common.rs"]
mod common;
#[path = "tests/input.rs"]
mod input;
#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/output_create.rs"]
mod output_create;
#[path = "tests/output_hls_dash.rs"]
mod output_hls_dash;
#[path = "tests/output_misc_lifecycle.rs"]
mod output_misc_lifecycle;
#[cfg(feature = "player")]
#[path = "tests/output_player.rs"]
mod output_player;
#[path = "tests/output_record.rs"]
mod output_record;
#[path = "tests/output_stream.rs"]
mod output_stream;
#[path = "tests/persistent_data.rs"]
mod persistent_data;
#[path = "tests/request_batch.rs"]
mod request_batch;
#[path = "tests/scene.rs"]
mod scene;
#[path = "tests/scene_item.rs"]
mod scene_item;
#[path = "tests/stream_service.rs"]
mod stream_service;
#[path = "tests/text_overlay.rs"]
mod text_overlay;
