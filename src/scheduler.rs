use crate::processor::{BoxedMediaProcessor, MediaProcessor};

#[derive(Debug)]
pub struct SchedulerBuilder {
    processors: Vec<BoxedMediaProcessor>,
}

impl SchedulerBuilder {
    pub fn new() -> Self {
        Self {
            processors: Vec::new(),
        }
    }

    pub fn register<P>(&mut self, processor: P)
    where
        P: 'static + Send + MediaProcessor,
    {
        self.processors.push(BoxedMediaProcessor::new(processor));
    }
}

#[derive(Debug)]
pub struct Scheduler {}

impl Scheduler {}
