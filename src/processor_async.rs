#![expect(dead_code)]
#![expect(clippy::new_without_default)]

use crate::media::MediaSample;

#[derive(Debug)]
pub struct ProcessorManager {}

impl ProcessorManager {
    pub fn new() -> Self {
        Self {}
    }

    // TODO: start 前に processor を登録できるようにする (空になったら終了したいので）

    pub fn start(self) -> ProcessorManagerHandle {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let runner = ProcessorManagerRunner::new(command_tx.clone(), command_rx);
        tokio::task::spawn(runner.run());

        ProcessorManagerHandle { command_tx }
    }
}

// [NOTE] Arc<Foo<Notify>> みたいな構造体を追加して drop() の中で通知を行うので実現できそう
#[derive(Debug, Clone)]
pub struct Syn(tokio::sync::mpsc::Sender<()>);

#[derive(Debug)]
pub struct Ack(tokio::sync::mpsc::Receiver<()>);

impl Ack {
    pub fn is_acked(&self) -> bool {
        self.0.is_closed()
    }
}

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
    Media(MediaSample),
    Eos,

    /// 送信側がメッセージグラフの末端まで到達したか確認するための制御メッセージ。
    /// mpsc チャネルの受信側でクローズを確認することで、メッセージが完全に処理されたこと（= Ack を受け取った）を検知できる。
    Syn(Syn),
}

impl Message {
    pub fn audio(data: crate::audio::AudioData) -> Self {
        Self::Media(MediaSample::new_audio(data))
    }

    pub fn video(frame: crate::video::VideoFrame) -> Self {
        Self::Media(MediaSample::new_video(frame))
    }
}

#[derive(Debug, Clone)]
pub struct ProcessorManagerHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<Command>,
}

impl ProcessorManagerHandle {
    // ID が衝突した場合は false が返される
    //
    // TODO: remove
    pub async fn spawn_processor<F>(&self, processor_id: ProcessorId, future: F) -> bool
    where
        F: Future<Output = Result<(), ProcessorError>> + Send + Unpin + 'static,
    {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::SpawnProcessor {
            processor_id,
            future: ProcessorFuture(Box::new(future)),
            reply_tx,
        };
        self.send(command);
        reply_rx.await.unwrap_or(false)
    }

    pub async fn register_processor(&self, processor_id: ProcessorId) -> Option<ProcessorHandle> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::RegisterProcessor {
            processor_id,
            reply_tx,
        };
        self.send(command);
        reply_rx.await.unwrap_or(None)
    }

    pub async fn wait_finish(&self) {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::WaitFinish { reply_tx };
        self.send(command);
        let _ = reply_rx.await;
    }

    fn send(&self, command: Command) {
        let _ = self.command_tx.send(command);
    }
}

#[derive(Debug)]
struct TrackPublisherState {
    incoming_rx: tokio::sync::mpsc::UnboundedReceiver<Message>,
}

#[derive(Debug)]
struct TrackSubscriberState {
    outgoing_tx: tokio::sync::mpsc::UnboundedSender<Message>,
}

#[derive(Debug)]
enum TrackCommand {
    Publish {
        publisher_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<Option<TrackPublishHandle>>,
    },
    Subscribe {
        // TODO: size_limit
        reply_tx: tokio::sync::oneshot::Sender<TrackSubscribeHandle>,
    },
}

#[derive(Debug)]
pub struct TrackSubscribeHandle {
    outgoing_rx: tokio::sync::mpsc::UnboundedReceiver<Message>,
}

impl TrackSubscribeHandle {
    // TODO: rename
    pub async fn recv_media(&mut self) -> Message {
        match self.outgoing_rx.recv().await {
            Some(m) => m,
            None => std::future::pending().await,
        }
    }
}

#[derive(Debug)]
struct TrackRunner {
    track_id: TrackId,
    handle: ProcessorManagerHandle,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<TrackCommand>,
    publisher: Option<TrackPublisherState>,
    subscribers: Vec<TrackSubscriberState>,
}

