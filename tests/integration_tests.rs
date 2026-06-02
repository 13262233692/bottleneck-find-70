use bottleneck_find::*;
use std::path::PathBuf;

fn get_test_data_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("sample_traces.json")
}

#[test]
fn test_full_analysis_pipeline() {
    let input_path = get_test_data_path();
    assert!(input_path.exists(), "测试数据文件不存在: {:?}", input_path);

    let trace = parse_jaeger_json(&input_path).expect("解析Jaeger JSON失败");
    
    assert_eq!(trace.data.len(), 5, "应该有5条追踪数据");
    
    let traces = parse_traces(&trace).expect("解析追踪数据失败");
    assert_eq!(traces.len(), 5);
    
    let service_calls = build_service_calls(&traces[0]).expect("构建服务调用失败");
    assert!(!service_calls.is_empty(), "应该有服务调用");
    
    let trace_analyses = analyze_all_traces(&traces).expect("分析追踪数据失败");
    assert_eq!(trace_analyses.len(), 5);
    
    for analysis in &trace_analyses {
        assert!(!analysis.critical_path.is_empty(), "每条追踪应该有关键路径");
        assert!(analysis.total_duration > 0, "总延迟应该大于0");
    }
    
    let dep_graph = ServiceDependencyGraph::from_trace_data(&traces)
        .expect("构建依赖图失败");
    
    assert!(dep_graph.get_all_services().len() > 0, "应该有服务节点");
    assert!(!dep_graph.to_dependency_edges().is_empty(), "应该有依赖边");
    
    let report = generate_report(&trace).expect("生成报告失败");
    
    assert_eq!(report.total_traces, 5);
    assert!(report.total_services >= 5, "至少有5个不同的服务");
    assert!(!report.service_ranks.is_empty(), "应该有服务排名");
    
    let top_bottlenecks = get_top_bottlenecks(&report, 3);
    assert_eq!(top_bottlenecks.len(), 3, "应该有前3个瓶颈服务");
    
    let text_output = format_text_report(&report);
    assert!(!text_output.is_empty());
    assert!(text_output.contains("分布式追踪瓶颈分析报告"));
    
    let json_output = format_json_report(&report).expect("生成JSON报告失败");
    assert!(!json_output.is_empty());
    let json: serde_json::Value = serde_json::from_str(&json_output).expect("JSON解析失败");
    assert_eq!(json["total_traces"], 5);
    
    let dot_output = format_dot_graph(&report, true);
    assert!(!dot_output.is_empty());
    assert!(dot_output.contains("digraph ServiceDependency"));
}

#[test]
fn test_cli_analyze_command() {
    let input_path = get_test_data_path();
    let temp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let output_path = temp_dir.path().join("output.txt");

    let result = run_analysis(&input_path, OutputFormat::Text, Some(output_path.as_path()));
    assert!(result.is_ok(), "分析命令执行失败: {:?}", result.err());
    assert!(output_path.exists(), "输出文件不存在");
}

#[test]
fn test_cli_json_output() {
    let input_path = get_test_data_path();
    let temp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let output_path = temp_dir.path().join("output.json");

    let result = run_analysis(&input_path, OutputFormat::Json, Some(output_path.as_path()));
    assert!(result.is_ok(), "JSON输出失败: {:?}", result.err());
    
    let content = std::fs::read_to_string(&output_path).expect("读取输出文件失败");
    let json: serde_json::Value = serde_json::from_str(&content).expect("JSON解析失败");
    
    assert_eq!(json["total_traces"], 5);
    assert!(json["service_ranks"].is_array());
    assert!(json["dependency_graph"].is_array());
}

#[test]
fn test_cli_dot_output() {
    let input_path = get_test_data_path();
    let temp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let output_path = temp_dir.path().join("output.dot");

    let result = run_analysis(&input_path, OutputFormat::Dot, Some(output_path.as_path()));
    assert!(result.is_ok(), "DOT输出失败: {:?}", result.err());
    
    let content = std::fs::read_to_string(&output_path).expect("读取输出文件失败");
    assert!(content.contains("digraph ServiceDependency"));
    assert!(content.contains("->"));
    assert!(content.contains("#ff6b6b"));
}

#[test]
fn test_sample_data_generation() {
    let sample_data = generate_sample_data(3, 4);
    
    assert_eq!(sample_data.data.len(), 3);
    
    for trace in &sample_data.data {
        assert!(!trace.trace_id.is_empty());
        assert!(!trace.spans.is_empty());
        assert!(!trace.processes.is_empty());
    }
    
    let report = generate_report(&sample_data).expect("生成报告失败");
    assert_eq!(report.total_traces, 3);
    assert!(report.total_services <= 4);
}

