use crate::models::AnalysisReport;
use crate::monitor::{Alert, AlertType, AlertSeverity, MonitorConfig};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

pub struct BottleneckHistory {
    pub service_scores: HashMap<String, ServiceScoreHistory>,
    pub top_bottleneck: Option<String>,
    pub history: VecDeque<BottleneckSnapshot>,
    max_history: usize,
}

#[derive(Debug, Clone)]
pub struct ServiceScoreHistory {
    pub current_score: f64,
    pub score_history: VecDeque<f64>,
    pub is_bottleneck_count: usize,
    pub last_seen: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct BottleneckSnapshot {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub top_bottleneck: String,
    pub top_score: f64,
    pub all_scores: HashMap<String, f64>,
}

impl Default for BottleneckHistory {
    fn default() -> Self {
        Self::new(20)
    }
}

impl BottleneckHistory {
    pub fn new(max_history: usize) -> Self {
        Self {
            service_scores: HashMap::new(),
            top_bottleneck: None,
            history: VecDeque::with_capacity(max_history),
            max_history,
        }
    }

    pub fn update(&mut self, report: &AnalysisReport) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let timestamp = chrono::Utc::now();

        let current_top = report.service_ranks.first();

        let current_top_name = current_top.map(|r| r.service_name.clone());
        let current_top_score = current_top.map(|r| r.bottleneck_score).unwrap_or(0.0);

        let mut all_scores = HashMap::new();
        for rank in &report.service_ranks {
            all_scores.insert(rank.service_name.clone(), rank.bottleneck_score);

            let history = self
                .service_scores
                .entry(rank.service_name.clone())
                .or_insert_with(|| ServiceScoreHistory {
                    current_score: 0.0,
                    score_history: VecDeque::with_capacity(self.max_history),
                    is_bottleneck_count: 0,
                    last_seen: timestamp,
                });

            let _previous_score = history.current_score;
            history.current_score = rank.bottleneck_score;
            history.last_seen = timestamp;
            history.score_history.push_front(rank.bottleneck_score);
            if history.score_history.len() > self.max_history {
                history.score_history.pop_back();
            }
        }

        if let Some(top) = &report.service_ranks.first() {
            if top.bottleneck_score > 0.0 {
                if let Some(history) = self.service_scores.get_mut(&top.service_name) {
                    history.is_bottleneck_count += 1;
                }
            }
        }

        let snapshot = BottleneckSnapshot {
            timestamp,
            top_bottleneck: current_top_name.clone().unwrap_or_default(),
            top_score: current_top_score,
            all_scores: all_scores.clone(),
        };

        self.history.push_front(snapshot);
        if self.history.len() > self.max_history {
            self.history.pop_back();
        }

        if let (Some(prev_top), Some(curr_top)) = (&self.top_bottleneck, &current_top_name) {
            if prev_top != curr_top {
                let prev_score = self.service_scores.get(prev_top).map(|h| h.current_score).unwrap_or(0.0);
                let score_change = (current_top_score - prev_score).abs();

                if score_change > 0.1 {
                    let alert = Alert::new(
                        AlertType::BottleneckShift,
                        format!("瓶颈转移检测: {} → {}", prev_top, curr_top),
                        format!(
                            "主要瓶颈从 {} 转移到 {}，得分变化 {:.1}%",
                            prev_top,
                            curr_top,
                            score_change * 100.0
                        ),
                    )
                    .with_severity(AlertSeverity::Warning)
                    .with_shift_info(prev_top.clone(), curr_top.clone(), score_change);

                    alerts.push(alert);
                }
            }
        }

        if self.top_bottleneck.is_none() && current_top_name.is_some() {
            if let Some(top_name) = &current_top_name {
                if current_top_score > 0.5 {
                    let alert = Alert::new(
                        AlertType::NewBottleneck,
                        format!("新瓶颈检测: {}", top_name),
                        format!(
                            "检测到新的主要瓶颈服务: {}，得分 {:.2}",
                            top_name, current_top_score
                        ),
                    )
                    .with_severity(AlertSeverity::Warning)
                    .with_service(top_name.clone());

                    alerts.push(alert);
                }
            }
        }