impl TrackRunner {
    fn new(track_id: TrackId, handle: ProcessorManagerHandle) -> (Self, TrackHandle) {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                track_id,
                handle,
                command_rx,
                publisher: None,
                subscribers: Vec::new(),
            },
            TrackHandle { command_tx },
        )
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                Some(command) = self.command_rx.recv() => self.handle_command(command),
                sample = async {
                    match &mut self.publisher {
                        Some(publisher) => publisher.incoming_rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.handle_sample(sample).await;
                }
                else => break,
            }
        }
    }

    async fn handle_sample(&mut self, sample: Option<Message>) {
        let Some(sample) = sample else {
            self.publisher = None;
            return;
        };

        let mut i = 0;
        while i < self.subscribers.len() {
            match self.subscribers[i].outgoing_tx.send(sample.clone()) {
                Ok(()) => {}
                Err(_) => {
                    self.subscribers.swap_remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }

    fn handle_command(&mut self, command: TrackCommand) {
        match command {
            TrackCommand::Publish {
                publisher_id,
                reply_tx,
            } => {
                let result = self.handle_publish(publisher_id);
                let _ = reply_tx.send(result);
            }
            TrackCommand::Subscribe { reply_tx } => {
                let result = self.handle_subscribe();
                let _ = reply_tx.send(result);
            } // TODO: subscriber 側は Unsubscribe も必要（必須ではないけど publish がないと残り続けてしまう）
        }
    }

    fn handle_subscribe(&mut self) -> TrackSubscribeHandle {
        let (outgoing_tx, outgoing_rx) = tokio::sync::mpsc::unbounded_channel();

        let subscriber_state = TrackSubscriberState { outgoing_tx };
        self.subscribers.push(subscriber_state);

        TrackSubscribeHandle { outgoing_rx }
    }

    fn handle_publish(&mut self, _publisher_id: ProcessorId) -> Option<TrackPublishHandle> {
        if self.publisher.is_some() {
            return None;
        }

        let (incoming_tx, incoming_rx) = tokio::sync::mpsc::unbounded_channel();
        self.publisher = Some(TrackPublisherState { incoming_rx });

        let handle = TrackPublishHandle { incoming_tx };
        Some(handle)
    }
}

impl Drop for TrackRunner {
    fn drop(&mut self) {
        self.handle.send(Command::RemoveTrack {
            track_id: self.track_id.clone(),
        });
    }
}

#[derive(Debug, Clone)]
struct TrackHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<TrackCommand>,
}

impl TrackHandle {
    async fn publish(&self, publisher_id: ProcessorId) -> Option<TrackPublishHandle> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let _ = self.command_tx.send(TrackCommand::Publish {
            publisher_id,
            reply_tx,
        });
        reply_rx.await.unwrap_or(None)
    }

    async fn subscribe(&self) -> TrackSubscribeHandle {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let _ = self.command_tx.send(TrackCommand::Subscribe { reply_tx });
        reply_rx.await.expect("bug")
    }
}

// TODO: remove
#[derive(Debug)]
struct TrackState {
    publisher: Option<ProcessorId>,
    subscribers: std::collections::HashSet<ProcessorId>,
    handle: TrackHandle,
}

#[derive(Debug)]
struct ProcessorManagerRunner {
    processors: std::collections::HashMap<ProcessorId, u64>, // value=seqno
    tracks: std::collections::HashMap<TrackId, TrackHandle>,
    handle: ProcessorManagerHandle,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    processor_seqno: u64,
    finish_waitings: Vec<tokio::sync::oneshot::Sender<()>>,
}

impl ProcessorManagerRunner {
    fn new(
        command_tx: tokio::sync::mpsc::UnboundedSender<Command>,
        command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    ) -> Self {
        Self {
            processors: std::collections::HashMap::new(),
            tracks: std::collections::HashMap::new(),
            handle: ProcessorManagerHandle { command_tx },
            command_rx,
            processor_seqno: 0,
            finish_waitings: Vec::new(),
        }
    }

    async fn run(mut self) {
        while let Some(command) = self.command_rx.recv().await {
            match command {
                Command::WaitFinish { reply_tx } => {
                    self.finish_waitings.push(reply_tx);
                }
                Command::SpawnProcessor {
                    processor_id,
                    future,
                    reply_tx,
                } => {
                    let result = self.handle_spawn_processor(processor_id, future);
                    let _ = reply_tx.send(result);
                }
                Command::RegisterProcessor {
                    processor_id,
                    reply_tx,
                } => {
                    let result = self.handle_register_processor(processor_id);
                    let _ = reply_tx.send(result);
                }
                Command::DeregisterProcessor { processor_id } => {
                    log::debug!("deregister processor: {}", processor_id.get());
                    self.processors.remove(&processor_id);

                    // TODO: 判定場所は変える
                    if self.processors.is_empty() {
                        for waiting in self.finish_waitings {
                            let _ = waiting.send(());
                        }
                        log::debug!("finish processor manager");
                        return;
                    }
                }
                Command::CreateTrack {
                    processor_id,
                    track_id,
                    reply_tx,
                } => {
                    let result = self.handle_create_track(processor_id, track_id);
                    let _ = reply_tx.send(result);
                }
                Command::RemoveTrack { track_id } => {
                    self.tracks.remove(&track_id);
                }
            }
        }

        // 自分が command_tx の参照を保持しているので、ここに来ることはない
        unreachable!()
    }

    fn handle_create_track(&mut self, processor_id: ProcessorId, track_id: TrackId) -> TrackHandle {
        assert!(self.processors.contains_key(&processor_id));

        let track_handle = self.tracks.entry(track_id.clone()).or_insert_with(|| {
            let (runner, handle) = TrackRunner::new(track_id.clone(), self.handle.clone());
            tokio::task::spawn(runner.run());
            handle
        });

        track_handle.clone()
    }

