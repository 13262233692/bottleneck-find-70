use crate::models::{CriticalPathNode, Process, Span, TraceAnalysis};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};

pub struct SpanTreeNode<'a> {
    pub span: &'a Span,
    pub children: Vec<SpanTreeNode<'a>>,
    pub follows_from: Vec<SpanTreeNode<'a>>,
    pub service_name: String,
    pub self_duration: u64,
    pub is_async_root: bool,
}

pub struct AsyncEdgeInfo<'a> {
    pub producer_span: &'a Span,
    pub consumer_span: &'a Span,
    pub queue_latency: u64,
}

pub fn build_span_tree<'a>(
    spans: &'a [Span],
    process_map: &'a HashMap<String, Process>,
) -> Result<(Vec<SpanTreeNode<'a>>, Vec<AsyncEdgeInfo<'a>>)> {
    let span_map: HashMap<&String, &Span> = spans
        .iter()
        .map(|s| (&s.span_id, s))
        .collect();

    let mut children_map: HashMap<&String, Vec<&Span>> = HashMap::new();
    let mut follows_from_map: HashMap<&String, Vec<&Span>> = HashMap::new();
    let mut root_spans: Vec<&Span> = Vec::new();
    let mut async_edges = Vec::new();

    for span in spans {
        let mut has_parent = false;
        
        for reference in &span.references {
            if reference.ref_type == "CHILD_OF" {
                if span_map.contains_key(&reference.span_id) {
                    children_map
                        .entry(&reference.span_id)
                        .or_insert_with(Vec::new)
                        .push(span);
                    has_parent = true;
                }
            } else if reference.ref_type == "FOLLOWS_FROM" {
                if let Some(producer_span) = span_map.get(&reference.span_id) {
                    follows_from_map
                        .entry(&reference.span_id)
                        .or_insert_with(Vec::new)
                        .push(span);
                    
                    has_parent = true;
                    
                    let queue_latency = span.start_time.saturating_sub(
                        producer_span.start_time + producer_span.duration
                    );
                    async_edges.push(AsyncEdgeInfo {
                        producer_span,
                        consumer_span: span,
                        queue_latency,
                    });
                }
            }
        }
        
        if !has_parent {
            root_spans.push(span);
        }
    }

    fn build_node<'a>(
        span: &'a Span,
        children_map: &HashMap<&String, Vec<&'a Span>>,
        follows_from_map: &HashMap<&String, Vec<&'a Span>>,
        process_map: &'a HashMap<String, Process>,
        visited: &mut HashSet<&'a String>,
    ) -> Result<SpanTreeNode<'a>> {
        if visited.contains(&span.span_id) {
            return Ok(SpanTreeNode {
                span,
                children: Vec::new(),
                follows_from: Vec::new(),
                service_name: process_map
                    .get(&span.process_id)
                    .map(|p| p.service_name.clone())
                    .unwrap_or_default(),
                self_duration: 0,
                is_async_root: false,
            });
        }
        visited.insert(&span.span_id);

        let service_name = process_map
            .get(&span.process_id)
            .with_context(|| format!("Process not found for span: {}", span.span_id))?
            .service_name
            .clone();

        let children = children_map
            .get(&span.span_id)
            .unwrap_or(&Vec::new())
            .iter()
            .map(|child| build_node(child, children_map, follows_from_map, process_map, visited))
            .collect::<Result<Vec<_>>>()?;

        let follows_from = follows_from_map
            .get(&span.span_id)
            .unwrap_or(&Vec::new())
            .iter()
            .map(|consumer| build_node(consumer, children_map, follows_from_map, process_map, visited))
            .collect::<Result<Vec<_>>>()?;

        let children_duration: u64 = children.iter().map(|c| c.span.duration).sum();
        let self_duration = span.duration.saturating_sub(children_duration);

        Ok(SpanTreeNode {
            span,
            children,
            follows_from,
            service_name,
            self_duration,
            is_async_root: false,
        })
    }

    let mut visited = HashSet::new();
    let trees = root_spans
        .iter()
        .map(|root| build_node(root, &children_map, &follows_from_map, process_map, &mut visited))
        .collect::<Result<Vec<_>>>()?;

    Ok((trees, async_edges))
}