#[test]
fn test_dependency_graph_construction() {
    let input_path = get_test_data_path();
    let trace = parse_jaeger_json(&input_path).expect("解析失败");
    let traces = parse_traces(&trace).expect("解析追踪失败");
    
    let dep_graph = ServiceDependencyGraph::from_trace_data(&traces).expect("构建依赖图失败");
    
    let services = dep_graph.get_all_services();
    assert!(services.contains(&"api-gateway".to_string()));
    assert!(services.contains(&"order-service".to_string()));
    assert!(services.contains(&"mysql-db".to_string()));
    
    let outgoing = dep_graph.get_outgoing_edges("api-gateway");
    assert!(!outgoing.is_empty(), "api-gateway应该有出边");
    
    let incoming = dep_graph.get_incoming_edges("mysql-db");
    assert!(!incoming.is_empty(), "mysql-db应该有入边");
    
    let edges = dep_graph.to_dependency_edges();
    assert!(!edges.is_empty());
    for edge in &edges {
        assert!(edge.call_count > 0);
        assert!(edge.total_duration > 0);
        assert!(edge.avg_duration > 0.0);
    }
}

#[test]
fn test_critical_path_analysis() {
    let input_path = get_test_data_path();
    let trace = parse_jaeger_json(&input_path).expect("解析失败");
    let traces = parse_traces(&trace).expect("解析追踪失败");
    
    for trace_data in &traces {
        let analysis = analyze_trace(trace_data).expect("分析追踪失败");
        
        assert!(!analysis.critical_path.is_empty());
        
        let mut prev_start = 0u64;
        for node in &analysis.critical_path {
            assert!(node.start_time >= prev_start, "关键路径节点应该按时间排序");
            prev_start = node.start_time;
        }
        
        if let Some(last_node) = analysis.critical_path.last() {
            assert!(last_node.cumulative_duration <= analysis.total_duration, 
                "关键路径累积延迟({})不应超过追踪总延迟({})", 
                last_node.cumulative_duration, analysis.total_duration);
        }
        
        let max_duration = analysis.critical_path.iter()
            .map(|n| n.duration)
            .max()
            .unwrap_or(0);
        assert!(max_duration <= analysis.total_duration, "单个节点延迟不应超过追踪总延迟");
    }
}

#[test]
fn test_bottleneck_ranking() {
    let input_path = get_test_data_path();
    let trace = parse_jaeger_json(&input_path).expect("解析失败");
    let report = generate_report(&trace).expect("生成报告失败");
    
    assert!(!report.service_ranks.is_empty());
    
    for i in 1..report.service_ranks.len() {
        assert!(
            report.service_ranks[i-1].bottleneck_score >= report.service_ranks[i].bottleneck_score,
            "服务应该按瓶颈得分降序排列"
        );
    }
    
    let top_service = &report.service_ranks[0];
    assert!(top_service.bottleneck_score > 0.0);
    assert!(top_service.critical_path_count > 0);
}

#[test]
fn test_output_format_parsing() {
    assert_eq!("text".parse::<OutputFormat>().unwrap(), OutputFormat::Text);
    assert_eq!("json".parse::<OutputFormat>().unwrap(), OutputFormat::Json);
    assert_eq!("dot".parse::<OutputFormat>().unwrap(), OutputFormat::Dot);
    assert_eq!("TEXT".parse::<OutputFormat>().unwrap(), OutputFormat::Text);
    assert_eq!("Json".parse::<OutputFormat>().unwrap(), OutputFormat::Json);
    assert_eq!("DOT".parse::<OutputFormat>().unwrap(), OutputFormat::Dot);
    
    assert!("invalid".parse::<OutputFormat>().is_err());
    assert!("xml".parse::<OutputFormat>().is_err());
    assert!("".parse::<OutputFormat>().is_err());
}

#[test]
fn test_report_summary() {
    let input_path = get_test_data_path();
    let trace = parse_jaeger_json(&input_path).expect("解析失败");
    let report = generate_report(&trace).expect("生成报告失败");
    
    let summary = get_service_analysis_summary(&report);
    assert!(!summary.is_empty());
    assert!(summary.contains("总追踪数: 5"));
    assert!(summary.contains("瓶颈服务排名"));
    assert!(summary.contains("瓶颈得分"));
    assert!(summary.contains("关键路径次数"));
}
