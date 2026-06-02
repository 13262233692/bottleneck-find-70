use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JaegerTrace {
    pub data: Vec<TraceData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceData {
    #[serde(rename = "traceID")]
    pub trace_id: String,
    pub spans: Vec<Span>,
    pub processes: HashMap<String, Process>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    #[serde(rename = "traceID")]
    pub trace_id: String,
    #[serde(rename = "spanID")]
    pub span_id: String,
    pub operation: String,
    #[serde(rename = "startTime")]
    pub start_time: u64,
    pub duration: u64,
    pub references: Vec<Reference>,
    #[serde(rename = "processID")]
    pub process_id: String,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    #[serde(rename = "refType")]
    pub ref_type: String,
    #[serde(rename = "traceID")]
    pub trace_id: String,
    #[serde(rename = "spanID")]
    pub span_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Process {
    #[serde(rename = "serviceName")]
    pub service_name: String,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub key: String,
    #[serde(rename = "type")]
    pub tag_type: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ServiceCall {
    pub from_service: String,
    pub to_service: String,
    pub duration: u64,
    pub trace_id: String,
    pub span_id: String,
    pub operation: String,
    pub start_time: u64,
    pub is_async: bool,
    pub queue_latency: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceAnalysis {
    pub trace_id: String,
    pub total_duration: u64,
    pub services: Vec<String>,
    pub critical_path: Vec<CriticalPathNode>,
    pub bottleneck_scores: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CriticalPathNode {
    pub service_name: String,
    pub span_id: String,
    pub operation: String,
    pub duration: u64,
    pub start_time: u64,
    pub cumulative_duration: u64,
    pub is_bottleneck: bool,
    pub is_async: bool,
    pub async_propagation_delay: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceBottleneckRank {
    pub service_name: String,
    pub total_contribution: u64,
    pub appearance_count: usize,
    pub avg_contribution: f64,
    pub bottleneck_score: f64,
    pub critical_path_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisReport {
    pub total_traces: usize,
    pub total_services: usize,
    pub service_ranks: Vec<ServiceBottleneckRank>,
    pub trace_analyses: Vec<TraceAnalysis>,
    pub dependency_graph: Vec<DependencyEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
    pub call_count: usize,
    pub avg_duration: f64,
    pub total_duration: u64,
    pub is_async: bool,
    pub avg_queue_latency: Option<f64>,
}
