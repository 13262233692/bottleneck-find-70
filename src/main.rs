use anyhow::Result;
use clap::{Parser, Subcommand};
use bottleneck_find::{
    generate_report, generate_sample_data, parse_jaeger_json, run_analysis,
    format_dot_graph, OutputFormat, MonitorConfig, start_monitoring,
};
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "bottleneck-find",
    version = "0.1.0",
    about = "分布式追踪瓶颈分析工具 - 分析Jaeger追踪数据，自动构建服务依赖图，识别性能瓶颈",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(about = "分析Jaeger JSON追踪文件，生成瓶颈分析报告")]
    Analyze {
        #[arg(short, long, value_name = "FILE", help = "Jaeger JSON格式的追踪数据文件")]
        input: PathBuf,

        #[arg(
            short,
            long,
            value_name = "FORMAT",
            default_value = "text",
            help = "输出格式: text, json, dot"
        )]
        format: String,

        #[arg(short, long, value_name = "FILE", help = "输出文件路径，不指定则输出到控制台")]
        output: Option<PathBuf>,
    },

    #[command(about = "生成模拟的Jaeger追踪数据用于测试")]
    Generate {
        #[arg(short, long, default_value_t = 10, value_name = "N", help = "生成的追踪数量")]
        traces: usize,

        #[arg(short, long, default_value_t = 5, value_name = "N", help = "模拟的服务数量")]
        services: usize,

        #[arg(short, long, value_name = "FILE", help = "输出文件路径")]
        output: PathBuf,
    },

    #[command(about = "生成服务依赖图的DOT格式文件")]
    Graph {
        #[arg(short, long, value_name = "FILE", help = "Jaeger JSON格式的追踪数据文件")]
        input: PathBuf,

        #[arg(short, long, value_name = "FILE", help = "输出DOT文件路径")]
        output: PathBuf,

        #[arg(long, default_value_t = true, help = "高亮显示瓶颈服务")]
        highlight: bool,
    },

    #[command(about = "列出追踪文件中的所有服务")]
    ListServices {
        #[arg(short, long, value_name = "FILE", help = "Jaeger JSON格式的追踪数据文件")]
        input: PathBuf,
    },

    #[command(about = "显示追踪文件的统计信息")]
    Stats {
        #[arg(short, long, value_name = "FILE", help = "Jaeger JSON格式的追踪数据文件")]
        input: PathBuf,
    },

    #[command(about = "启动实时监控模式，持续分析追踪数据并检测瓶颈转移")]
    Monitor {
        #[arg(short, long, default_value = "http://localhost:16686", help = "Jaeger gRPC端点")]
        jaeger: String,

        #[arg(short, long, help = "Webhook URL，用于发送告警通知")]
        webhook: Option<String>,

        #[arg(short, long, default_value_t = 10, value_name = "SECONDS", help = "分析间隔（秒）")]
        interval: u64,

        #[arg(short, long, default_value_t = 0.5, value_name = "FLOAT", help = "瓶颈阈值 (0.0-1.0)")]
        threshold: f64,

        #[arg(long, default_value_t = 5, help = "每次分析的最小追踪数")]
        min_traces: usize,

        #[arg(long, default_value_t = 8, help = "模拟服务数量")]
        simulated_services: usize,

        #[arg(long, default_value_t = 2.0, help = "每秒生成的模拟追踪数")]
        traces_per_second: f64,

        #[arg(long, default_value_t = true, help = "启用模拟数据流")]
        simulated: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Analyze { input, format, output } => {
            let output_format: OutputFormat = format
                .parse()
                .map_err(|e: String| anyhow::anyhow!(e))?;

            println!("正在分析追踪数据: {:?}", input);
            println!("输出格式: {:?}", output_format);

            run_analysis(&input, output_format, output.as_deref())?;

            println!("\n分析完成!");
        }

        Commands::Generate { traces, services, output } => {
            println!("正在生成模拟数据...");
            println!("  追踪数量: {}", traces);
            println!("  服务数量: {}", services);

            let sample_data = generate_sample_data(traces, services);
            let json = serde_json::to_string_pretty(&sample_data)?;

            fs::write(&output, json)?;
            println!("模拟数据已生成: {:?}", output);
        }

        Commands::Graph { input, output, highlight } => {
            println!("正在生成服务依赖图...");
            
            let trace = parse_jaeger_json(&input)?;
            let report = generate_report(&trace)?;
            let dot_content = format_dot_graph(&report, highlight);

            fs::write(&output, dot_content)?;
            println!("DOT图已生成: {:?}", output);
            println!("使用以下命令渲染为图片: dot -Tpng {} -o graph.png", output.display());
        }

        Commands::ListServices { input } => {
            let trace = parse_jaeger_json(&input)?;
            let report = generate_report(&trace)?;

            println!("追踪文件中的服务列表 (共{}个服务):", report.total_services);
            println!("{}", "─".repeat(50));
            for (i, rank) in report.service_ranks.iter().enumerate() {
                println!(
                    "{:2}. {:<30} 出现次数: {:<5} 关键路径次数: {}",
                    i + 1,
                    rank.service_name,
                    rank.appearance_count,
                    rank.critical_path_count
                );
            }
        }

        Commands::Stats { input } => {
            let trace = parse_jaeger_json(&input)?;
            let report = generate_report(&trace)?;

            println!("=== 追踪数据统计信息 ===");
            println!("总追踪数: {}", report.total_traces);
            println!("总服务数: {}", report.total_services);
            println!();

            let total_spans: usize = report
                .trace_analyses
                .iter()
                .map(|t| t.critical_path.len())
                .sum();
            println!("总关键路径Span数: {}", total_spans);

            let avg_duration: f64 = if report.total_traces > 0 {
                report.trace_analyses.iter()
                    .map(|t| t.total_duration as f64)
                    .sum::<f64>()
                    / report.total_traces as f64
            } else {
                0.0
            };
            println!("平均追踪延迟: {:.2}μs", avg_duration);

            if let Some(max_duration) = report
                .trace_analyses
                .iter()
                .map(|t| t.total_duration)
                .max()
            {
                println!("最大追踪延迟: {}μs", max_duration);
            }

            if let Some(min_duration) = report
                .trace_analyses
                .iter()
                .map(|t| t.total_duration)
                .min()
            {
                println!("最小追踪延迟: {}μs", min_duration);
            }
            println!();

            println!("=== Top 3 瓶颈服务 ===");
            for (i, rank) in report.service_ranks.iter().take(3).enumerate() {
                println!(
                    "{}. {} - 瓶颈得分: {:.4}, 总贡献: {}μs",
                    i + 1,
                    rank.service_name,
                    rank.bottleneck_score,
                    rank.total_contribution
                );
            }
            println!();

            println!("=== 依赖图统计 ===");
            println!("服务调用关系数: {}", report.dependency_graph.len());
            
            let total_calls: usize = report
                .dependency_graph
                .iter()
                .map(|e| e.call_count)
                .sum();
            println!("总调用次数: {}", total_calls);

            let avg_call_duration: f64 = if total_calls > 0 {
                report.dependency_graph.iter()
                    .map(|e| e.avg_duration * e.call_count as f64)
                    .sum::<f64>()
                    / total_calls as f64
            } else {
                0.0
            };
            println!("平均调用延迟: {:.2}μs", avg_call_duration);
        }

        Commands::Monitor { 
            jaeger, 
            webhook, 
            interval, 
            threshold, 
            min_traces,
            simulated_services,
            traces_per_second,
            simulated,
        } => {
            let config = MonitorConfig {
                jaeger_endpoint: jaeger,
                webhook_url: webhook,
                webhook_headers: None,
                analysis_interval_secs: interval,
                bottleneck_threshold: threshold,
                shift_detection_threshold: 0.3,
                min_traces_per_analysis: min_traces,
                max_bottleneck_history: 20,
                enable_simulated_stream: simulated,
                simulated_services,
                simulated_traces_per_second: traces_per_second,
            };

            start_monitoring(config).await?;
        }
    }

    Ok(())
}
