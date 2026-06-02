use crate::models::{AnalysisReport, DependencyEdge};
use anyhow::{Context, Result};
use prettytable::{Cell, Row, Table};
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
    Dot,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            "dot" => Ok(OutputFormat::Dot),
            _ => Err(format!("Invalid output format: {}. Valid formats: text, json, dot", s)),
        }
    }
}

pub fn format_text_report(report: &AnalysisReport) -> String {
    let mut output = String::new();

    output.push_str(&format!("╔{}╗\n", "═".repeat(78)));
    output.push_str(&format!("║{:^78}║\n", "分布式追踪瓶颈分析报告"));
    output.push_str(&format!("╠{}╣\n", "═".repeat(78)));

    output.push_str(&format!("║ {:<20} {:<54} ║\n", "总追踪数:", report.total_traces));
    output.push_str(&format!("║ {:<20} {:<54} ║\n", "总服务数:", report.total_services));
    output.push_str(&format!("╠{}╣\n", "═".repeat(78)));

    output.push_str(&format!("║{:^78}║\n", "瓶颈服务排名"));
    output.push_str(&format!("╠{}╣\n", "═".repeat(78)));
    output.push_str(&format!(
        "║ {:<3} {:<25} {:<12} {:<12} {:<10} {:<12} ║\n",
        "排名", "服务名称", "瓶颈得分", "总贡献(μs)", "出现次数", "关键路径次数"
    ));
    output.push_str(&format!("╠{}╣\n", "═".repeat(78)));

    for (i, rank) in report.service_ranks.iter().enumerate() {
        let marker = if i < 3 { "★" } else { " " };
        output.push_str(&format!(
            "║ {:<3} {:<25} {:<12.4} {:<12} {:<10} {:<12} ║\n",
            format!("{}{}", i + 1, marker),
            rank.service_name,
            rank.bottleneck_score,
            rank.total_contribution,
            rank.appearance_count,
            rank.critical_path_count
        ));
    }

    output.push_str(&format!("╠{}╣\n", "═".repeat(78)));
    output.push_str(&format!("║{:^78}║\n", "服务依赖图"));
    output.push_str(&format!("╠{}╣\n", "═".repeat(78)));
    output.push_str(&format!(
        "║ {:<22} {:<22} {:<7} {:<10} {:<8} {:<7} ║\n",
        "调用方", "被调用方", "类型", "平均延迟", "调用次数", "队列延迟"
    ));
    output.push_str(&format!("╠{}╣\n", "═".repeat(78)));

    for edge in &report.dependency_graph {
        let call_type = if edge.is_async { "异步" } else { "同步" };
        let queue_latency = edge.avg_queue_latency.map_or("-".to_string(), |l| format!("{:.0}", l));
        output.push_str(&format!(
            "║ {:<22} {:<22} {:<7} {:<10.1} {:<8} {:<7} ║\n",
            edge.from,
            edge.to,
            call_type,
            edge.avg_duration,
            edge.call_count,
            queue_latency
        ));
    }

    output.push_str(&format!("╚{}╝\n", "═".repeat(78)));

    output.push_str(&format!("\n{}", format_critical_paths_table(report)));

    output
}

pub fn format_critical_paths_table(report: &AnalysisReport) -> String {
    let mut table = Table::new();
    
    table.add_row(Row::new(vec![
        Cell::new("追踪ID"),
        Cell::new("总延迟(μs)"),
        Cell::new("关键路径"),
        Cell::new("瓶颈节点"),
    ]));

    for analysis in &report.trace_analyses {
        let path_str = analysis
            .critical_path
            .iter()
            .map(|n| {
                let async_marker = if n.is_async { "⇝" } else { "" };
                let queue_delay = n.async_propagation_delay
                    .map(|d| format!(" [queue:{}μs]", d))
                    .unwrap_or_default();
                format!("{}{}({}μs{})", n.service_name, async_marker, n.duration, queue_delay)
            })
            .collect::<Vec<_>>()
            .join(" → ");

        let bottleneck_str = analysis
            .critical_path
            .iter()
            .filter(|n| n.is_bottleneck)
            .map(|n| {
                let async_marker = if n.is_async { "⇝" } else { "" };
                format!("{}{}({}μs)", n.service_name, async_marker, n.duration)
            })
            .collect::<Vec<_>>()
            .join(", ");

        table.add_row(Row::new(vec![
            Cell::new(&analysis.trace_id),
            Cell::new(&analysis.total_duration.to_string()),
            Cell::new(&path_str),
            Cell::new(&if bottleneck_str.is_empty() {
                "无".to_string()
            } else {
                bottleneck_str
            }),
        ]));
    }

    table.to_string()
}

