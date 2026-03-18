//! Notifications service — in-memory notification store exposed via JSON-RPC.
//!
//! Methods:
//!   notifications.list        {}                                          -> Vec<Notification>
//!   notifications.add         { title, body, app_id?, icon?, urgency? }   -> Notification
//!   notifications.dismiss     { id }                                      -> ()
//!   notifications.dismiss_all {}                                          -> ()
//!   notifications.action      { id, action_id }                           -> ()

use std::sync::{Arc, Mutex};

use chrono::Utc;
use yantrik_ipc_contracts::email::ServiceError;
use yantrik_ipc_contracts::notifications::*;
use yantrik_ipc_transport::server::{RpcServer, ServiceHandler};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("notifications_service=info".parse().unwrap()),
        )
        .init();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    rt.block_on(async {
        let handler = Arc::new(NotificationsHandler::new());
        let addr = RpcServer::default_address("notifications");
        let server = RpcServer::new(&addr);
        tracing::info!("Starting notifications service");
        if let Err(e) = server.serve(handler).await {
            tracing::error!(error = %e, "Notifications service failed");
        }
    });
}

struct NotificationsHandler {
    store: Mutex<Vec<Notification>>,
}

impl NotificationsHandler {
    fn new() -> Self {
        Self {
            store: Mutex::new(Vec::new()),
        }
    }
}

impl ServiceHandler for NotificationsHandler {
    fn service_id(&self) -> &str {
        "notifications"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "notifications.list" => {
                let store = self.store.lock().unwrap();
                Ok(serde_json::to_value(store.as_slice()).unwrap())
            }
            "notifications.add" => {
                let title = params["title"]
                    .as_str()
                    .ok_or_else(|| ServiceError {
                        code: -32602,
                        message: "Missing 'title' parameter".to_string(),
                    })?
                    .to_string();

                let body = params["body"]
                    .as_str()
                    .ok_or_else(|| ServiceError {
                        code: -32602,
                        message: "Missing 'body' parameter".to_string(),
                    })?
                    .to_string();

                let app_id = params["app_id"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();

                let icon = params["icon"].as_str().map(|s| s.to_string());

                let urgency = match params["urgency"].as_str() {
                    Some("low") | Some("Low") => Urgency::Low,
                    Some("critical") | Some("Critical") => Urgency::Critical,
                    _ => Urgency::Normal,
                };

                let notification = Notification {
                    id: uuid7::uuid7().to_string(),
                    title,
                    body,
                    icon,
                    urgency,
                    source_app: app_id,
                    timestamp: Utc::now().to_rfc3339(),
                    actions: Vec::new(),
                };

                let mut store = self.store.lock().unwrap();
                store.push(notification.clone());
                Ok(serde_json::to_value(notification).unwrap())
            }
            "notifications.dismiss" => {
                let id = params["id"]
                    .as_str()
                    .ok_or_else(|| ServiceError {
                        code: -32602,
                        message: "Missing 'id' parameter".to_string(),
                    })?;

                let mut store = self.store.lock().unwrap();
                store.retain(|n| n.id != id);
                Ok(serde_json::Value::Null)
            }
            "notifications.dismiss_all" => {
                let mut store = self.store.lock().unwrap();
                store.clear();
                Ok(serde_json::Value::Null)
            }
            "notifications.action" => {
                let id = params["id"]
                    .as_str()
                    .ok_or_else(|| ServiceError {
                        code: -32602,
                        message: "Missing 'id' parameter".to_string(),
                    })?;

                let action_id = params["action_id"]
                    .as_str()
                    .ok_or_else(|| ServiceError {
                        code: -32602,
                        message: "Missing 'action_id' parameter".to_string(),
                    })?;

                let store = self.store.lock().unwrap();
                let notification = store.iter().find(|n| n.id == id);

                match notification {
                    Some(n) => {
                        if n.actions.iter().any(|a| a.id == action_id) {
                            tracing::info!(
                                notification_id = id,
                                action_id = action_id,
                                "Action triggered"
                            );
                            Ok(serde_json::Value::Null)
                        } else {
                            Err(ServiceError {
                                code: -32602,
                                message: format!("Action '{action_id}' not found on notification '{id}'"),
                            })
                        }
                    }
                    None => Err(ServiceError {
                        code: -32602,
                        message: format!("Notification '{id}' not found"),
                    }),
                }
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method}"),
            }),
        }
    }
}
