use shiguredo_webrtc::{DataChannel, DataChannelObserver, PeerConnectionState, RtpTransceiver};
use tokio::sync::mpsc;

pub enum ClientEvent {
    ConnectionChange(PeerConnectionState),
    Track(RtpTransceiver),
    DataChannel(DataChannel, Option<DataChannelObserver>),
    ObswsDataChannelStateChange,
    SignalingMessage { data: Vec<u8> },
    ObswsMessage { data: Vec<u8> },
}

// VideoSinkHandler から送信するフレームデータ
pub struct VideoFrameData {
    pub track_id: String,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub stride_y: usize,
    pub stride_u: usize,
    pub stride_v: usize,
    pub timestamp_us: i64,
}

// AudioTrackSinkHandler から送信する音声データ
pub struct AudioFrameData {
    pub track_id: String,
    pub pcm: Vec<i16>,
    pub sample_rate: i32,
    pub channels: usize,
}

pub enum IceObserverEvent {
    Candidate {
        sdp_mid: String,
        sdp_mline_index: i32,
        candidate: String,
    },
    Complete,
}

pub type EventTx = mpsc::UnboundedSender<ClientEvent>;
pub type EventRx = mpsc::UnboundedReceiver<ClientEvent>;
pub type IceTx = mpsc::UnboundedSender<IceObserverEvent>;
pub type IceRx = mpsc::UnboundedReceiver<IceObserverEvent>;