pub fn format_json_report(report: &AnalysisReport) -> Result<String> {
    #[derive(Serialize)]
    struct JsonCriticalPathNode<'a> {
        service_name: &'a str,
        span_id: &'a str,
        operation: &'a str,
        duration: u64,
        start_time: u64,
        cumulative_duration: u64,
        is_bottleneck: bool,
        is_async: bool,
        async_propagation_delay: Option<u64>,
    }

    #[derive(Serialize)]
    struct JsonTraceAnalysis<'a> {
        trace_id: &'a str,
        total_duration: u64,
        services: &'a [String],
        critical_path: Vec<JsonCriticalPathNode<'a>>,
        bottleneck_scores: &'a std::collections::HashMap<String, f64>,
    }

    #[derive(Serialize)]
    struct JsonReport<'a> {
        total_traces: usize,
        total_services: usize,
        service_ranks: &'a [crate::models::ServiceBottleneckRank],
        trace_analyses: Vec<JsonTraceAnalysis<'a>>,
        dependency_graph: &'a [DependencyEdge],
    }

    let trace_analyses: Vec<JsonTraceAnalysis> = report
        .trace_analyses
        .iter()
        .map(|ta| JsonTraceAnalysis {
            trace_id: &ta.trace_id,
            total_duration: ta.total_duration,
            services: &ta.services,
            critical_path: ta
                .critical_path
                .iter()
                .map(|cp| JsonCriticalPathNode {
                    service_name: &cp.service_name,
                    span_id: &cp.span_id,
                    operation: &cp.operation,
                    duration: cp.duration,
                    start_time: cp.start_time,
                    cumulative_duration: cp.cumulative_duration,
                    is_bottleneck: cp.is_bottleneck,
                    is_async: cp.is_async,
                    async_propagation_delay: cp.async_propagation_delay,
                })
                .collect(),
            bottleneck_scores: &ta.bottleneck_scores,
        })
        .collect();

    let json_report = JsonReport {
        total_traces: report.total_traces,
        total_services: report.total_services,
        service_ranks: &report.service_ranks,
        trace_analyses,
        dependency_graph: &report.dependency_graph,
    };

    serde_json::to_string_pretty(&json_report).context("Failed to serialize report to JSON")
}

