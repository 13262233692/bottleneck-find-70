use crate::models::{DependencyEdge, ServiceCall, TraceData};
use crate::parser::build_service_calls;
use anyhow::Result;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

pub struct ServiceDependencyGraph {
    pub graph: DiGraph<String, EdgeWeight>,
    pub node_indices: HashMap<String, NodeIndex>,
}

#[derive(Debug, Clone)]
pub struct EdgeWeight {
    pub call_count: usize,
    pub total_duration: u64,
    pub avg_duration: f64,
    pub is_async: bool,
    pub total_queue_latency: u64,
    pub queue_latency_count: usize,
}

impl ServiceDependencyGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_indices: HashMap::new(),
        }
    }

    pub fn get_or_add_node(&mut self, service_name: &str) -> NodeIndex {
        if let Some(&idx) = self.node_indices.get(service_name) {
            idx
        } else {
            let idx = self.graph.add_node(service_name.to_string());
            self.node_indices.insert(service_name.to_string(), idx);
            idx
        }
    }

    pub fn add_service_call(&mut self, call: &ServiceCall) {
        let from_idx = self.get_or_add_node(&call.from_service);
        let to_idx = self.get_or_add_node(&call.to_service);
        
        if let Some(edge_idx) = self.graph.find_edge(from_idx, to_idx) {
            let weight = self.graph.edge_weight_mut(edge_idx).unwrap();
            weight.call_count += 1;
            weight.total_duration += call.duration;
            weight.avg_duration = weight.total_duration as f64 / weight.call_count as f64;
            
            if call.is_async {
                weight.is_async = true;
            }
            
            if let Some(queue_latency) = call.queue_latency {
                weight.total_queue_latency += queue_latency;
                weight.queue_latency_count += 1;
            }
        } else {
            let weight = EdgeWeight {
                call_count: 1,
                total_duration: call.duration,
                avg_duration: call.duration as f64,
                is_async: call.is_async,
                total_queue_latency: call.queue_latency.unwrap_or(0),
                queue_latency_count: if call.queue_latency.is_some() { 1 } else { 0 },
            };
            self.graph.add_edge(from_idx, to_idx, weight);
        }
    }

    pub fn add_service_calls(&mut self, calls: &[ServiceCall]) {
        for call in calls {
            self.add_service_call(call);
        }
    }

    pub fn from_trace_data(traces: &[TraceData]) -> Result<Self> {
        let mut graph = Self::new();
        
        for trace in traces {
            let calls = build_service_calls(trace)?;
            graph.add_service_calls(&calls);
        }
        
        Ok(graph)
    }

    pub fn get_all_services(&self) -> Vec<String> {
        self.node_indices.keys().cloned().collect()
    }

    pub fn to_dependency_edges(&self) -> Vec<DependencyEdge> {
        let mut edges = Vec::new();
        
        for edge_idx in self.graph.edge_indices() {
            let (from_idx, to_idx) = self.graph.edge_endpoints(edge_idx).unwrap();
            let weight = self.graph.edge_weight(edge_idx).unwrap();
            let from = self.graph.node_weight(from_idx).unwrap().clone();
            let to = self.graph.node_weight(to_idx).unwrap().clone();
            
            let avg_queue_latency = if weight.queue_latency_count > 0 {
                Some(weight.total_queue_latency as f64 / weight.queue_latency_count as f64)
            } else {
                None
            };
            
            edges.push(DependencyEdge {
                from,
                to,
                call_count: weight.call_count,
                avg_duration: weight.avg_duration,
                total_duration: weight.total_duration,
                is_async: weight.is_async,
                avg_queue_latency,
            });
        }
        
        edges
    }

    pub fn get_outgoing_edges(&self, service: &str) -> Vec<(String, &EdgeWeight)> {
        let mut result = Vec::new();
        
        if let Some(&idx) = self.node_indices.get(service) {
            for edge_idx in self.graph.edges(idx) {
                let target = self.graph.node_weight(edge_idx.target()).unwrap().clone();
                result.push((target, edge_idx.weight()));
            }
        }
        
        result
    }

    pub fn get_incoming_edges(&self, service: &str) -> Vec<(String, &EdgeWeight)> {
        let mut result = Vec::new();
        
        if let Some(&idx) = self.node_indices.get(service) {
            for edge_idx in self.graph.edges_directed(idx, petgraph::Direction::Incoming) {
                let source = self.graph.node_weight(edge_idx.source()).unwrap().clone();
                result.push((source, edge_idx.weight()));
            }
        }
        
        result
    }

    pub fn get_async_edges(&self) -> Vec<DependencyEdge> {
        self.to_dependency_edges()
            .into_iter()
            .filter(|e| e.is_async)
            .collect()
    }
}