    fn handle_spawn_processor(
        &mut self,
        processor_id: ProcessorId,
        future: ProcessorFuture,
    ) -> bool {
        if self.processors.contains_key(&processor_id) {
            return false;
        }

        tokio::task::spawn(async move {
            if let Err(_e) = future.0.await {
                todo!("error handling");
            }
        });

        true
    }

    fn handle_register_processor(&mut self, processor_id: ProcessorId) -> Option<ProcessorHandle> {
        log::debug!("register processor: {}", processor_id.get());
        if self.processors.contains_key(&processor_id) {
            return None;
        }

        let processor_seqno = self.processor_seqno;
        self.processor_seqno += 1;

        self.processors
            .insert(processor_id.clone(), processor_seqno);

        let (rpc_tx, rpc_rx) = tokio::sync::mpsc::channel(10); // TODO: これは共通設定でいいけど、最初に変更できるようにはする
        let _ = rpc_tx; // ProcessorState を追加して、外から ID => rpc sender を解決できるようにする

        Some(ProcessorHandle {
            inner: self.handle.clone(),
            processor_id,
            processor_seqno,
            rpc_rx,
        })
    }
}

// TODO: clone 可能なやつと無理なやつで両方 "XxxHandle" という命名にしているのは紛らわしいのでどちらかを変えたい
// こっちは "er" 形式にしてしまう？
#[derive(Debug)]
pub struct TrackPublishHandle {
    incoming_tx: tokio::sync::mpsc::UnboundedSender<Message>,
}

impl TrackPublishHandle {
    pub fn send_media(&self, sample: MediaSample) {
        let _ = self.incoming_tx.send(Message::Media(sample));
    }

    pub fn send_eos(&self) {
        let _ = self.incoming_tx.send(Message::Eos);
    }

    pub fn send_syn(&self) -> Ack {
        let (tx, rx) = tokio::sync::mpsc::channel(1); // NOTE: 0 だとエラーになる
        let _ = self.incoming_tx.send(Message::Syn(Syn(tx)));
        Ack(rx)
    }
}

// TODO: clone 不可なものは名前を変える
#[derive(Debug)]
pub struct ProcessorHandle {
    inner: ProcessorManagerHandle,
    processor_id: ProcessorId,
    processor_seqno: u64, // TDOO: 不要かも
    // TODO: add rx for RPC requests
    rpc_rx: tokio::sync::mpsc::Receiver<JsonRpcRequest>,
}

impl ProcessorHandle {
    pub fn processor_id(&self) -> &ProcessorId {
        &self.processor_id
    }

    pub async fn publish_track(&self, track_id: TrackId) -> Option<TrackPublishHandle> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::CreateTrack {
            processor_id: self.processor_id.clone(),
            track_id,
            reply_tx,
        };
        self.inner.send(command);
        let track_handle = reply_rx.await.ok()?;

        track_handle.publish(self.processor_id.clone()).await
    }

    pub async fn subscribe_track(&self, track_id: TrackId) -> TrackSubscribeHandle {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::CreateTrack {
            processor_id: self.processor_id.clone(),
            track_id,
            reply_tx,
        };
        self.inner.send(command);
        let track_handle = reply_rx.await.expect("bug");

        track_handle.subscribe().await
    }

    pub async fn recv_rpc_request(&mut self) -> JsonRpcRequest {
        match self.rpc_rx.recv().await {
            Some(request) => request,
            None => std::future::pending().await,
        }
    }
}

impl Drop for ProcessorHandle {
    fn drop(&mut self) {
        self.inner.send(Command::DeregisterProcessor {
            processor_id: self.processor_id.clone(),
        });
    }
}

#[derive(Debug)]
enum Command {
    WaitFinish {
        reply_tx: tokio::sync::oneshot::Sender<()>,
    },
    SpawnProcessor {
        processor_id: ProcessorId,
        future: ProcessorFuture,
        reply_tx: tokio::sync::oneshot::Sender<bool>,
    },
    RegisterProcessor {
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<Option<ProcessorHandle>>,
    },
    DeregisterProcessor {
        processor_id: ProcessorId,
        // processor_seqno: u64,
    },
    CreateTrack {
        processor_id: ProcessorId,
        track_id: TrackId,
        reply_tx: tokio::sync::oneshot::Sender<TrackHandle>,
    },
    RemoveTrack {
        track_id: TrackId,
    },
    /*UnpublishTrack {
        track_id: TrackId,
    },*/
}

pub struct ProcessorFuture(
    Box<dyn Future<Output = Result<(), ProcessorError>> + Send + Unpin + 'static>,
);

impl std::fmt::Debug for ProcessorFuture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessorFuture").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct ProcessorError;

#[derive(Debug)]
pub struct JsonRpcRequest(pub nojson::RawJsonOwned); // TODO: reply_tx を追加する

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProcessorId(String);

impl ProcessorId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn get(&self) -> &str {
        &self.0
    }
}

// TODO: この中に processor id を含んでいても良さそう
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(String);

impl TrackId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn get(&self) -> &str {
        &self.0
    }
}
