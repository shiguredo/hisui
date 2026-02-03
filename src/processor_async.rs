#![expect(dead_code)]

#[derive(Debug)]
pub struct ProcessorManager {}

impl ProcessorManager {
    pub fn new() -> Self {
        Self {}
    }

    pub fn start(self) -> ProcessorManagerHandle {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let runner = ProcessorManagerRunner::new(command_tx.clone(), command_rx);
        tokio::task::spawn(runner.run());

        ProcessorManagerHandle { command_tx }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessorManagerHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<Command>,
}

impl ProcessorManagerHandle {
    // ID が衝突した場合は false が返される
    //
    // TODO: ここで統計構造体を登録できてもいいかも
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

    fn send(&self, command: Command) {
        let _ = self.command_tx.send(command);
    }
}

#[derive(Debug)]
struct TrackState {
    publisher: ProcessorId,
    subscribers: std::collections::HashSet<ProcessorId>,
    handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug)]
struct ProcessorManagerRunner {
    processors: std::collections::HashMap<ProcessorId, u64>, // value=seqno
    tracks: std::collections::HashMap<TrackId, TrackState>,
    handle: ProcessorManagerHandle,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    processor_seqno: u64,
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
        }
    }

    async fn run(mut self) {
        while let Some(command) = self.command_rx.recv().await {
            match command {
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
                    self.processors.remove(&processor_id);
                }
                Command::PublishTrack {
                    processor_id,
                    track_id,
                    reply_tx,
                } => {
                    todo!()
                }
            }
        }

        // 自分が command_tx の参照を保持しているので、ここに来ることはない
        unreachable!()
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
        if self.processors.contains_key(&processor_id) {
            return None;
        }

        let processor_seqno = self.processor_seqno;
        self.processor_seqno += 1;

        self.processors
            .insert(processor_id.clone(), processor_seqno);

        Some(ProcessorHandle {
            inner: self.handle.clone(),
            processor_id,
            processor_seqno,
        })
    }
}

#[derive(Debug)]
pub struct TrackPublishHandle {
    inner: ProcessorManagerHandle,
    processor_id: ProcessorId,
    track_id: TrackId,
}

#[derive(Debug)]
pub struct ProcessorHandle {
    inner: ProcessorManagerHandle,
    processor_id: ProcessorId,
    processor_seqno: u64, // TDOO: 不要かも
}

impl ProcessorHandle {
    pub fn processor_id(&self) -> &ProcessorId {
        &self.processor_id
    }

    pub async fn publish_track(&self, track_id: TrackId) -> Option<TrackPublishHandle> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::PublishTrack {
            processor_id: self.processor_id.clone(),
            track_id,
            reply_tx,
        };
        self.inner.send(command);
        reply_rx.await.unwrap_or(None)
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
    PublishTrack {
        processor_id: ProcessorId,
        track_id: TrackId,
        reply_tx: tokio::sync::oneshot::Sender<Option<TrackPublishHandle>>,
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
pub struct JsonRpcRequest(pub nojson::RawJsonOwned);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProcessorId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId;

#[derive(Debug)]
pub struct ChannelRegistry {}

impl ChannelRegistry {
    pub fn register(&mut self, _pid: ProcessorId) -> Option<(ChannelSender, ChannelReceiver)> {
        todo!()
    }

    // list_tracks(), list_processors()
}

#[derive(Debug)]
pub struct TrackInfo {}

#[derive(Debug)]
pub struct ChannelSender {}

impl ChannelSender {
    pub fn publish_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId, _info: TrackInfo) {}

    pub fn unpublish_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId) {}

    pub fn send_output(&mut self, _tid: TrackId, _frame: ()) {}

    pub fn send_feedback(&mut self, _tid: TrackId, _feedback: ()) {}
}

#[derive(Debug)]
pub struct SubscribeOptions {
    pub channel_size: usize,
}

#[derive(Debug)]
pub struct ChannelReceiver {}

impl ChannelReceiver {
    pub fn subscribe_track(
        &mut self,
        _reg: &ChannelRegistry,
        _tid: TrackId,
        _options: SubscribeOptions,
    ) {
    }

    pub fn subscribe_processor(
        &mut self,
        _reg: &ChannelRegistry,
        _pid: ProcessorId,
        _options: SubscribeOptions,
    ) {
    }

    pub fn unsubscribe_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId) {}

    pub fn unsubscribe_processor(&mut self, _reg: &ChannelRegistry, _pid: ProcessorId) {}

    pub fn recv(&mut self) -> Recv {
        todo!()
    }
}

#[derive(Debug)]
pub enum Recv {
    MediaFrame {
        pid: ProcessorId,
        tid: TrackId,
        data: (),
    },
    Feedback {
        pid: ProcessorId,
        tid: TrackId,
        data: Feedback,
    },
    Rpc {
        from: (),
        data: (),
    },
}

#[derive(Debug)]
pub enum Feedback {
    KeyFrameRequired,
}
