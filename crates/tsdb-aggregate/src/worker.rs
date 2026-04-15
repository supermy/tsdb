use tsdb_types::model::DataPoint;
use crate::aggregator::{Aggregator, AggregateSpec};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

pub enum AggregateMessage {
    DataPoint(DataPoint),
    Shutdown,
}

pub struct AggregateWorker {
    receiver: Receiver<AggregateMessage>,
    buffer: Vec<DataPoint>,
    buffer_size: usize,
    specs: Vec<AggregateSpec>,
}

impl AggregateWorker {
    pub fn new(
        receiver: Receiver<AggregateMessage>,
        buffer_size: usize,
        specs: Vec<AggregateSpec>,
    ) -> Self {
        Self {
            receiver,
            buffer: Vec::with_capacity(buffer_size),
            buffer_size,
            specs,
        }
    }

    pub fn run(mut self) {
        loop {
            match self.receiver.recv() {
                Ok(AggregateMessage::DataPoint(dp)) => {
                    self.buffer.push(dp);
                    if self.buffer.len() >= self.buffer_size {
                        self.flush();
                    }
                }
                Ok(AggregateMessage::Shutdown) => {
                    self.flush();
                    break;
                }
                Err(_) => {
                    self.flush();
                    break;
                }
            }
        }
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        for spec in &self.specs {
            let results = Aggregator::aggregate(&self.buffer, spec);
            for result in results {
                tracing::debug!(
                    "aggregate result: measurement={} bucket={} value={:?}",
                    result.measurement,
                    result.time_bucket,
                    result.value,
                );
            }
        }

        self.buffer.clear();
    }
}

pub struct AggregateDispatcher {
    senders: Vec<Sender<AggregateMessage>>,
    next_worker: usize,
}

impl AggregateDispatcher {
    pub fn new(worker_count: usize, buffer_size: usize, specs: Vec<AggregateSpec>) -> Self {
        let mut senders = Vec::new();

        for _ in 0..worker_count {
            let (tx, rx) = mpsc::channel();
            let worker_specs = specs.clone();
            thread::spawn(move || {
                let worker = AggregateWorker::new(rx, buffer_size, worker_specs);
                worker.run();
            });
            senders.push(tx);
        }

        Self {
            senders,
            next_worker: 0,
        }
    }

    pub fn dispatch(&mut self, dp: DataPoint) {
        if self.senders.is_empty() {
            return;
        }
        let msg = AggregateMessage::DataPoint(dp);
        let _ = self.senders[self.next_worker].send(msg);
        self.next_worker = (self.next_worker + 1) % self.senders.len();
    }

    pub fn shutdown(&mut self) {
        for sender in &self.senders {
            let _ = sender.send(AggregateMessage::Shutdown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregator::{TimeDimension, AggFunc, AggregateSpec};
    use tsdb_types::model::{DataPoint, FieldValue};

    #[test]
    fn test_worker_flush() {
        let (tx, rx) = mpsc::channel();
        let specs = vec![AggregateSpec {
            time_dimension: TimeDimension::Hour,
            field_name: "cpu".to_string(),
            func: AggFunc::Avg,
        }];

        let mut worker = AggregateWorker::new(rx, 3, specs);

        let dp = DataPoint::new("cpu", 1_000_000_000)
            .with_field("cpu", FieldValue::Float(0.5));
        tx.send(AggregateMessage::DataPoint(dp)).unwrap();

        let dp2 = DataPoint::new("cpu", 1_800_000_000)
            .with_field("cpu", FieldValue::Float(0.7));
        tx.send(AggregateMessage::DataPoint(dp2)).unwrap();

        let dp3 = DataPoint::new("cpu", 2_500_000_000)
            .with_field("cpu", FieldValue::Float(0.9));
        tx.send(AggregateMessage::DataPoint(dp3)).unwrap();

        tx.send(AggregateMessage::Shutdown).unwrap();
        worker.run();
    }
}
