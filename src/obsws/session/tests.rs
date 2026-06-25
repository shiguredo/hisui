// `src/obsws/session.rs` から `#[cfg(test)] #[path = "session/tests.rs"]` で読み込まれるテストエントリポイント。
//
// 共通ヘルパーは `tests/common.rs` に集約し、各機能群のテストはサブモジュールに分かれている。
// 専用ヘルパーは利用するサブモジュール内に閉じる (例: text_overlay 専用 3 件は `tests/text_overlay.rs` 内)。
// `output_player` は `#[cfg(feature = "player")]` でモジュール宣言ごとゲートしている。
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
