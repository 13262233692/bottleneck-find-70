use crate::critical_path::analyze_all_traces;
use crate::models::{AnalysisReport, TraceData};
use crate::monitor::{Alert, BottleneckDetector, MonitorConfig, TraceBatch, TraceStream, print_alert_console};
use crate::ranking::{aggregate_service_stats, rank_services};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct RealTimeMonitor {
    config: Arc<MonitorConfig>,
    detector: BottleneckDetector,
    trace_batch: TraceBatch,
    alert_sender: Option<mpsc::Sender<Alert>>,
    analysis_count: u64,
    last_analysis_time: Option<chrono::DateTime<chrono::Utc>>,
}

impl RealTimeMonitor {
    pub fn new(config: Arc<MonitorConfig>) -> Self {
        Self {
            config: config.clone(),
            detector: BottleneckDetector::new(config),
            trace_batch: TraceBatch::new(),
            alert_sender: None,
            analysis_count: 0,
            last_analysis_time: None,
        }
    }

    pub fn with_alert_sender(mut self, sender: mpsc::Sender<Alert>) -> Self {
        self.alert_sender = Some(sender);
        self
    }

    pub async fn run(&mut self) -> Result<()> {
        println!("\n");
        println!("╔══════════════════════════════════════════════════════════════╗");
        println!("║          分布式追踪瓶颈实时监控系统启动中...                   ║");
        println!("╚══════════════════════════════════════════════════════════════╝");
        println!();
        println!("📊 分析间隔: {} 秒", self.config.analysis_interval_secs);
        println!("🎯 瓶颈阈值: {:.0}%", self.config.bottleneck_threshold * 100.0);
        println!("📡 Webhook: {}", self.config.webhook_url.as_deref().unwrap_or("未配置"));
        println!("🔄 模拟数据流: {}", if self.config.enable_simulated_stream { "已启用" } else { "已禁用" });
        println!();

        let mut trace_stream = TraceStream::new(self.config.clone());
        let mut trace_receiver = trace_stream.take_receiver()
            .ok_or_else(|| anyhow::anyhow!("无法获取追踪接收器"))?;

        trace_stream.start_streaming().await?;

        let mut interval = tokio::time::interval(self.config.analysis_interval());
        let mut last_display = String::new();

        println!("🚀 监控系统已启动，按 Ctrl+C 停止");
        println!();

        loop {
            tokio::select! {
                Some(message) = trace_receiver.recv() => {
                    self.trace_batch.add(message);
                }

                _ = interval.tick() => {
                    if self.trace_batch.len() >= self.config.min_traces_per_analysis {
                        let alerts = self.analyze_batch().await?;
                        
                        for alert in &alerts {
                            print_alert_console(alert);
                            
                            if let Some(sender) = &self.alert_sender {
                                if let Err(e) = sender.send(alert.clone()).await {
                                    eprintln!("[Monitor] 发送告警失败: {}", e);
                                }
                            }
                        }

                        let status_display = self.get_status_display();
                        if status_display != last_display {
                            println!("\n{}", status_display);
                            last_display = status_display;
                        }
                    } else {
                        println!("[Monitor] 等待更多追踪数据 (当前: {}/{} 条)", 
                            self.trace_batch.len(), self.config.min_traces_per_analysis);
                    }
                }
            }
        }
    }

    async fn analyze_batch(&mut self) -> Result<Vec<Alert>> {
        self.analysis_count += 1;
        self.last_analysis_time = Some(chrono::Utc::now());

        let all_trace_data: Vec<TraceData> = self.trace_batch
            .traces
            .iter()
            .flat_map(|t| t.data.clone())
            .collect();
        
        let trace_analyses = analyze_all_traces(&all_trace_data)?;
        
        let dependency_graph = crate::dependency_graph::ServiceDependencyGraph::from_trace_data(&all_trace_data)?;
        let dependency_edges = dependency_graph.to_dependency_edges();
        
        let service_stats = aggregate_service_stats(&trace_analyses);
        let service_ranks = rank_services(&service_stats, trace_analyses.len());

        let report = AnalysisReport {
            total_traces: trace_analyses.len(),
            total_services: service_ranks.len(),
            service_ranks,
            trace_analyses,
            dependency_graph: dependency_edges,
        };

        self.trace_batch.clear();

        let alerts = self.detector.analyze(&report);

        Ok(alerts)
    }

    fn get_status_display(&self) -> String {
        let mut lines = Vec::new();

        lines.push("━".repeat(60));
        lines.push(format!("分析次数: #{} | 时间: {}", 
            self.analysis_count,
            self.last_analysis_time.map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default()
        ));

        if let Some(top_bottleneck) = self.detector.get_current_top_bottleneck() {
            let score = self.detector.get_service_score(top_bottleneck).unwrap_or(0.0);
            lines.push(format!("🔥 当前瓶颈: {} (得分: {:.2})", top_bottleneck, score));
        } else {
            lines.push("✅ 当前无明显瓶颈".to_string());
        }

        lines.push("━".repeat(60));

        lines.join("\n")
    }
}

pub async fn start_monitoring(config: MonitorConfig) -> Result<()> {
    let config = Arc::new(config);

    let (alert_sender, alert_receiver) = mpsc::channel::<Alert>(100);

    let webhook_config = config.clone();
    tokio::spawn(async move {
        crate::monitor::start_webhook_worker(alert_receiver, webhook_config).await;
    });

    let mut monitor = RealTimeMonitor::new(config)
        .with_alert_sender(alert_sender);

    monitor.run().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_creation() {
        let config = Arc::new(MonitorConfig::default());
        let monitor = RealTimeMonitor::new(config);
        
        assert_eq!(monitor.analysis_count, 0);
        assert!(monitor.last_analysis_time.is_none());
        assert!(monitor.trace_batch.is_empty());
    }



    #[test]
    fn test_monitor_with_alert_sender() {
        let config = Arc::new(MonitorConfig::default());
        let (sender, _receiver) = mpsc::channel::<Alert>(10);
        
        let monitor = RealTimeMonitor::new(config).with_alert_sender(sender);
        
        assert!(monitor.alert_sender.is_some());
    }
}
