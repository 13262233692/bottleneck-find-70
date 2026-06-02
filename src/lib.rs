pub mod models;
pub mod parser;
pub mod dependency_graph;
pub mod critical_path;
pub mod ranking;
pub mod output;
pub mod monitor;

pub use models::*;
pub use parser::*;
pub use dependency_graph::*;
pub use critical_path::*;
pub use ranking::*;
pub use output::*;
pub use monitor::*;

use anyhow::Result;
use std::path::Path;

pub fn run_analysis(
    input_path: &Path,
    output_format: OutputFormat,
    output_path: Option<&Path>,
) -> Result<()> {
    let trace = parse_jaeger_json(input_path)?;
    let report = generate_report(&trace)?;
    generate_output(&report, output_format, output_path)?;
    Ok(())
}

pub fn generate_sample_data(num_traces: usize, num_services: usize) -> JaegerTrace {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    let services: Vec<String> = (0..num_services)
        .map(|i| format!("service-{}", i))
        .collect();

    let mut traces_data = Vec::new();

    for trace_idx in 0..num_traces {
        let trace_id = format!("trace-{:04}", trace_idx);
        let num_spans = rng.gen_range(3..10);
        let mut spans = Vec::new();
        let mut processes = std::collections::HashMap::new();

        for (span_idx, service_idx) in (0..num_spans).enumerate() {
            let process_id = format!("p{}", service_idx % num_services);
            let service_name = services[service_idx % num_services].clone();
            
            processes.insert(
                process_id.clone(),
                Process {
                    service_name,
                    tags: vec![],
                },
            );

            let span_id = format!("span-{}-{}", trace_idx, span_idx);
            let start_time = (1000000 + trace_idx * 1000000 + span_idx * 10000) as u64;
            let duration = rng.gen_range(100..5000);

            let mut references = Vec::new();
            if span_idx > 0 {
                references.push(Reference {
                    ref_type: "CHILD_OF".to_string(),
                    trace_id: trace_id.clone(),
                    span_id: format!("span-{}-{}", trace_idx, span_idx - 1),
                });
            }

            spans.push(Span {
                trace_id: trace_id.clone(),
                span_id,
                operation: format!("operation-{}", rng.gen_range(0..5)),
                start_time,
                duration,
                references,
                process_id,
                tags: vec![],
            });
        }

        traces_data.push(TraceData {
            trace_id,
            spans,
            processes,
        });
    }

    JaegerTrace { data: traces_data }
}