pub fn format_dot_graph(report: &AnalysisReport, highlight_bottlenecks: bool) -> String {
    let mut output = String::new();
    
    output.push_str("digraph ServiceDependency {\n");
    output.push_str("    graph [fontname=\"Arial\", rankdir=\"LR\", bgcolor=\"#ffffff\"];\n");
    output.push_str("    node [fontname=\"Arial\", fontsize=12, shape=\"box\", style=\"rounded,filled\"];\n");
    output.push_str("    edge [fontname=\"Arial\", fontsize=10];\n\n");

    let bottleneck_services: std::collections::HashSet<String> = report
        .service_ranks
        .iter()
        .take(3)
        .map(|r| r.service_name.clone())
        .collect();

    let service_colors: std::collections::HashMap<String, (String, String)> = report
        .service_ranks
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (fill, font) = if i == 0 && highlight_bottlenecks {
                ("#ff6b6b".to_string(), "#ffffff".to_string())
            } else if i == 1 && highlight_bottlenecks {
                ("#ffa502".to_string(), "#ffffff".to_string())
            } else if i == 2 && highlight_bottlenecks {
                ("#ffd93d".to_string(), "#000000".to_string())
            } else {
                ("#e8e8e8".to_string(), "#333333".to_string())
            };
            (r.service_name.clone(), (fill, font))
        })
        .collect();

    output.push_str("    // 服务节点\n");
    for (service_name, (fill_color, font_color)) in &service_colors {
        let rank = report
            .service_ranks
            .iter()
            .position(|r| &r.service_name == service_name)
            .map(|p| p + 1)
            .unwrap_or(0);
        
        let label = if highlight_bottlenecks && bottleneck_services.contains(service_name) {
            format!(
                "{} #{}",
                service_name.replace("-", "\\n"),
                rank
            )
        } else {
            service_name.replace("-", "\\n")
        };
        
        output.push_str(&format!(
            "    \"{}\" [label=\"{}\", fillcolor=\"{}\", fontcolor=\"{}\"];\n",
            service_name, label, fill_color, font_color
        ));
    }

    output.push_str("\n    // 服务调用边\n");
    for edge in &report.dependency_graph {
        let thickness = if edge.avg_duration > 500.0 {
            3.0
        } else if edge.avg_duration > 200.0 {
            2.0
        } else {
            1.0
        };

        let color = if edge.avg_duration > 500.0 {
            "#ff6b6b"
        } else if edge.avg_duration > 200.0 {
            "#ffa502"
        } else {
            "#666666"
        };

        let style = if edge.is_async { "dashed" } else { "solid" };
        
        let queue_label = if let Some(queue_latency) = edge.avg_queue_latency {
            format!("\\nqueue: {:.1}μs", queue_latency)
        } else {
            String::new()
        };

        let async_label = if edge.is_async { " [async]" } else { "" };

        output.push_str(&format!(
            "    \"{}\" -> \"{}\" [label=\"{} calls{}\\n{:.1}μs avg{}\", penwidth={}, color=\"{}\", fontcolor=\"{}\", style=\"{}\"];\n",
            edge.from,
            edge.to,
            edge.call_count,
            async_label,
            edge.avg_duration,
            queue_label,
            thickness,
            color,
            color,
            style
        ));
    }

    if highlight_bottlenecks {
        output.push_str("\n    // 图例\n");
        output.push_str("    subgraph cluster_legend {\n");
        output.push_str("        label=\"图例\";\n");
        output.push_str("        style=\"dashed\";\n");
        output.push_str("        color=\"#999999\";\n");
        output.push_str("        rank=source;\n");
        output.push_str("        \"Top 1 Bottleneck\" [shape=box, fillcolor=\"#ff6b6b\", fontcolor=\"white\", style=\"rounded,filled\"];\n");
        output.push_str("        \"Top 2 Bottleneck\" [shape=box, fillcolor=\"#ffa502\", fontcolor=\"white\", style=\"rounded,filled\"];\n");
        output.push_str("        \"Top 3 Bottleneck\" [shape=box, fillcolor=\"#ffd93d\", fontcolor=\"black\", style=\"rounded,filled\"];\n");
        output.push_str("        \"Normal Service\" [shape=box, fillcolor=\"#e8e8e8\", fontcolor=\"#333333\", style=\"rounded,filled\"];\n");
        output.push_str("    }\n");
    }

    output.push_str("}\n");
    
    output
}

pub fn write_output<P: AsRef<Path>>(
    content: &str,
    output_path: Option<P>,
) -> Result<()> {
    match output_path {
        Some(path) => {
            fs::write(path.as_ref(), content)
                .with_context(|| format!("Failed to write output to {:?}", path.as_ref()))?;
            println!("输出已写入: {:?}", path.as_ref());
        }
        None => {
            println!("{}", content);
        }
    }
    Ok(())
}

