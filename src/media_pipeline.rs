#[derive(Debug)]
pub struct MediaPipeline {
    command_tx: Option<tokio::sync::mpsc::UnboundedSender<Command>>,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    processors: std::collections::HashSet<ProcessorId>,
}

impl MediaPipeline {
    pub fn new() -> Self {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            command_tx: Some(command_tx),
            command_rx,
            processors: std::collections::HashSet::new(),
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
        }
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
        let _ = self.command_tx.send(command);
        if let Ok(true) = reply_rx.await {
            Some(ProcessorHandle {
                pipeline_handle: self.clone(),
                processor_id,
            })
        } else {
            None
        }
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

#[derive(Debug)]
pub struct ProcessorHandle {
    pipeline_handle: MediaPipelineHandle,
    processor_id: ProcessorId,
}

impl ProcessorHandle {
    pub fn processor_id(&self) -> &ProcessorId {
        &self.processor_id
    }

    /*pub async fn publish_track(&self, track_id: TrackId) -> Option<TrackPublishHandle> {
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
    }*/
}

impl Drop for ProcessorHandle {
    fn drop(&mut self) {
        let _ = self
            .pipeline_handle
            .command_tx
            .send(Command::DeregisterProcessor {
                processor_id: self.processor_id.clone(),
            });
    }
}
