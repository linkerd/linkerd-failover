use crate::table::{Column, Table};
use anyhow::Result;
use kube::{api::ListParams, Api, Client, ResourceExt};
use linkerd_failover_controller::TrafficSplit;
use serde::Serialize;
use std::fmt::Display;

#[derive(Serialize)]
pub struct TrafficSplitStatus {
    namespace: String,
    name: String,
    status: FailoverStatus,
    services: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum FailoverStatus {
    Primary,
    Fallback,
}

impl Display for FailoverStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Primary => std::fmt::Display::fmt("Primary", f)?,
            Self::Fallback => std::fmt::Display::fmt("Fallback", f)?,
        };
        Ok(())
    }
}

pub async fn status(client: Client, label_selector: &str) -> Result<Vec<TrafficSplitStatus>> {
    let api = Api::<TrafficSplit>::all(client);
    let list_params = ListParams::default().labels(label_selector);
    let traffic_splits = api.list(&list_params).await?;
    let statuses = traffic_splits
        .items
        .into_iter()
        .flat_map(|ts| {
            let primary = ts
                .metadata
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get("failover.linkerd.io/primary-service"));
            primary.map(|primary| {
                let active_backends = active_backends(&ts);
                let status = if active_backends.contains(primary) {
                    FailoverStatus::Primary
                } else {
                    FailoverStatus::Fallback
                };
                TrafficSplitStatus {
                    namespace: ts.namespace().expect("TrafficSplits must be namespaced"),
                    name: ts.name_any(),
                    status,
                    services: active_backends,
                }
            })
        })
        .collect();
    Ok(statuses)
}

pub fn print_status(results: &[TrafficSplitStatus]) {
    let columns: Vec<Column<TrafficSplitStatus>> = vec![
        Column::new("NAMESPACE", Box::new(|r| r.namespace.clone())),
        Column::new("TRAFFIC_SPLIT", Box::new(|r| r.name.clone())),
        Column::new("STATUS", Box::new(|r| r.status.to_string())),
        Column::new("ACTIVE_BACKENDS", Box::new(|r| r.services.join(", "))),
    ];
    let table = Table {
        cols: columns,
        data: results,
    };
    print!("{table}");
}

pub fn json_print_status(results: &[TrafficSplitStatus]) {
    serde_json::to_writer_pretty(std::io::stdout(), &results).expect("serialization failed");
    println!();
}

fn active_backends(ts: &TrafficSplit) -> Vec<String> {
    ts.spec
        .backends
        .iter()
        .filter_map(|backend| {
            if backend.weight > 0 {
                Some(backend.service.clone())
            } else {
                None
            }
        })
        .collect()
}
