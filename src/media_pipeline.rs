#[derive(Debug)]
pub struct MediaPipeline {
    command_tx: Option<tokio::sync::mpsc::UnboundedSender<Command>>,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    processors: std::collections::HashSet<ProcessorId>,
    tracks: std::collections::HashMap<TrackId, TrackState>,
}

impl MediaPipeline {
    pub fn new() -> Self {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            command_tx: Some(command_tx),
            command_rx,
            processors: std::collections::HashSet::new(),
            tracks: std::collections::HashMap::new(),
        }
    }

    /// このパイプラインを操作するためのハンドルを返す
    ///
    /// 全てのハンドルが（間接的なものも含めて）ドロップしたら、パイプラインも終了する
    pub fn handle(&self) -> MediaPipelineHandle {
        MediaPipelineHandle {
            command_tx: self.command_tx.clone().expect("infallible"),
        }
    }

    pub async fn run(mut self) {
        log::debug!("MediaPipeline started");

        self.command_tx = None; // 参照カウントから自分を外すために None にする

        loop {
            tokio::select! {
                Some(command) = self.command_rx.recv() => self.handle_command(command),
                else => break,
            }
        }

        log::debug!("MediaPipeline stopped");
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::RegisterProcessor {
                processor_id,
                reply_tx,
            } => {
                let result = self.handle_register_processor(processor_id);
                let _ = reply_tx.send(result);
            }
            Command::DeregisterProcessor { processor_id } => {
                self.handle_deregister_processor(processor_id);
            }
            Command::PublishTrack {
                processor_id,
                track_id,
                reply_tx,
            } => {
                let result = self.handle_publish_track(processor_id, track_id);
                let _ = reply_tx.send(result);
            }
            Command::SubscribeTrack {
                processor_id,
                track_id,
                tx,
            } => {
                self.handle_subscribe_track(processor_id, track_id, tx);
            }
        }
    }

    fn handle_subscribe_track(
        &mut self,
        processor_id: ProcessorId,
        track_id: TrackId,
        tx: tokio::sync::mpsc::UnboundedSender<Message>,
    ) {
        log::debug!("subscribe track: processor={processor_id}, track={track_id}");

        if !self.processors.contains(&processor_id) {
            log::warn!("attempt to subscribe to track from unregistered processor: {processor_id}");
            return;
        }

        // トラックが存在しない場合は新規作成
        let track = self.tracks.entry(track_id.clone()).or_insert_with(|| {
            log::debug!("creating new track: {track_id}");
            TrackState::default()
        });

        if let Some(publisher_tx) = &track.publisher_command_tx {
            let _ = publisher_tx.send(TrackCommand::AddSubscriber(tx));
        } else {
            // publisher がまだ登録されていない場合は、subscriber を待機させる
            log::debug!("publisher not yet registered for track: {track_id}");
        }
    }

    fn handle_publish_track(
        &mut self,
        processor_id: ProcessorId,
        track_id: TrackId,
    ) -> Option<MessageSender> {
        log::debug!("publish track: processor={processor_id}, track={track_id}");

        if !self.processors.contains(&processor_id) {
            log::warn!("attempt to publish track from unregistered processor: {processor_id}");
            return None;
        }

        // トラックが存在しない場合は新規作成
        let track = self.tracks.entry(track_id.clone()).or_insert_with(|| {
            log::debug!("creating new track: {track_id}");
            TrackState::default()
        });

        if track.publisher_command_tx.is_some() {
            log::warn!("publisher conflict for track: processor={processor_id}, track={track_id}");
            return None;
        }

        // TODO: すでに存在する subscriber の情報を伝えるべき

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        track.publisher_command_tx = Some(command_tx);

        Some(MessageSender {
            rx: command_rx,
            txs: Vec::new(),
        })
    }

    fn handle_register_processor(&mut self, processor_id: ProcessorId) -> bool {
        log::debug!("register processor: {processor_id}");

        if self.processors.contains(&processor_id) {
            log::warn!("processor already registered: {processor_id}");
            return false;
        }

        self.processors.insert(processor_id.clone());
        true
    }

    fn handle_deregister_processor(&mut self, processor_id: ProcessorId) {
        log::debug!("deregister processor: {processor_id}");
        self.processors.remove(&processor_id);
    }
}

impl Default for MediaPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct MediaPipelineHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<Command>,
}

impl MediaPipelineHandle {
    pub async fn register_processor(&self, processor_id: ProcessorId) -> Option<ProcessorHandle> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::RegisterProcessor {
            processor_id: processor_id.clone(),
            reply_tx,
        };
        self.send(command);
        if let Ok(true) = reply_rx.await {
            Some(ProcessorHandle {
                pipeline_handle: self.clone(),
                processor_id,
            })
        } else {
            None
        }
    }

    fn send(&self, command: Command) {
        self.command_tx.send(command).expect("bug");
    }
}

