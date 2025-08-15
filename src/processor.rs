use crate::stats::ProcessorStats;

pub trait MediaProcessor {
    fn process_input(&mut self);
    fn generate_output(&mut self);
    fn stats(&self) -> ProcessorStats;
}
