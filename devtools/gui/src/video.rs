//! 映像を GPUI 上に表示するコンポーネント。
//!
//! 入力は GPUI の [`RenderImage`] であり、WebRTC の受信や色変換は含まない。
//! トラックの追加・削除とタイルのドラッグ / リサイズだけを担当する。

mod tile;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{Context, Render, RenderImage, Window, div, rgb};

use tile::VideoTile;

/// 初回フレーム受信時の表示幅 (ピクセル)。高さはアスペクト比で決める。
const INITIAL_TILE_WIDTH: f32 = 480.0;

/// リサイズ時の最小幅 (ピクセル)
const MIN_TILE_WIDTH: f32 = 80.0;

/// リサイズ時の最小高さ (ピクセル)
const MIN_TILE_HEIGHT: f32 = 45.0;

/// 1 トラック分の表示状態。
struct TileState {
    image: Arc<RenderImage>,
    position: gpui::Point<f32>,
    size: gpui::Size<f32>,
}

/// 映像タイルを自由配置で表示する GPUI コンポーネント。
pub struct VideoDisplay {
    tiles: BTreeMap<String, TileState>,
    /// 接続処理中かどうか (プレースホルダ文言の切り替え用)
    connecting: bool,
    /// ドラッグ中の前回マウス位置
    drag_prev_mouse: Option<gpui::Point<f32>>,
}

impl VideoDisplay {
    /// 空の映像表示領域を作る。
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            tiles: BTreeMap::new(),
            connecting: false,
            drag_prev_mouse: None,
        }
    }

    /// トラックの最新フレームを表示する。
    ///
    /// 未知の `track_id` ならタイルを追加し、既知なら画像だけを更新する。
    /// 初回追加時の表示サイズは `width` / `height` のアスペクト比で決める。
    pub fn show_frame(
        &mut self,
        track_id: String,
        image: Arc<RenderImage>,
        width: i32,
        height: i32,
        cx: &mut Context<Self>,
    ) {
        match self.tiles.entry(track_id) {
            Entry::Occupied(mut occupied) => {
                occupied.get_mut().image = image;
            }
            Entry::Vacant(vacant) => {
                vacant.insert(TileState {
                    image,
                    position: gpui::point(0.0, 0.0),
                    size: initial_tile_size(width, height),
                });
            }
        }
        cx.notify();
    }

    /// トラックのタイルを取り除く。
    pub fn remove_track(&mut self, track_id: &str, cx: &mut Context<Self>) {
        self.tiles.remove(track_id);
        cx.notify();
    }

    /// 接続処理中かどうかをプレースホルダ表示に反映する。
    pub fn set_connecting(&mut self, connecting: bool, cx: &mut Context<Self>) {
        self.connecting = connecting;
        cx.notify();
    }

    /// ドラッグ開始時のマウス位置を記録する。
    fn begin_drag(&mut self, mouse: gpui::Point<f32>) {
        self.drag_prev_mouse = Some(mouse);
    }

    /// マウス移動量だけタイルを動かす。
    fn move_tile(&mut self, track_id: &str, mouse: gpui::Point<f32>) {
        let prev = self.drag_prev_mouse.unwrap_or(mouse);
        if let Some(tile) = self.tiles.get_mut(track_id) {
            tile.position.x += mouse.x - prev.x;
            tile.position.y += mouse.y - prev.y;
        }
        self.drag_prev_mouse = Some(mouse);
    }

    /// マウス移動量だけタイルをリサイズする。
    fn resize_tile(&mut self, track_id: &str, mouse: gpui::Point<f32>) {
        let prev = self.drag_prev_mouse.unwrap_or(mouse);
        if let Some(tile) = self.tiles.get_mut(track_id) {
            tile.size.width = (tile.size.width + mouse.x - prev.x).max(MIN_TILE_WIDTH);
            tile.size.height = (tile.size.height + mouse.y - prev.y).max(MIN_TILE_HEIGHT);
        }
        self.drag_prev_mouse = Some(mouse);
    }
}

impl Render for VideoDisplay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tiles.is_empty() {
            return div()
                .size_full()
                .items_center()
                .justify_center()
                .flex()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x888888))
                        .child(if self.connecting {
                            "接続中..."
                        } else {
                            "映像なし"
                        }),
                )
                .into_any_element();
        }

        let display = cx.entity();
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .children(self.tiles.iter().map(|(track_id, tile)| {
                VideoTile::new(
                    display.clone(),
                    track_id.clone(),
                    tile.image.clone(),
                    tile.position,
                    tile.size,
                )
            }))
            .into_any_element()
    }
}

/// 初回表示サイズを決める。幅を 480px に合わせ、高さはアスペクト比でスケールする。
fn initial_tile_size(width: i32, height: i32) -> gpui::Size<f32> {
    let scale = INITIAL_TILE_WIDTH / width.max(1) as f32;
    gpui::size(INITIAL_TILE_WIDTH, (height as f32 * scale).max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_tile_size_keeps_aspect_ratio() {
        assert_eq!(initial_tile_size(1920, 1080), gpui::size(480.0, 270.0));
        assert_eq!(initial_tile_size(640, 480), gpui::size(480.0, 360.0));
    }
}
