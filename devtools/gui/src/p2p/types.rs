//! P2P 接続の型定義。
//!
//! `devtools/src/p2p/types.ts` の Rust 移植。

/// 接続状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// 未接続
    Idle,
    /// HTTP Bootstrap 実行中
    Bootstrapping,
    /// WebRTC ハンドシェイク中
    Connecting,
    /// 接続済み
    Connected,
    /// 切断処理中
    Disconnecting,
    /// 切断済み
    Closed,
}

/// サーバーから送信される close コード。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseCode {
    UnknownType,
    Timeout,
    SdpError,
    SrdError,
    Unexpected,
    MissingSdp,
}

impl CloseCode {
    /// close コードの文字列表現から enum へ変換する。未知のコードは None。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unknown-type" => Some(Self::UnknownType),
            "timeout" => Some(Self::Timeout),
            "sdp-error" => Some(Self::SdpError),
            "srd-error" => Some(Self::SrdError),
            "unexpected" => Some(Self::Unexpected),
            "missing-sdp" => Some(Self::MissingSdp),
            _ => None,
        }
    }
}

/// サーバーから送信される close メッセージ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseMessage {
    pub code: CloseCode,
    pub reason: String,
}

/// サーバーから送信されるメッセージ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage {
    /// サーバー主導の renegotiation offer
    Offer { sdp: String },
    /// セッション終了通知
    Close(CloseMessage),
}

/// クライアントから送信されるメッセージ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// renegotiation offer への answer
    Answer { sdp: String },
    /// 切断要求
    Disconnect,
}

/// DataChannel の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataChannelState {
    /// DataChannel が作成されていない
    NotCreated,
    /// 接続中
    Connecting,
    /// オープン済み
    Open,
    /// クローズ中
    Closing,
    /// クローズ済み
    Closed,
}

/// DataChannel の状態 (signaling / obsdc)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataChannelStates {
    pub signaling: DataChannelState,
    pub obsdc: DataChannelState,
}

/// HTTP Bootstrap の設定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapConfig {
    pub bootstrap_url: String,
    pub ice_servers: Vec<IceServer>,
    pub data_channel_only: bool,
}

/// ICE サーバーの設定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// ログレベル。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// ログカテゴリ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogCategory {
    /// PeerConnection
    Pc,
    /// シグナリング DataChannel
    Signaling,
    /// OBS WebSocket DataChannel
    Obsdc,
}

impl std::fmt::Display for LogCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pc => write!(f, "pc"),
            Self::Signaling => write!(f, "signaling"),
            Self::Obsdc => write!(f, "obsdc"),
        }
    }
}

/// ログエントリ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub category: LogCategory,
    pub message: String,
}
