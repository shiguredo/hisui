use shiguredo_webrtc::RtpTransceiver;

/// SoraSubscriber processor から coordinator へ通知するイベント。
///
/// sora_sdk のコールバック（on_track, on_remove_track, on_notify 等）は
/// 別スレッドから呼ばれるため、mpsc channel 経由で coordinator に転送する。
pub enum SoraSourceEvent {
    /// on_track: リモートトラック到着
    TrackReceived {
        subscriber_name: String,
        transceiver: RtpTransceiver,
    },
    /// on_remove_track: リモートトラック削除
    TrackRemoved {
        subscriber_name: String,
        track_id: String,
    },
    /// on_notify: シグナリング通知（JSON 文字列）
    Notify {
        subscriber_name: String,
        json: String,
    },
    /// on_websocket_close: WebSocket 切断
    WebSocketClose {
        subscriber_name: String,
        code: Option<u16>,
        reason: String,
    },
    /// SoraClient タスク終了（正常・異常問わず）
    Disconnected { subscriber_name: String },
}

/// SoraSubscriber の接続パラメータ。
#[derive(Clone)]
pub struct SoraSubscriber {
    pub subscriber_name: String,
    pub signaling_urls: Vec<String>,
    pub channel_id: String,
    pub client_id: Option<String>,
    pub bundle_id: Option<String>,
    pub metadata: Option<nojson::RawJsonOwned>,
    /// coordinator へのイベント送信チャネル
    pub event_tx: tokio::sync::mpsc::UnboundedSender<SoraSourceEvent>,
}

impl SoraSubscriber {
    pub async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let subscriber_name = self.subscriber_name.clone();
        let event_tx = self.event_tx.clone();

        // SoraClientContext を生成（RecvOnly なので外部 ADM は不要）
        let context = sora_sdk::SoraClientContext::new()
            .map_err(|e| crate::Error::new(format!("failed to create SoraClientContext: {e}")))?;

        // コールバック用のクローン
        let on_track_name = subscriber_name.clone();
        let on_track_tx = event_tx.clone();

        let on_remove_track_name = subscriber_name.clone();
        let on_remove_track_tx = event_tx.clone();

        let on_notify_name = subscriber_name.clone();
        let on_notify_tx = event_tx.clone();

        let on_ws_close_name = subscriber_name.clone();
        let on_ws_close_tx = event_tx.clone();

        // SoraClient を構築（RecvOnly）
        let mut builder = sora_sdk::SoraClient::builder(
            context,
            self.signaling_urls.clone(),
            self.channel_id.clone(),
            sora_sdk::Role::RecvOnly,
        )
        .on_track(move |transceiver| {
            let _ = on_track_tx.send(SoraSourceEvent::TrackReceived {
                subscriber_name: on_track_name.clone(),
                transceiver,
            });
        })
        .on_remove_track(move |receiver| {
            let track = receiver.track();
            let track_id = track.id().unwrap_or_default();
            let _ = on_remove_track_tx.send(SoraSourceEvent::TrackRemoved {
                subscriber_name: on_remove_track_name.clone(),
                track_id,
            });
        })
        .on_notify(move |json| {
            let _ = on_notify_tx.send(SoraSourceEvent::Notify {
                subscriber_name: on_notify_name.clone(),
                json: json.to_owned(),
            });
        })
        .on_websocket_close(move |code, reason| {
            let _ = on_ws_close_tx.send(SoraSourceEvent::WebSocketClose {
                subscriber_name: on_ws_close_name.clone(),
                code,
                reason: reason.to_owned(),
            });
        });

        if let Some(client_id) = &self.client_id {
            builder = builder.client_id(client_id.clone());
        }
        if let Some(bundle_id) = &self.bundle_id {
            builder = builder.bundle_id(bundle_id.clone());
        }
        if let Some(metadata) = &self.metadata {
            let json_string: sora_sdk::JsonString = metadata
                .to_string()
                .parse()
                .map_err(|e| crate::Error::new(format!("failed to parse metadata: {e}")))?;
            builder = builder.metadata(json_string);
        }

        let (client, client_handle) = builder
            .build()
            .map_err(|e| crate::Error::new(format!("failed to build SoraClient: {e}")))?;

        // Sora 接続を開始（バックグラウンドタスク）
        let disconnected_name = subscriber_name.clone();
        let disconnected_tx = event_tx.clone();
        let mut client_task = tokio::spawn(async move {
            if let Err(e) = client.run().await {
                tracing::warn!(
                    "SoraSubscriber '{}' terminated with error: {e}",
                    disconnected_name
                );
            }
            let _ = disconnected_tx.send(SoraSourceEvent::Disconnected {
                subscriber_name: disconnected_name,
            });
        });

        handle.notify_ready();

        // client_task の完了を待つ。
        // processor が TerminateProcessor で abort された場合はここで中断される。
        let _ = (&mut client_task).await;

        // 切断（まだ接続中の場合）
        if let Err(e) = client_handle.disconnect().await {
            tracing::warn!(
                "failed to disconnect SoraSubscriber '{}': {e}",
                subscriber_name
            );
        }
        // タスク終了を待ち、タイムアウト時は abort する
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), &mut client_task).await;
        client_task.abort();

        Ok(())
    }
}

pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    subscriber: SoraSubscriber,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    let processor_id = processor_id.unwrap_or_else(|| {
        crate::ProcessorId::new(format!("soraSubscriber:{}", subscriber.subscriber_name))
    });
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("sora_subscriber"),
            move |h| subscriber.run(h),
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}