pub fn find_critical_path<'a>(
    roots: &'a [SpanTreeNode<'a>],
    async_edges: &'a [AsyncEdgeInfo<'a>],
    _process_map: &HashMap<String, Process>,
) -> Result<Vec<CriticalPathNode>> {
    #[derive(Clone)]
    #[allow(dead_code)]
    struct PathNode {
        node: CriticalPathNode,
        cumulative_self_duration: u64,
    }

    fn find_longest_path<'a>(
        node: &'a SpanTreeNode<'a>,
        current_path: &mut Vec<PathNode>,
        longest_path: &mut Vec<CriticalPathNode>,
        current_self_duration: u64,
        longest_self_duration: &mut u64,
        root_start_time: u64,
        is_async: bool,
        async_propagation_delay: Option<u64>,
        async_edges: &'a [AsyncEdgeInfo<'a>],
    ) {
        let node_end_time = node.span.start_time + node.span.duration;
        let cumulative_duration = node_end_time - root_start_time;

        let path_node = PathNode {
            node: CriticalPathNode {
                service_name: node.service_name.clone(),
                span_id: node.span.span_id.clone(),
                operation: node.span.operation.clone(),
                duration: node.span.duration,
                start_time: node.span.start_time,
                cumulative_duration,
                is_bottleneck: false,
                is_async,
                async_propagation_delay,
            },
            cumulative_self_duration: current_self_duration + node.self_duration,
        };

        current_path.push(path_node);
        let new_self_duration = current_self_duration + node.self_duration;

        if node.children.is_empty() && node.follows_from.is_empty() {
            if new_self_duration > *longest_self_duration {
                *longest_self_duration = new_self_duration;
                *longest_path = current_path.iter().map(|p| p.node.clone()).collect();
            }
        }

        for child in &node.children {
            find_longest_path(
                child,
                current_path,
                longest_path,
                new_self_duration,
                longest_self_duration,
                root_start_time,
                false,
                None,
                async_edges,
            );
        }

        for consumer in &node.follows_from {
            let queue_latency = async_edges
                .iter()
                .find(|e| e.consumer_span.span_id == consumer.span.span_id)
                .map(|e| e.queue_latency);

            find_longest_path(
                consumer,
                current_path,
                longest_path,
                new_self_duration,
                longest_self_duration,
                root_start_time,
                true,
                queue_latency,
                async_edges,
            );
        }

        current_path.pop();
    }

    let mut longest_path = Vec::new();
    let mut longest_self_duration = 0u64;

    for root in roots {
        let mut current_path = Vec::new();
        let root_start_time = root.span.start_time;
        find_longest_path(
            root,
            &mut current_path,
            &mut longest_path,
            0,
            &mut longest_self_duration,
            root_start_time,
            false,
            None,
            async_edges,
        );
    }

    if !longest_path.is_empty() {
        let max_self_duration = longest_path
            .iter()
            .map(|n| n.duration)
            .max()
            .unwrap_or(0);
        
        let threshold = (max_self_duration as f64) * 0.3;
        
        for node in &mut longest_path {
            node.is_bottleneck = node.duration as f64 >= threshold;
        }
    }

    Ok(longest_path)
}

pub fn calculate_bottleneck_scores(
    critical_path: &[CriticalPathNode],
    total_duration: u64,
) -> HashMap<String, f64> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    
    if total_duration == 0 {
        return scores;
    }

    let mut producer_contribution: HashMap<String, u64> = HashMap::new();

    for i in 0..critical_path.len() {
        let node = &critical_path[i];
        let mut contribution = node.duration;

        if node.is_async {
            if let Some(delay) = node.async_propagation_delay {
                contribution += delay;
            }
            
            if i > 0 {
                let producer_service = &critical_path[i - 1].service_name;
                *producer_contribution.entry(producer_service.clone()).or_insert(0) += node.duration;
            }
        }

        let score_contribution = contribution as f64 / total_duration as f64;
        let entry = scores.entry(node.service_name.clone()).or_insert(0.0);
        *entry += score_contribution;
    }

    for (service, contribution) in producer_contribution {
        let score_contribution = contribution as f64 / total_duration as f64;
        let entry = scores.entry(service).or_insert(0.0);
        *entry += score_contribution * 0.5;
    }

    scores
}

pub fn analyze_trace(
    trace_data: &crate::models::TraceData,
) -> Result<TraceAnalysis> {
    let (roots, async_edges) = build_span_tree(&trace_data.spans, &trace_data.processes)?;
    let critical_path = find_critical_path(&roots, &async_edges, &trace_data.processes)?;

    let total_duration = trace_data
        .spans
        .iter()
        .map(|s| s.duration)
        .max()
        .unwrap_or(0);

    let services: HashSet<String> = trace_data
        .processes
        .values()
        .map(|p| p.service_name.clone())
        .collect();

    let bottleneck_scores = calculate_bottleneck_scores(&critical_path, total_duration);

    Ok(TraceAnalysis {
        trace_id: trace_data.trace_id.clone(),
        total_duration,
        services: services.into_iter().collect(),
        critical_path,
        bottleneck_scores,
    })
}