impl Default for ServiceDependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_calls() -> Vec<ServiceCall> {
        vec![
            ServiceCall {
                from_service: "api-gateway".to_string(),
                to_service: "order-service".to_string(),
                duration: 100,
                trace_id: "t1".to_string(),
                span_id: "s1".to_string(),
                operation: "op1".to_string(),
                start_time: 1000,
                is_async: false,
                queue_latency: None,
            },
            ServiceCall {
                from_service: "api-gateway".to_string(),
                to_service: "order-service".to_string(),
                duration: 200,
                trace_id: "t2".to_string(),
                span_id: "s2".to_string(),
                operation: "op2".to_string(),
                start_time: 2000,
                is_async: false,
                queue_latency: None,
            },
            ServiceCall {
                from_service: "order-service".to_string(),
                to_service: "redis-cache".to_string(),
                duration: 50,
                trace_id: "t1".to_string(),
                span_id: "s3".to_string(),
                operation: "op3".to_string(),
                start_time: 1100,
                is_async: false,
                queue_latency: None,
            },
        ]
    }

    fn create_test_async_calls() -> Vec<ServiceCall> {
        vec![
            ServiceCall {
                from_service: "order-service".to_string(),
                to_service: "kafka".to_string(),
                duration: 100,
                trace_id: "t1".to_string(),
                span_id: "s1".to_string(),
                operation: "produce".to_string(),
                start_time: 1000,
                is_async: false,
                queue_latency: None,
            },
            ServiceCall {
                from_service: "kafka".to_string(),
                to_service: "notification-service".to_string(),
                duration: 300,
                trace_id: "t1".to_string(),
                span_id: "s2".to_string(),
                operation: "consume".to_string(),
                start_time: 1500,
                is_async: true,
                queue_latency: Some(400),
            },
        ]
    }

    #[test]
    fn test_add_service_call() {
        let mut graph = ServiceDependencyGraph::new();
        let calls = create_test_calls();
        
        graph.add_service_calls(&calls);
        
        assert_eq!(graph.node_indices.len(), 3);
        assert!(graph.node_indices.contains_key("api-gateway"));
        assert!(graph.node_indices.contains_key("order-service"));
        assert!(graph.node_indices.contains_key("redis-cache"));
        
        let edges = graph.to_dependency_edges();
        assert_eq!(edges.len(), 2);
        
        let order_edge = edges.iter().find(|e| e.from == "api-gateway" && e.to == "order-service").unwrap();
        assert_eq!(order_edge.call_count, 2);
        assert_eq!(order_edge.total_duration, 300);
        assert_eq!(order_edge.avg_duration, 150.0);
        assert!(!order_edge.is_async);
        
        let redis_edge = edges.iter().find(|e| e.from == "order-service" && e.to == "redis-cache").unwrap();
        assert_eq!(redis_edge.call_count, 1);
        assert_eq!(redis_edge.total_duration, 50);
        assert_eq!(redis_edge.avg_duration, 50.0);
    }

    #[test]
    fn test_async_service_call() {
        let mut graph = ServiceDependencyGraph::new();
        let calls = create_test_async_calls();
        
        graph.add_service_calls(&calls);
        
        let edges = graph.to_dependency_edges();
        assert_eq!(edges.len(), 2);
        
        let async_edge = edges.iter().find(|e| e.is_async).unwrap();
        assert_eq!(async_edge.from, "kafka");
        assert_eq!(async_edge.to, "notification-service");
        assert_eq!(async_edge.avg_queue_latency, Some(400.0));
        
        let async_edges = graph.get_async_edges();
        assert_eq!(async_edges.len(), 1);
    }

    #[test]
    fn test_get_outgoing_edges() {
        let mut graph = ServiceDependencyGraph::new();
        let calls = create_test_calls();
        graph.add_service_calls(&calls);
        
        let outgoing = graph.get_outgoing_edges("api-gateway");
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].0, "order-service");
        
        let outgoing_order = graph.get_outgoing_edges("order-service");
        assert_eq!(outgoing_order.len(), 1);
        assert_eq!(outgoing_order[0].0, "redis-cache");
    }

    #[test]
    fn test_get_incoming_edges() {
        let mut graph = ServiceDependencyGraph::new();
        let calls = create_test_calls();
        graph.add_service_calls(&calls);
        
        let incoming = graph.get_incoming_edges("order-service");
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].0, "api-gateway");
        
        let incoming_redis = graph.get_incoming_edges("redis-cache");
        assert_eq!(incoming_redis.len(), 1);
        assert_eq!(incoming_redis[0].0, "order-service");
    }
}
