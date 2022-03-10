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
                    name: ts.name(),
                    status,
                    services: active_backends,
                }
            })
        })
        .collect();
    Ok(statuses)
}

pub fn print_status(results: Vec<TrafficSplitStatus>) {
    println!(
        "{:15}\t{:15}\t{:10}\tACTIVE BACKENDS",
        "NAMESPACE", "TRAFFIC_SPLIT", "STATUS"
    );
    for result in results.iter() {
        println!(
            "{:15}\t{:15}\t{:10}\t{}",
            result.namespace,
            result.name,
            result.status,
            result.services.join(", ")
        );
    }
}

pub fn json_print_status(results: Vec<TrafficSplitStatus>) {
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
