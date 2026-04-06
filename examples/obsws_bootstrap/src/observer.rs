use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use shiguredo_webrtc::{
    AudioTrackSinkHandler, DataChannelObserver, DataChannelObserverHandler, IceGatheringState,
    PeerConnectionObserverHandler, PeerConnectionState, VideoSinkHandler,
};

use crate::event::{AudioFrameData, ClientEvent, EventTx, IceObserverEvent, IceTx, VideoFrameData};

pub struct ClientPcObserver {
    pub event_tx: EventTx,
    pub ice_tx: IceTx,
}

impl PeerConnectionObserverHandler for ClientPcObserver {
    fn on_connection_change(&mut self, state: PeerConnectionState) {
        let _ = self.event_tx.send(ClientEvent::ConnectionChange(state));
    }

    fn on_track(&mut self, transceiver: shiguredo_webrtc::RtpTransceiver) {
        let _ = self.event_tx.send(ClientEvent::Track(transceiver));
    }

    fn on_data_channel(&mut self, mut dc: shiguredo_webrtc::DataChannel) {
        let label = dc.label().unwrap_or_default();
        let observer = if label == "signaling" {
            let observer = DataChannelObserver::new_with_handler(Box::new(SignalingDcHandler {
                event_tx: self.event_tx.clone(),
            }));
            dc.register_observer(&observer);
            Some(observer)
        } else if label == "obsdc" {
            let observer = DataChannelObserver::new_with_handler(Box::new(ObswsDcHandler {
                event_tx: self.event_tx.clone(),
            }));
            dc.register_observer(&observer);
            Some(observer)
        } else {
            None
        };
        let _ = self.event_tx.send(ClientEvent::DataChannel(dc, observer));
    }

    fn on_ice_gathering_change(&mut self, state: IceGatheringState) {
        if state == IceGatheringState::Complete {
            let _ = self.ice_tx.send(IceObserverEvent::Complete);
        }
    }

    fn on_ice_candidate(&mut self, candidate: shiguredo_webrtc::IceCandidateRef<'_>) {
        let Ok(sdp_mid) = candidate.sdp_mid() else {
            return;
        };
        let sdp_mline_index = candidate.sdp_mline_index();
        let Ok(candidate) = candidate.to_string() else {
            return;
        };
        let _ = self.ice_tx.send(IceObserverEvent::Candidate {
            sdp_mid,
            sdp_mline_index,
            candidate,
        });
    }
}

pub struct SignalingDcHandler {
    pub event_tx: EventTx,
}

impl DataChannelObserverHandler for SignalingDcHandler {
    fn on_message(&mut self, data: &[u8], _is_binary: bool) {
        let _ = self.event_tx.send(ClientEvent::SignalingMessage {
            data: data.to_vec(),
        });
    }
}

pub struct ObswsDcHandler {
    pub event_tx: EventTx,
}

impl DataChannelObserverHandler for ObswsDcHandler {
    fn on_state_change(&mut self) {
        let _ = self.event_tx.send(ClientEvent::ObswsDataChannelStateChange);
    }

    fn on_message(&mut self, data: &[u8], _is_binary: bool) {
        let _ = self.event_tx.send(ClientEvent::ObswsMessage {
            data: data.to_vec(),
        });
    }
}

// フレームデータをチャネルで送信するハンドラ
pub struct FrameRecordHandler {
    pub track_id: String,
    pub frame_count: Arc<AtomicUsize>,
    pub first_frame_logged: Arc<AtomicBool>,
    pub frame_tx: std::sync::mpsc::SyncSender<VideoFrameData>,
}

impl VideoSinkHandler for FrameRecordHandler {
    fn on_frame(&mut self, frame: shiguredo_webrtc::VideoFrameRef<'_>) {
        let previous = self.frame_count.fetch_add(1, Ordering::Relaxed);
        let w = frame.width();
        let h = frame.height();
        if previous == 0 && !self.first_frame_logged.swap(true, Ordering::Relaxed) {
            tracing::info!(
                "first video frame received: width={w}, height={h}, timestamp_us={}",
                frame.timestamp_us()
            );
        }

        // I420 バッファからプレーンデータをコピーする
        let mut frame_buffer = frame.buffer();
        let Some(i420) = frame_buffer.to_i420() else {
            return;
        };
        let data = VideoFrameData {
            track_id: self.track_id.clone(),
            y: i420.y_data().to_vec(),
            u: i420.u_data().to_vec(),
            v: i420.v_data().to_vec(),
            width: w,
            height: h,
            stride_y: i420.stride_y() as usize,
            stride_u: i420.stride_u() as usize,
            stride_v: i420.stride_v() as usize,
            timestamp_us: frame.timestamp_us(),
        };
        // バッファがいっぱいの場合はフレームを捨てる
        let _ = self.frame_tx.try_send(data);
    }
}

// 受信音声データをチャネルで送信するハンドラ
pub struct AudioRecordHandler {
    pub track_id: String,
    pub audio_frame_count: Arc<AtomicUsize>,
    pub audio_tx: std::sync::mpsc::SyncSender<AudioFrameData>,
}

impl AudioTrackSinkHandler for AudioRecordHandler {
    fn on_data(
        &mut self,
        audio_data: &[u8],
        bits_per_sample: i32,
        sample_rate: i32,
        number_of_channels: usize,
        _number_of_frames: usize,
    ) {
        self.audio_frame_count.fetch_add(1, Ordering::Relaxed);
        if bits_per_sample != 16 {
            return;
        }
        // u8 スライスをネイティブエンディアン i16 に変換する
        let pcm: Vec<i16> = audio_data
            .chunks_exact(2)
            .map(|chunk| i16::from_ne_bytes([chunk[0], chunk[1]]))
            .collect();
        let _ = self.audio_tx.try_send(AudioFrameData {
            track_id: self.track_id.clone(),
            pcm,
            sample_rate,
            channels: number_of_channels,
        });
    }
}
