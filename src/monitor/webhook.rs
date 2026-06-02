use crate::monitor::{Alert, MonitorConfig};
use anyhow::{Context, Result};
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct WebhookSender {
    client: Client,
    config: Arc<MonitorConfig>,
}

impl WebhookSender {
    pub fn new(config: Arc<MonitorConfig>) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    pub async fn send_alert(&self, alert: &Alert) -> Result<()> {
        let webhook_url = match &self.config.webhook_url {
            Some(url) => url,
            None => {
                println!("[Webhook] 未配置Webhook URL，跳过发送: {}", alert.title);
                return Ok(());
            }
        };

        let mut request = self.client.post(webhook_url);

        if let Some(headers) = &self.config.webhook_headers {
            for (key, value) in headers {
                request = request.header(key, value);
            }
        }

        let payload = serde_json::json!({
            "timestamp": alert.timestamp.to_rfc3339(),
            "alert_type": alert.alert_type.to_string(),
            "severity": alert.severity.to_string(),
            "title": alert.title,
            "message": alert.message,
            "service_name": alert.service_name,
            "previous_bottleneck": alert.previous_bottleneck,
            "new_bottleneck": alert.new_bottleneck,
            "score_change": alert.score_change,
            "metadata": alert.metadata,
        });

        let response = request
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("发送Webhook告警失败到: {}", webhook_url))?;

        if response.status().is_success() {
            println!("[Webhook ✓] 告警发送成功: {}", alert.title);
        } else {
            eprintln!(
                "[Webhook ✗] 告警发送失败: HTTP {}, {}",
                response.status(),
                alert.title
            );
        }

        Ok(())
    }

    pub async fn send_alert_batch(&self, alerts: &[Alert]) -> Result<()> {
        for alert in alerts {
            if let Err(e) = self.send_alert(alert).await {
                eprintln!("发送告警失败: {}", e);
            }
        }
        Ok(())
    }
}

pub async fn start_webhook_worker(
    mut receiver: mpsc::Receiver<Alert>,
    config: Arc<MonitorConfig>,
) {
    let sender = WebhookSender::new(config);

    while let Some(alert) = receiver.recv().await {
        if let Err(e) = sender.send_alert(&alert).await {
            eprintln!("[Webhook Worker] 发送告警错误: {}", e);
        }
    }
}

pub fn print_alert_console(alert: &Alert) {
    let emoji = match alert.severity {
        crate::monitor::AlertSeverity::Info => "ℹ️",
        crate::monitor::AlertSeverity::Warning => "⚠️",
        crate::monitor::AlertSeverity::Critical => "🔴",
    };

    let timestamp = alert.timestamp.format("%Y-%m-%d %H:%M:%S");

    println!(
        "\n{} [{}] [{}] {}",
        emoji, timestamp, alert.alert_type, alert.title
    );
    println!("   {}", alert.message);

    if let Some(service) = &alert.service_name {
        println!("   服务: {}", service);
    }

    if let (Some(prev), Some(new)) = (&alert.previous_bottleneck, &alert.new_bottleneck) {
        println!("   瓶颈转移: {} → {}", prev, new);
    }

    if let Some(change) = alert.score_change {
        println!("   得分变化: {:.2}%", change * 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{AlertType, AlertSeverity};

    #[test]
    fn test_alert_creation() {
        let alert = Alert::new(
            AlertType::NewBottleneck,
            "新瓶颈检测".to_string(),
            "检测到新的瓶颈服务".to_string(),
        )
        .with_severity(AlertSeverity::Warning)
        .with_service("order-service".to_string());

        assert_eq!(alert.title, "新瓶颈检测");
        assert_eq!(alert.service_name, Some("order-service".to_string()));
    }

    #[test]
    fn test_alert_with_shift_info() {
        let alert = Alert::new(
            AlertType::BottleneckShift,
            "瓶颈转移".to_string(),
            "主要瓶颈从A转移到B".to_string(),
        )
        .with_shift_info("service-a".to_string(), "service-b".to_string(), 0.25);

        assert_eq!(alert.previous_bottleneck, Some("service-a".to_string()));
        assert_eq!(alert.new_bottleneck, Some("service-b".to_string()));
        assert_eq!(alert.score_change, Some(0.25));
    }

    #[test]
    fn test_alert_severity_display() {
        assert_eq!(AlertSeverity::Critical.to_string(), "CRITICAL");
        assert_eq!(AlertSeverity::Warning.to_string(), "WARNING");
        assert_eq!(AlertSeverity::Info.to_string(), "INFO");
    }

    #[test]
    fn test_alert_type_display() {
        assert_eq!(AlertType::BottleneckShift.to_string(), "BOTTLENECK_SHIFT");
        assert_eq!(AlertType::NewBottleneck.to_string(), "NEW_BOTTLENECK");
        assert_eq!(AlertType::LatencySpike.to_string(), "LATENCY_SPIKE");
    }
}
