use crate::critical_path::analyze_all_traces;
use crate::dependency_graph::ServiceDependencyGraph;
use crate::models::{AnalysisReport, JaegerTrace, ServiceBottleneckRank, TraceAnalysis};
use crate::parser::parse_traces;
use anyhow::Result;
use std::collections::HashMap;

pub struct ServiceStats {
    pub total_contribution: u64,
    pub appearance_count: usize,
    pub critical_path_count: usize,
    pub score_sum: f64,
    pub durations: Vec<u64>,
}

impl Default for ServiceStats {
    fn default() -> Self {
        Self {
            total_contribution: 0,
            appearance_count: 0,
            critical_path_count: 0,
            score_sum: 0.0,
            durations: Vec::new(),
        }
    }
}

pub fn aggregate_service_stats(
    trace_analyses: &[TraceAnalysis],
) -> HashMap<String, ServiceStats> {
    let mut stats: HashMap<String, ServiceStats> = HashMap::new();

    for analysis in trace_analyses {
        let mut in_critical_path = HashSet::new();
        
        for node in &analysis.critical_path {
            in_critical_path.insert(node.service_name.clone());
            let service_stats = stats.entry(node.service_name.clone()).or_default();
            service_stats.total_contribution += node.duration;
            service_stats.durations.push(node.duration);
        }

        for service in &analysis.services {
            let service_stats = stats.entry(service.clone()).or_default();
            service_stats.appearance_count += 1;
            
            if in_critical_path.contains(service) {
                service_stats.critical_path_count += 1;
            }
        }

        for (service, score) in &analysis.bottleneck_scores {
            let service_stats = stats.entry(service.clone()).or_default();
            service_stats.score_sum += score;
        }
    }

    stats
}

use std::collections::HashSet;

pub fn calculate_bottleneck_score(
    stats: &ServiceStats,
    total_traces: usize,
) -> f64 {
    if stats.appearance_count == 0 {
        return 0.0;
    }

    let critical_path_ratio = if total_traces > 0 {
        stats.critical_path_count as f64 / total_traces as f64
    } else {
        0.0
    };

    let avg_score = if stats.durations.is_empty() {
        0.0
    } else {
        stats.score_sum / stats.durations.len() as f64
    };

    let avg_duration = if stats.durations.is_empty() {
        0.0
    } else {
        stats.total_contribution as f64 / stats.durations.len() as f64
    };

    critical_path_ratio * 0.5 + avg_score * 0.3 + (avg_duration.log10().max(0.0) * 0.0001) * 0.2
}

