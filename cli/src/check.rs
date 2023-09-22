use k8s_openapi::{
    api::{apps::v1::Deployment, core::v1::Namespace},
    apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition,
};
use kube::{api::ListParams, Api, Client, ResourceExt};
use serde::Serialize;

const CHECK: &str = "√";
const EX: &str = "×";

#[derive(Serialize)]
struct CheckOutput {
    success: bool,
    categories: Vec<Category>,
}

#[derive(Serialize)]
struct Category {
    category_name: &'static str,
    checks: Vec<CheckResult>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
    #[default]
    Success,
    Error,
}

#[derive(Serialize, Default)]
pub struct CheckResult {
    description: &'static str,
    result: CheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl CheckResult {
    pub fn success(&self) -> bool {
        matches!(self.result, CheckStatus::Success)
    }
}

pub async fn traffic_split_check(client: Client) -> CheckResult {
    let api = Api::<CustomResourceDefinition>::all(client);
    let description = "TrafficSplit CRD exists";
    match api.get_opt("trafficsplits.split.smi-spec.io").await {
        Ok(Some(_)) => CheckResult {
            description,
            result: CheckStatus::Success,
            ..Default::default()
        },
        Ok(None) => CheckResult {
            description,
            result: CheckStatus::Error,
            error: Some("TrafficSplit CRD is not installed".into()),
            hint: Some("https://github.com/linkerd/linkerd-smi"),
        },
        Err(err) => CheckResult {
            description,
            result: CheckStatus::Error,
            error: Some(err.to_string()),
            hint: Some("https://github.com/linkerd/linkerd-failover#troubleshooting"),
        },
    }
}

pub async fn namespace_check(client: Client) -> (CheckResult, Option<String>) {
    let api = Api::<Namespace>::all(client);
    let description = "failover extension namespace exists";
    let extension_label = ListParams::default().labels("linkerd.io/extension=failover");
    let ns_list = api.list(&extension_label).await;
    match ns_list {
        Ok(ref objs) if objs.items.len() == 1 => {
            let ns = objs.items.first().expect("failover namespace must exist");
            (
                CheckResult {
                    description,
                    result: CheckStatus::Success,
                    ..Default::default()
                },
                Some(ns.name_any()),
            )
        }
        Ok(ref objs) if objs.items.is_empty() => (
            CheckResult {
                description,
                result: CheckStatus::Error,
                error: Some("Failover namespace not found".into()),
                hint: Some("https://github.com/linkerd/linkerd-failover#troubleshooting"),
            },
            None,
        ),
        Ok(_) => (
            CheckResult {
                description,
                result: CheckStatus::Error,
                error: Some("Multiple failover namespaces found".into()),
                hint: Some("https://github.com/linkerd/linkerd-failover#troubleshooting"),
            },
            None,
        ),
        Err(err) => (
            CheckResult {
                description,
                result: CheckStatus::Error,
                error: Some(err.to_string()),
                hint: Some("https://github.com/linkerd/linkerd-failover#troubleshooting"),
            },
            None,
        ),
    }
}

pub async fn deploy_check(client: Client, ns: &str) -> CheckResult {
    let api = Api::<Deployment>::namespaced(client, ns);
    let description = "failover controller is healthy";
    match api.get_opt("linkerd-failover").await {
        Ok(Some(deploy)) => {
            let has_available_replicas = deploy.status.map_or(false, |status| {
                status
                    .available_replicas
                    .map_or(false, |replicas| replicas > 0)
            });

            if has_available_replicas {
                CheckResult {
                    description,
                    result: CheckStatus::Success,
                    ..Default::default()
                }
            } else {
                CheckResult {
                    description,
                    result: CheckStatus::Error,
                    error: Some("linkerd-failover deployment has no available replicas".into()),
                    hint: Some("https://github.com/linkerd/linkerd-failover#troubleshooting"),
                }
            }
        }
        Ok(None) => CheckResult {
            description,
            result: CheckStatus::Error,
            error: Some("linkerd-failover deployment not found".into()),
            hint: Some("https://github.com/linkerd/linkerd-failover#troubleshooting"),
        },
        Err(err) => CheckResult {
            description,
            result: CheckStatus::Error,
            error: Some(err.to_string()),
            hint: Some("https://github.com/linkerd/linkerd-failover#troubleshooting"),
        },
    }
}

pub async fn run_checks(client: Client, pre: bool) -> Vec<CheckResult> {
    let mut results = Vec::new();
    results.push(traffic_split_check(client.clone()).await);
    if pre {
        return results;
    }
    let (result, ns) = namespace_check(client.clone()).await;
    results.push(result);

    if let Some(ns) = ns {
        results.push(deploy_check(client, &ns).await);
    }

    results
}

pub fn print_checks(results: Vec<CheckResult>) -> bool {
    let mut success = true;
    let category = "linkerd-failover";
    println!("{}", category);
    println!("{}", category.chars().map(|_| '-').collect::<String>());
    for result in results {
        match result.result {
            CheckStatus::Success => {
                println!("{} {}", CHECK, result.description);
            }
            CheckStatus::Error => {
                success = false;
                println!("{} {}", EX, result.description);
                if let Some(error) = result.error {
                    println!("    {}", error);
                }
                if let Some(hint) = result.hint {
                    println!("    see {} for hints", hint);
                }
            }
        }
    }

    println!();
    let success_symbol = if success { CHECK } else { EX };
    println!("Status check results are {}", success_symbol);
    success
}

pub fn json_print_checks(results: Vec<CheckResult>) -> bool {
    let success = results
        .iter()
        .all(|r| matches!(r.result, CheckStatus::Success));
    let output = CheckOutput {
        success,
        categories: vec![Category {
            category_name: "linkerd-failover",
            checks: results,
        }],
    };
    serde_json::to_writer_pretty(std::io::stdout(), &output).expect("serialization failed");
    println!();
    success
}
