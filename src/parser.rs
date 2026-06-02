use crate::models::{JaegerTrace, Process, ServiceCall, Span, TraceData};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn parse_jaeger_json<P: AsRef<Path>>(path: P) -> Result<JaegerTrace> {
    let content = fs::read_to_string(path.as_ref())
        .with_context(|| format!("Failed to read file: {:?}", path.as_ref()))?;
    
    let trace: JaegerTrace = serde_json::from_str(&content)
        .with_context(|| "Failed to parse Jaeger JSON")?;
    
    Ok(trace)
}

pub fn parse_traces(trace: &JaegerTrace) -> Result<Vec<TraceData>> {
    Ok(trace.data.clone())
}

pub fn build_service_calls(trace_data: &TraceData) -> Result<Vec<ServiceCall>> {
    let mut service_calls = Vec::new();
    
    let span_map: HashMap<&String, &Span> = trace_data.spans
        .iter()
        .map(|span| (&span.span_id, span))
        .collect();
    
    let process_map: &HashMap<String, Process> = &trace_data.processes;
    
    for span in &trace_data.spans {
        let to_process = process_map.get(&span.process_id)
            .with_context(|| format!("Process not found for span: {}", span.span_id))?;
        let to_service = &to_process.service_name;
        
        for reference in &span.references {
            let is_async = reference.ref_type == "FOLLOWS_FROM";
            
            if reference.ref_type == "CHILD_OF" || is_async {
                if let Some(parent_span) = span_map.get(&reference.span_id) {
                    let from_process = process_map.get(&parent_span.process_id)
                        .with_context(|| format!("Process not found for parent span: {}", parent_span.span_id))?;
                    let from_service = &from_process.service_name;
                    
                    if from_service != to_service {
                        let queue_latency = if is_async {
                            Some(span.start_time.saturating_sub(parent_span.start_time + parent_span.duration))
                        } else {
                            None
                        };
                        
                        service_calls.push(ServiceCall {
                            from_service: from_service.clone(),
                            to_service: to_service.clone(),
                            duration: span.duration,
                            trace_id: trace_data.trace_id.clone(),
                            span_id: span.span_id.clone(),
                            operation: span.operation.clone(),
                            start_time: span.start_time,
                            is_async,
                            queue_latency,
                        });
                    }
                }
            }
        }
    }
    
    Ok(service_calls)
}

pub fn get_span_service<'a>(span: &Span, process_map: &'a HashMap<String, Process>) -> Option<&'a str> {
    process_map.get(&span.process_id).map(|p| p.service_name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_jaeger_json() -> String {
        r#"{
            "data": [{
                "traceID": "test-trace-1",
                "spans": [
                    {
                        "traceID": "test-trace-1",
                        "spanID": "span-1",
                        "operation": "http_get",
                        "startTime": 1000,
                        "duration": 5000,
                        "references": [],
                        "processID": "p1",
                        "tags": []
                    },
                    {
                        "traceID": "test-trace-1",
                        "spanID": "span-2",
                        "operation": "db_query",
                        "startTime": 1500,
                        "duration": 3000,
                        "references": [{"refType": "CHILD_OF", "traceID": "test-trace-1", "spanID": "span-1"}],
                        "processID": "p2",
                        "tags": []
                    },
                    {
                        "traceID": "test-trace-1",
                        "spanID": "span-3",
                        "operation": "cache_get",
                        "startTime": 2000,
                        "duration": 1000,
                        "references": [{"refType": "CHILD_OF", "traceID": "test-trace-1", "spanID": "span-2"}],
                        "processID": "p3",
                        "tags": []
                    }
                ],
                "processes": {
                    "p1": {"serviceName": "api-gateway", "tags": []},
                    "p2": {"serviceName": "order-service", "tags": []},
                    "p3": {"serviceName": "redis-cache", "tags": []}
                }
            }]
        }"#.to_string()
    }

    #[test]
    fn test_parse_jaeger_json() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "{}", create_test_jaeger_json()).unwrap();
        let path = temp_file.path();
        
        let result = parse_jaeger_json(path);
        assert!(result.is_ok());
        
        let trace = result.unwrap();
        assert_eq!(trace.data.len(), 1);
        assert_eq!(trace.data[0].trace_id, "test-trace-1");
        assert_eq!(trace.data[0].spans.len(), 3);
    }

    #[test]
    fn test_build_service_calls() {
        let trace_data: JaegerTrace = serde_json::from_str(&create_test_jaeger_json()).unwrap();
        let calls = build_service_calls(&trace_data.data[0]).unwrap();
        
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].from_service, "api-gateway");
        assert_eq!(calls[0].to_service, "order-service");
        assert_eq!(calls[1].from_service, "order-service");
        assert_eq!(calls[1].to_service, "redis-cache");
    }
}
