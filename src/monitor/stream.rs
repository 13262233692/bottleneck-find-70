use crate::models::{JaegerTrace, Process, Reference, Span, Tag, TraceData};
use anyhow::Result;
use rand::{Rng, rngs::StdRng, SeedableRng};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::monitor::MonitorConfig;

#[derive(Debug, Clone)]
pub struct TraceStreamMessage {
    pub trace: JaegerTrace,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub source: TraceSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSource {
    JaegerGrpc,
    Simulated,
    File,
}

pub struct TraceStream {
    config: Arc<MonitorConfig>,
    sender: mpsc::Sender<TraceStreamMessage>,
    receiver: Option<mpsc::Receiver<TraceStreamMessage>>,
}

impl TraceStream {
    pub fn new(config: Arc<MonitorConfig>) -> Self {
        let (sender, receiver) = mpsc::channel(1000);
        Self {
            config,
            sender,
            receiver: Some(receiver),
        }
    }

    pub fn sender(&self) -> mpsc::Sender<TraceStreamMessage> {
        self.sender.clone()
    }

    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<TraceStreamMessage>> {
        self.receiver.take()
    }

    pub async fn start_streaming(&self) -> Result<()> {
        if self.config.enable_simulated_stream {
            self.start_simulated_stream().await?;
        }
        Ok(())
    }

    async fn start_simulated_stream(&self) -> Result<()> {
        let sender = self.sender.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            simulate_trace_stream(sender, config).await;
        });

        Ok(())
    }
}

async fn simulate_trace_stream(sender: mpsc::Sender<TraceStreamMessage>, config: Arc<MonitorConfig>) {
    let service_names: Vec<String> = (0..config.simulated_services)
        .map(|i| format!("service-{:02}", i + 1))
        .collect();

    let mut rng = StdRng::from_entropy();
    let mut trace_counter = 0u64;

    let base_delay = (1000.0 / config.simulated_traces_per_second) as u64;

    loop {
        trace_counter += 1;

        let bottleneck_idx = (trace_counter / 20) as usize % service_names.len();
        let has_spike = trace_counter % 100 == 0;

        let trace = generate_simulated_trace(
            trace_counter,
            &service_names,
            bottleneck_idx,
            has_spike,
            &mut rng,
        );

        let message = TraceStreamMessage {
            trace,
            received_at: chrono::Utc::now(),
            source: TraceSource::Simulated,
        };

        if let Err(e) = sender.send(message).await {
            eprintln!("[Simulated Stream] 发送追踪数据失败: {}", e);
            break;
        }

        let jitter = rng.gen_range(0..=base_delay / 2);
        tokio::time::sleep(tokio::time::Duration::from_millis(base_delay + jitter)).await;
    }
}

fn generate_simulated_trace(
    trace_id: u64,
    services: &[String],
    bottleneck_idx: usize,
    has_spike: bool,
    rng: &mut impl Rng,
) -> JaegerTrace {
    let trace_id_hex = format!("{:032x}", trace_id);

    let mut processes: HashMap<String, Process> = HashMap::new();
    for (i, service) in services.iter().enumerate() {
        processes.insert(
            format!("p{}", i),
            Process {
                service_name: service.clone(),
                tags: vec![],
            },
        );
    }

    let mut spans = Vec::new();
    let root_span_id = format!("{:016x}", trace_id * 1000);
    let bottleneck_service = &services[bottleneck_idx];

    let base_duration = if has_spike { 3000000 } else { 500000 };

    let root_span = Span {
        trace_id: trace_id_hex.clone(),
        span_id: root_span_id.clone(),
        operation: "handle_request".to_string(),
        references: vec![],
        start_time: chrono::Utc::now().timestamp_micros() as u64,
        duration: base_duration * 3,
        process_id: "p0".to_string(),
        tags: vec![],
    };
    spans.push(root_span);

    let mut prev_span_id = root_span_id.clone();
    let mut current_time = chrono::Utc::now().timestamp_micros() as u64 + 1000;

    for (i, service) in services.iter().enumerate().skip(1) {
        let span_id = format!("{:016x}", trace_id * 1000 + i as u64);

        let is_bottleneck = service == bottleneck_service;
        let duration = if is_bottleneck {
            if has_spike {
                base_duration * 4
            } else {
                base_duration * 2
            }
        } else {
            base_duration / 2 + rng.gen_range(0..=base_duration / 4)
        };

        let span = Span {
            trace_id: trace_id_hex.clone(),
            span_id: span_id.clone(),
            operation: format!("call_{}", service),
            references: vec![Reference {
                ref_type: "CHILD_OF".to_string(),
                trace_id: trace_id_hex.clone(),
                span_id: prev_span_id.clone(),
            }],
            start_time: current_time,
            duration,
            process_id: format!("p{}", i),
            tags: vec![Tag {
                key: "http.method".to_string(),
                tag_type: "string".to_string(),
                value: serde_json::Value::String("GET".to_string()),
            }],
        };

        spans.push(span);
        prev_span_id = span_id;
        current_time += duration + rng.gen_range(0..=1000);
    }

    let trace_data = TraceData {
        trace_id: trace_id_hex,
        spans,
        processes,
    };

    JaegerTrace {
        data: vec![trace_data],
    }
}

pub struct TraceBatch {
    pub traces: Vec<JaegerTrace>,
    pub window_start: chrono::DateTime<chrono::Utc>,
    pub window_end: chrono::DateTime<chrono::Utc>,
}

impl TraceBatch {
    pub fn new() -> Self {
        Self {
            traces: Vec::new(),
            window_start: chrono::Utc::now(),
            window_end: chrono::Utc::now(),
        }
    }

    pub fn add(&mut self, message: TraceStreamMessage) {
        self.traces.push(message.trace);
        self.window_end = message.received_at;
        if self.traces.len() == 1 {
            self.window_start = message.received_at;
        }
    }

    pub fn len(&self) -> usize {
        self.traces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.traces.is_empty()
    }

    pub fn clear(&mut self) {
        self.traces.clear();
        self.window_start = chrono::Utc::now();
        self.window_end = chrono::Utc::now();
    }
}

impl Default for TraceBatch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_batch_operations() {
        let mut batch = TraceBatch::new();
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);

        batch.add(TraceStreamMessage {
            trace: JaegerTrace {
                data: vec![],
            },
            received_at: chrono::Utc::now(),
            source: TraceSource::Simulated,
        });

        assert!(!batch.is_empty());
        assert_eq!(batch.len(), 1);

        batch.clear();
        assert!(batch.is_empty());
    }

    #[test]
    fn test_trace_source_display() {
        assert_eq!(format!("{:?}", TraceSource::Simulated), "Simulated");
        assert_eq!(format!("{:?}", TraceSource::JaegerGrpc), "JaegerGrpc");
    }

    #[tokio::test]
    async fn test_trace_stream_creation() {
        let config = Arc::new(MonitorConfig::default());
        let mut stream = TraceStream::new(config);
        
        let _sender = stream.sender();
        let receiver = stream.take_receiver();
        
        assert!(receiver.is_some());
    }
}
