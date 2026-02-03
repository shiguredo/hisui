#![expect(dead_code)]

#[derive(Debug)]
pub struct ProcessorManager {
    // [NOTE]
    // 今は一つだけだが、将来的には特殊用途向け（e.g., CPU ヘビーなエンコード処理など）に
    // 専用のランタイムを追加する可能性があるため、それを意識した名前となっている
    default_runtime_handle: tokio::runtime::Handle,
}

impl ProcessorManager {
    pub fn new(default_runtime_handle: tokio::runtime::Handle) -> Self {
        Self {
            default_runtime_handle,
        }
    }

    pub fn start(self) -> ProcessorManagerHandle {
        let handle = self.default_runtime_handle.clone();

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let runner = ProcessorManagerRunner::new(command_tx.clone(), command_rx, handle.clone());
        handle.spawn(runner.run());

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
    pub async fn spawn_processor<F, Fut>(&self, processor_id: ProcessorId, f: F) -> bool
    where
        F: FnOnce(ProcessorHandle) -> Fut,
        Fut: Future<Output = Result<(), ProcessorError>> + Send + Unpin + 'static,
    {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let handle = ProcessorHandle {
            inner: self.clone(),
            processor_id: processor_id.clone(),
        };
        let command = Command::SpawnProcessor {
            processor_id,
            future: ProcessorFuture(Box::new(f(handle))),
            reply_tx,
        };
        self.send(command);
        reply_rx.await.unwrap_or(false)
    }

    fn send(&self, command: Command) {
        let _ = self.command_tx.send(command);
    }
}

#[derive(Debug)]
struct ProcessorManagerRunner {
    processors: std::collections::HashMap<ProcessorId, ()>,
    command_tx: tokio::sync::mpsc::UnboundedSender<Command>,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    default_runtime_handle: tokio::runtime::Handle,
}

impl ProcessorManagerRunner {
    fn new(
        command_tx: tokio::sync::mpsc::UnboundedSender<Command>,
        command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
        default_runtime_handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            processors: std::collections::HashMap::new(),
            command_tx,
            command_rx,
            default_runtime_handle,
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
                Command::NotifyProcessorFinish { processor_id } => {
                    self.processors.remove(&processor_id);
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

        let command_tx = self.command_tx.clone();
        self.default_runtime_handle.spawn(async move {
            if let Err(_e) = future.0.await {
                todo!("error handling");
            }
            let _ = command_tx.send(Command::NotifyProcessorFinish { processor_id });
        });

        true
    }
}

#[derive(Debug, Clone)]
pub struct ProcessorHandle {
    inner: ProcessorManagerHandle,
    processor_id: ProcessorId,
}

impl ProcessorHandle {
    pub fn processor_id(&self) -> &ProcessorId {
        &self.processor_id
    }
}

#[derive(Debug)]
enum Command {
    SpawnProcessor {
        processor_id: ProcessorId,
        future: ProcessorFuture,
        reply_tx: tokio::sync::oneshot::Sender<bool>,
    },
    NotifyProcessorFinish {
        processor_id: ProcessorId,
    },
    /*PublishTrack {
        track_id: TrackId,
    },
    UnpublishTrack {
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

#[derive(Debug)]
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