        for rank in &report.service_ranks {
            if let Some(history) = self.service_scores.get(&rank.service_name) {
                if history.score_history.len() >= 3 {
                    let avg: f64 = history.score_history.iter().skip(1).take(2).sum::<f64>() / 2.0;
                    let spike = (rank.bottleneck_score - avg) / avg.max(0.01);

                    if spike > 0.5 && rank.bottleneck_score > 0.3 {
                        let alert = Alert::new(
                            AlertType::LatencySpike,
                            format!("延迟突增: {}", rank.service_name),
                            format!(
                                "服务 {} 延迟突增 {:.1}%，当前得分 {:.2}",
                                rank.service_name,
                                spike * 100.0,
                                rank.bottleneck_score
                            ),
                        )
                        .with_severity(AlertSeverity::Critical)
                        .with_service(rank.service_name.clone());

                        alerts.push(alert);
                    }
                }
            }
        }

        self.top_bottleneck = current_top_name;

        alerts
    }

    pub fn get_trend(&self, service: &str) -> Option<f64> {
        let history = self.service_scores.get(service)?;
        if history.score_history.len() < 2 {
            return None;
        }

        let recent = history.score_history.front()?;
        let older = history.score_history.back()?;

        Some(recent - older)
    }

    pub fn get_average_score(&self, service: &str, window: usize) -> Option<f64> {
        let history = self.service_scores.get(service)?;
        let scores: Vec<f64> = history.score_history.iter().take(window).copied().collect();

        if scores.is_empty() {
            None
        } else {
            Some(scores.iter().sum::<f64>() / scores.len() as f64)
        }
    }
}

pub struct BottleneckDetector {
    history: BottleneckHistory,
    config: Arc<MonitorConfig>,
}

impl BottleneckDetector {
    pub fn new(config: Arc<MonitorConfig>) -> Self {
        Self {
            history: BottleneckHistory::new(config.max_bottleneck_history),
            config,
        }
    }

    pub fn analyze(&mut self, report: &AnalysisReport) -> Vec<Alert> {
        self.history.update(report)
    }

    pub fn get_current_top_bottleneck(&self) -> Option<&String> {
        self.history.top_bottleneck.as_ref()
    }

    pub fn get_service_score(&self, service: &str) -> Option<f64> {
        self.history
            .service_scores
            .get(service)
            .map(|h| h.current_score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AnalysisReport, ServiceBottleneckRank};

    fn create_test_report(top_service: &str, score: f64) -> AnalysisReport {
        AnalysisReport {
            total_traces: 10,
            total_services: 5,
            service_ranks: vec![
                ServiceBottleneckRank {
                    service_name: top_service.to_string(),
                    total_contribution: 1000,
                    appearance_count: 10,
                    avg_contribution: 100.0,
                    bottleneck_score: score,
                    critical_path_count: 10,
                },
                ServiceBottleneckRank {
                    service_name: "other-service".to_string(),
                    total_contribution: 500,
                    appearance_count: 10,
                    avg_contribution: 50.0,
                    bottleneck_score: 0.3,
                    critical_path_count: 5,
                },
            ],
            trace_analyses: vec![],
            dependency_graph: vec![],
        }
    }

    #[test]
    fn test_bottleneck_history_update() {
        let mut history = BottleneckHistory::new(10);
        let report = create_test_report("service-a", 0.8);
        
        let alerts = history.update(&report);
        
        assert!(!alerts.is_empty());
        assert_eq!(history.top_bottleneck, Some("service-a".to_string()));
    }

    #[test]
    fn test_bottleneck_shift_detection() {
        let mut history = BottleneckHistory::new(10);
        
        let report1 = create_test_report("service-a", 0.8);
        history.update(&report1);
        
        let report2 = create_test_report("service-b", 0.95);
        let alerts = history.update(&report2);
        
        let shift_alert = alerts.iter().find(|a| matches!(a.alert_type, AlertType::BottleneckShift));
        assert!(shift_alert.is_some());
        assert_eq!(shift_alert.unwrap().previous_bottleneck, Some("service-a".to_string()));
        assert_eq!(shift_alert.unwrap().new_bottleneck, Some("service-b".to_string()));
    }

    #[test]
    fn test_new_bottleneck_detection() {
        let mut history = BottleneckHistory::new(10);
        
        let report = create_test_report("service-a", 0.6);
        let alerts = history.update(&report);
        
        let new_alert = alerts.iter().find(|a| matches!(a.alert_type, AlertType::NewBottleneck));
        assert!(new_alert.is_some());
        assert_eq!(new_alert.unwrap().service_name, Some("service-a".to_string()));
    }

    #[test]
    fn test_service_score_history() {
        let mut history = BottleneckHistory::new(10);
        
        let report = create_test_report("service-a", 0.7);
        history.update(&report);
        
        assert!(history.service_scores.contains_key("service-a"));
        assert_eq!(history.service_scores.get("service-a").unwrap().current_score, 0.7);
    }
}
