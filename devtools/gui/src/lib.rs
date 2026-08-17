//! Hisui DevTools のネイティブ GUI アプリ。
//!
//! ブラウザ向け devtools (`devtools/`) の機能を GPUI で実装し直したもので、
//! P2P 接続・映像再生・OBS WebSocket 操作を提供する。

pub mod error;
pub mod obsdc;
pub mod p2p;
pub mod video;
pub mod webrtc;

pub use error::{Error, Result};