pub fn analyze_all_traces(
    traces: &[crate::models::TraceData],
) -> Result<Vec<TraceAnalysis>> {
    traces.iter().map(analyze_trace).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Process, Reference, Span, TraceData};
    use std::collections::HashMap;

    fn create_test_trace_with_kafka() -> TraceData {
        let processes = HashMap::from([
            ("p1".to_string(), Process {
                service_name: "order-service".to_string(),
                tags: vec![],
            }),
            ("p2".to_string(), Process {
                service_name: "kafka".to_string(),
                tags: vec![],
            }),
            ("p3".to_string(), Process {
                service_name: "notification-service".to_string(),
                tags: vec![],
            }),
        ]);

        let spans = vec![
            Span {
                trace_id: "t1".to_string(),
                span_id: "s1".to_string(),
                operation: "create_order".to_string(),
                start_time: 1000,
                duration: 2000,
                references: vec![],
                process_id: "p1".to_string(),
                tags: vec![],
            },
            Span {
                trace_id: "t1".to_string(),
                span_id: "s2".to_string(),
                operation: "send_to_kafka".to_string(),
                start_time: 1500,
                duration: 1500,
                references: vec![Reference {
                    ref_type: "CHILD_OF".to_string(),
                    trace_id: "t1".to_string(),
                    span_id: "s1".to_string(),
                }],
                process_id: "p1".to_string(),
                tags: vec![],
            },
            Span {
                trace_id: "t1".to_string(),
                span_id: "s3".to_string(),
                operation: "kafka_produce".to_string(),
                start_time: 1600,
                duration: 100,
                references: vec![Reference {
                    ref_type: "CHILD_OF".to_string(),
                    trace_id: "t1".to_string(),
                    span_id: "s2".to_string(),
                }],
                process_id: "p2".to_string(),
                tags: vec![],
            },
            Span {
                trace_id: "t1".to_string(),
                span_id: "s4".to_string(),
                operation: "kafka_consume".to_string(),
                start_time: 2800,
                duration: 200,
                references: vec![Reference {
                    ref_type: "FOLLOWS_FROM".to_string(),
                    trace_id: "t1".to_string(),
                    span_id: "s3".to_string(),
                }],
                process_id: "p2".to_string(),
                tags: vec![],
            },
            Span {
                trace_id: "t1".to_string(),
                span_id: "s5".to_string(),
                operation: "send_email".to_string(),
                start_time: 2900,
                duration: 300,
                references: vec![Reference {
                    ref_type: "CHILD_OF".to_string(),
                    trace_id: "t1".to_string(),
                    span_id: "s4".to_string(),
                }],
                process_id: "p3".to_string(),
                tags: vec![],
            },
        ];

        TraceData {
            trace_id: "t1".to_string(),
            spans,
            processes,
        }
    }

    #[test]
    fn test_build_span_tree_with_follows_from() {
        let trace_data = create_test_trace_with_kafka();
        let (roots, async_edges) = build_span_tree(&trace_data.spans, &trace_data.processes).unwrap();
        
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].children.len(), 1);
        assert_eq!(async_edges.len(), 1);
        assert_eq!(async_edges[0].queue_latency, 1100);
    }

    #[test]
    fn test_critical_path_with_async() {
        let trace_data = create_test_trace_with_kafka();
        let (roots, async_edges) = build_span_tree(&trace_data.spans, &trace_data.processes).unwrap();
        let critical_path = find_critical_path(&roots, &async_edges, &trace_data.processes).unwrap();
        
        assert!(!critical_path.is_empty());
        
        let has_async = critical_path.iter().any(|n| n.is_async);
        assert!(has_async, "关键路径应该包含异步节点");
    }

    #[test]
    fn test_bottleneck_scores_with_async_propagation() {
        let trace_data = create_test_trace_with_kafka();
        let analysis = analyze_trace(&trace_data).unwrap();
        
        let order_score = analysis.bottleneck_scores.get("order-service");
        let notification_score = analysis.bottleneck_scores.get("notification-service");
        
        assert!(order_score.is_some());
        assert!(*order_score.unwrap() > 0.0);
        
        if let Some(notification_score) = notification_score {
            assert!(*order_score.unwrap() >= notification_score * 0.5, 
                "生产者应该承担部分消费者延迟");
        }
    }

    #[test]
    fn test_queue_latency_calculation() {
        let trace_data = create_test_trace_with_kafka();
        let (_, async_edges) = build_span_tree(&trace_data.spans, &trace_data.processes).unwrap();
        
        assert_eq!(async_edges.len(), 1);
        assert_eq!(async_edges[0].queue_latency, 2800 - (1600 + 100));
    }
}