pub fn rank_services(
    stats: &HashMap<String, ServiceStats>,
    total_traces: usize,
) -> Vec<ServiceBottleneckRank> {
    let mut ranks: Vec<ServiceBottleneckRank> = stats
        .iter()
        .map(|(service_name, stats)| {
            let avg_contribution = if stats.durations.is_empty() {
                0.0
            } else {
                stats.total_contribution as f64 / stats.durations.len() as f64
            };

            let bottleneck_score = calculate_bottleneck_score(stats, total_traces);

            ServiceBottleneckRank {
                service_name: service_name.clone(),
                total_contribution: stats.total_contribution,
                appearance_count: stats.appearance_count,
                avg_contribution,
                bottleneck_score,
                critical_path_count: stats.critical_path_count,
            }
        })
        .collect();

    ranks.sort_by(|a, b| {
        b.bottleneck_score
            .partial_cmp(&a.bottleneck_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    ranks
}

pub fn generate_report(
    jaeger_trace: &JaegerTrace,
) -> Result<AnalysisReport> {
    let traces = parse_traces(jaeger_trace)?;
    let trace_analyses = analyze_all_traces(&traces)?;
    let dependency_graph = ServiceDependencyGraph::from_trace_data(&traces)?;
    let dependency_edges = dependency_graph.to_dependency_edges();

    let service_stats = aggregate_service_stats(&trace_analyses);
    let service_ranks = rank_services(&service_stats, trace_analyses.len());

    let total_services = service_ranks.len();

    Ok(AnalysisReport {
        total_traces: trace_analyses.len(),
        total_services,
        service_ranks,
        trace_analyses,
        dependency_graph: dependency_edges,
    })
}

pub fn get_top_bottlenecks(
    report: &AnalysisReport,
    top_n: usize,
) -> Vec<&ServiceBottleneckRank> {
    report.service_ranks.iter().take(top_n).collect()
}

pub fn get_service_analysis_summary(
    report: &AnalysisReport,
) -> String {
    let mut summary = String::new();
    
    summary.push_str(&format!("=== 分布式追踪瓶颈分析报告 ===\n\n"));
    summary.push_str(&format!("总追踪数: {}\n", report.total_traces));
    summary.push_str(&format!("总服务数: {}\n\n", report.total_services));
    
    summary.push_str("=== 瓶颈服务排名 (前5名) ===\n");
    for (i, rank) in report.service_ranks.iter().take(5).enumerate() {
        summary.push_str(&format!(
            "{}. {} - 瓶颈得分: {:.4}, 总贡献: {}μs, 出现次数: {}, 关键路径次数: {}\n",
            i + 1,
            rank.service_name,
            rank.bottleneck_score,
            rank.total_contribution,
            rank.appearance_count,
            rank.critical_path_count
        ));
    }
    
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CriticalPathNode, TraceAnalysis};
    use std::collections::HashMap;

    fn create_test_trace_analyses() -> Vec<TraceAnalysis> {
        vec![
            TraceAnalysis {
                trace_id: "t1".to_string(),
                total_duration: 1000,
                services: vec!["api-gateway".to_string(), "order-service".to_string(), "db".to_string()],
                critical_path: vec![
                    CriticalPathNode {
                        service_name: "api-gateway".to_string(),
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
                        duration: 500,
                        start_time: 1100,
                        cumulative_duration: 600,
                        is_bottleneck: true,
                        is_async: false,
                        async_propagation_delay: None,
                    },
                    CriticalPathNode {
                        service_name: "db".to_string(),
                        span_id: "s3".to_string(),
                        operation: "op3".to_string(),
                        duration: 400,
                        start_time: 1200,
                        cumulative_duration: 1000,
                        is_bottleneck: true,
                        is_async: false,
                        async_propagation_delay: None,
                    },
                ],
                bottleneck_scores: HashMap::from([
                    ("api-gateway".to_string(), 0.1),
                    ("order-service".to_string(), 0.5),
                    ("db".to_string(), 0.4),
                ]),
            },
            TraceAnalysis {
                trace_id: "t2".to_string(),
                total_duration: 800,
                services: vec!["api-gateway".to_string(), "order-service".to_string(), "cache".to_string()],
                critical_path: vec![
                    CriticalPathNode {
                        service_name: "api-gateway".to_string(),
                        span_id: "s4".to_string(),
                        operation: "op4".to_string(),
                        duration: 100,
                        start_time: 2000,
                        cumulative_duration: 100,
                        is_bottleneck: false,
                        is_async: false,
                        async_propagation_delay: None,
                    },
                    CriticalPathNode {
                        service_name: "order-service".to_string(),
                        span_id: "s5".to_string(),
                        operation: "op5".to_string(),
                        duration: 600,
                        start_time: 2100,
                        cumulative_duration: 700,
                        is_bottleneck: true,
                        is_async: false,
                        async_propagation_delay: None,
                    },
                    CriticalPathNode {
                        service_name: "cache".to_string(),
                        span_id: "s6".to_string(),
                        operation: "op6".to_string(),
                        duration: 100,
                        start_time: 2200,
                        cumulative_duration: 800,
                        is_bottleneck: false,
                        is_async: false,
                        async_propagation_delay: None,
                    },
                ],
                bottleneck_scores: HashMap::from([
                    ("api-gateway".to_string(), 0.125),
                    ("order-service".to_string(), 0.75),
                    ("cache".to_string(), 0.125),
                ]),
            },
        ]
    }

    #[test]
    fn test_aggregate_service_stats() {
        let analyses = create_test_trace_analyses();
        let stats = aggregate_service_stats(&analyses);
        
        assert_eq!(stats.len(), 4);
        
        let order_stats = stats.get("order-service").unwrap();
        assert_eq!(order_stats.total_contribution, 1100);
        assert_eq!(order_stats.appearance_count, 2);
        assert_eq!(order_stats.critical_path_count, 2);
        
        let api_stats = stats.get("api-gateway").unwrap();
        assert_eq!(api_stats.appearance_count, 2);
        assert_eq!(api_stats.critical_path_count, 2);
        
        let db_stats = stats.get("db").unwrap();
        assert_eq!(db_stats.appearance_count, 1);
        assert_eq!(db_stats.critical_path_count, 1);
    }

    #[test]
    fn test_calculate_bottleneck_score() {
        let mut stats = ServiceStats::default();
        stats.total_contribution = 1000;
        stats.appearance_count = 2;
        stats.critical_path_count = 2;
        stats.score_sum = 1.5;
        stats.durations = vec![500, 500];
        
        let score = calculate_bottleneck_score(&stats, 2);
        assert!(score > 0.0);
    }

    #[test]
    fn test_rank_services() {
        let analyses = create_test_trace_analyses();
        let stats = aggregate_service_stats(&analyses);
        let ranks = rank_services(&stats, 2);
        
        assert_eq!(ranks.len(), 4);
        assert_eq!(ranks[0].service_name, "order-service");
    }

    #[test]
    fn test_get_top_bottlenecks() {
        let analyses = create_test_trace_analyses();
        let stats = aggregate_service_stats(&analyses);
        let ranks = rank_services(&stats, 2);
        
        let report = AnalysisReport {
            total_traces: 2,
            total_services: 4,
            service_ranks: ranks,
            trace_analyses: analyses,
            dependency_graph: vec![],
        };
        
        let top = get_top_bottlenecks(&report, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].service_name, "order-service");
    }

    #[test]
    fn test_get_service_analysis_summary() {
        let analyses = create_test_trace_analyses();
        let stats = aggregate_service_stats(&analyses);
        let ranks = rank_services(&stats, 2);
        
        let report = AnalysisReport {
            total_traces: 2,
            total_services: 4,
            service_ranks: ranks,
            trace_analyses: analyses,
            dependency_graph: vec![],
        };
        
        let summary = get_service_analysis_summary(&report);
        assert!(summary.contains("瓶颈服务排名"));
        assert!(summary.contains("order-service"));
        assert!(summary.contains("总追踪数: 2"));
    }
}