#[derive(Debug)]
enum Command {
    RegisterProcessor {
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<bool>,
    },
    DeregisterProcessor {
        processor_id: ProcessorId,
    },
    PublishTrack {
        processor_id: ProcessorId,
        track_id: TrackId,
        reply_tx: tokio::sync::oneshot::Sender<Option<MessageSender>>,
    },
    SubscribeTrack {
        processor_id: ProcessorId,
        track_id: TrackId,
        tx: tokio::sync::mpsc::UnboundedSender<Message>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProcessorId(String);

impl ProcessorId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn get(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProcessorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackId(String);

impl TrackId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn get(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TrackId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Default)]
struct TrackState {
    publisher_command_tx: Option<tokio::sync::mpsc::UnboundedSender<TrackCommand>>,
}

#[derive(Debug)]
pub struct ProcessorHandle {
    pipeline_handle: MediaPipelineHandle,
    processor_id: ProcessorId,
}

impl ProcessorHandle {
    pub fn processor_id(&self) -> &ProcessorId {
        &self.processor_id
    }

    pub async fn publish_track(&self, track_id: TrackId) -> Option<MessageSender> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::PublishTrack {
            processor_id: self.processor_id.clone(),
            track_id,
            reply_tx,
        };
        self.pipeline_handle.send(command);
        reply_rx.await.ok()?
    }

    pub fn subscribe_track(&self, track_id: TrackId) -> MessageReceiver {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let command = Command::SubscribeTrack {
            processor_id: self.processor_id.clone(),
            track_id,
            tx,
        };
        self.pipeline_handle.send(command);

        // トラックが存在しなかったりした場合は、すぐに受信側が閉じるだけなので、
        // 上のコマンドの結果はまたない
        MessageReceiver { rx }
    }

    // TODO: これは実際に必要になったタイミングで実装する
    // （publish / susbscribe と同様に RPC 用のチャネルの作成を MediaPipeline に依頼するのが良さそう）
    //
    // pub async fn recv_rpc_request(&mut self) -> JsonRpcRequest {
    //    match self.rpc_rx.recv().await {
    //        Some(request) => request,
    //        None => std::future::pending().await,
    //    }
    // }
}

impl Drop for ProcessorHandle {
    fn drop(&mut self) {
        self.pipeline_handle.send(Command::DeregisterProcessor {
            processor_id: self.processor_id.clone(),
        });
    }
}

#[derive(Debug, Clone)]
pub struct Syn(#[expect(dead_code)] tokio::sync::mpsc::Sender<()>);

#[derive(Debug)]
pub struct Ack(tokio::sync::mpsc::Receiver<()>);

impl std::future::Future for Ack {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll_recv(cx).map(|_| ())
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Media(crate::MediaSample),
    Eos,

    /// 送信側がメッセージグラフの末端まで到達したか確認するための制御メッセージ。
    /// mpsc チャネルの受信側でクローズを確認することで、メッセージが完全に処理されたこと（= Ack を受け取った）を検知できる。
    Syn(Syn),
}

#[derive(Debug)]
enum TrackCommand {
    AddSubscriber(tokio::sync::mpsc::UnboundedSender<Message>),
}

#[derive(Debug)]
pub struct MessageSender {
    rx: tokio::sync::mpsc::UnboundedReceiver<TrackCommand>,
    txs: Vec<tokio::sync::mpsc::UnboundedSender<Message>>,
}

impl MessageSender {
    // MediaPipeline が途中終了した場合は false が返される
    pub fn send(&mut self, message: Message) -> bool {
        loop {
            match self.rx.try_recv() {
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    break;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.txs.clear();
                    return false;
                }
                Ok(TrackCommand::AddSubscriber(tx)) => {
                    self.txs.push(tx);
                }
            }
        }

        self.txs.retain_mut(|tx| tx.send(message.clone()).is_ok());
        true
    }

    pub fn send_media(&mut self, sample: crate::MediaSample) -> bool {
        self.send(Message::Media(sample))
    }

    pub fn send_audio(&mut self, data: crate::AudioData) -> bool {
        self.send(Message::Media(crate::MediaSample::new_audio(data)))
    }

    pub fn send_video(&mut self, frame: crate::VideoFrame) -> bool {
        self.send(Message::Media(crate::MediaSample::new_video(frame)))
    }

    pub fn send_eos(&mut self) -> bool {
        self.send(Message::Eos)
    }

    pub fn send_syn(&mut self) -> Ack {
        let (tx, rx) = tokio::sync::mpsc::channel(1); // NOTE: 0 だとエラーになる
        let _ = self.send(Message::Syn(Syn(tx))); // NOTE: ここでは false を特別扱いする必要はないので無視する
        Ack(rx)
    }
}

#[derive(Debug)]
pub struct MessageReceiver {
    rx: tokio::sync::mpsc::UnboundedReceiver<Message>,
}

impl MessageReceiver {
    pub async fn recv(&mut self) -> Message {
        if let Some(m) = self.rx.recv().await {
            m
        } else {
            // MediaPipeline::run() が何らかの理由で途中で終了した場合にはここに来る（EOS 扱いにする）
            Message::Eos
        }
    }
}