pub fn generate_output(
    report: &AnalysisReport,
    format: OutputFormat,
    output_path: Option<&Path>,
) -> Result<()> {
    let content = match format {
        OutputFormat::Text => format_text_report(report),
        OutputFormat::Json => format_json_report(report)?,
        OutputFormat::Dot => format_dot_graph(report, true),
    };

    write_output(&content, output_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AnalysisReport, CriticalPathNode, DependencyEdge, ServiceBottleneckRank, TraceAnalysis,
    };
    use std::collections::HashMap;

    fn create_test_report() -> AnalysisReport {
        AnalysisReport {
            total_traces: 2,
            total_services: 4,
            service_ranks: vec![
                ServiceBottleneckRank {
                    service_name: "order-service".to_string(),
                    total_contribution: 1100,
                    appearance_count: 2,
                    avg_contribution: 550.0,
                    bottleneck_score: 0.85,
                    critical_path_count: 2,
                },
                ServiceBottleneckRank {
                    service_name: "db".to_string(),
                    total_contribution: 400,
                    appearance_count: 1,
                    avg_contribution: 400.0,
                    bottleneck_score: 0.65,
                    critical_path_count: 1,
                },
            ],
            trace_analyses: vec![
                TraceAnalysis {
                    trace_id: "t1".to_string(),
                    total_duration: 1000,
                    services: vec!["api".to_string(), "order".to_string()],
                    critical_path: vec![
                        CriticalPathNode {
                            service_name: "api".to_string(),
                            span_id: "s1".to_string(),
                            operation: "op1".to_string(),
                            duration: 100,
                            start_time: 1000,
                            cumulative_duration: 100,
                            is_bottleneck: false,
                            is_async: false,
                            async_propagation_delay: None,
                        },
                        CriticalPathNode {
                            service_name: "order-service".to_string(),
                            span_id: "s2".to_string(),
                            operation: "op2".to_string(),
                            duration: 900,
                            start_time: 1100,
                            cumulative_duration: 1000,
                            is_bottleneck: true,
                            is_async: false,
                            async_propagation_delay: None,
                        },
                    ],
                    bottleneck_scores: HashMap::new(),
                },
            ],
            dependency_graph: vec![DependencyEdge {
                from: "api-gateway".to_string(),
                to: "order-service".to_string(),
                call_count: 10,
                avg_duration: 150.0,
                total_duration: 1500,
                is_async: false,
                avg_queue_latency: None,
            }],
        }
    }

    #[test]
    fn test_format_text_report() {
        let report = create_test_report();
        let output = format_text_report(&report);
        
        assert!(output.contains("分布式追踪瓶颈分析报告"));
        assert!(output.contains("order-service"));
        assert!(output.contains("瓶颈得分"));
        assert!(output.contains("总追踪数:"));
        assert!(output.contains("2"));
        assert!(output.contains("总服务数:"));
        assert!(output.contains("4"));
    }

    #[test]
    fn test_format_json_report() {
        let report = create_test_report();
        let output = format_json_report(&report).unwrap();
        
        let json: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(json["total_traces"], 2);
        assert_eq!(json["total_services"], 4);
        assert!(json["service_ranks"].is_array());
        assert!(json["dependency_graph"].is_array());
    }

    #[test]
    fn test_format_dot_graph() {
        let report = create_test_report();
        let output = format_dot_graph(&report, true);
        
        assert!(output.contains("digraph ServiceDependency"));
        assert!(output.contains("order-service"));
        assert!(output.contains("api-gateway"));
        assert!(output.contains("->"));
        assert!(output.contains("#ff6b6b"));
    }

    #[test]
    fn test_output_format_from_str() {
        assert_eq!("text".parse::<OutputFormat>().unwrap(), OutputFormat::Text);
        assert_eq!("json".parse::<OutputFormat>().unwrap(), OutputFormat::Json);
        assert_eq!("dot".parse::<OutputFormat>().unwrap(), OutputFormat::Dot);
        assert_eq!("TEXT".parse::<OutputFormat>().unwrap(), OutputFormat::Text);
        assert!("invalid".parse::<OutputFormat>().is_err());
    }

    #[test]
    fn test_format_critical_paths_table() {
        let report = create_test_report();
        let output = format_critical_paths_table(&report);
        
        assert!(output.contains("追踪ID"));
        assert!(output.contains("关键路径"));
        assert!(output.contains("order-service"));
        assert!(output.contains("t1"));
    }
}
