use anyhow::Result;
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

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
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

impl Default for CheckStatus {
    fn default() -> CheckStatus {
        CheckStatus::Success
    }
}

impl CheckResult {
    fn new(description: &'static str) -> CheckResult {
        CheckResult {
            description,
            ..Default::default()
        }
    }
}

pub async fn check(client: Client, pre: bool) -> Vec<CheckResult> {
    let mut checks: Vec<CheckResult> = vec![];

    let api = Api::<CustomResourceDefinition>::all(client.clone());
    let mut traffic_split_check = CheckResult::new("TrafficSplit CRD exists");
    match api.get_opt("trafficsplits.split.smi-spec.io").await {
        Result::Ok(Some(_)) => traffic_split_check.result = CheckStatus::Success,
        Result::Ok(None) => {
            traffic_split_check.result = CheckStatus::Error;
            traffic_split_check.error = Some("TrafficSplit CRD is not installed".into());
            traffic_split_check.hint = Some("https://github.com/linkerd/linkerd-smi");
        }
        Result::Err(err) => {
            traffic_split_check.result = CheckStatus::Error;
            traffic_split_check.error = Some(err.to_string());
            traffic_split_check.hint =
                Some("https://github.com/linkerd/linkerd-failover#troubleshooting");
        }
    };
    checks.push(traffic_split_check);

    if pre {
        return checks;
    }

    let mut namespace_check = CheckResult::new("failover extension namespace exists");
    let extension_label = ListParams::default().labels("linkerd.io/extension=failover");
    let api = Api::<Namespace>::all(client.clone());
    let ns_list = api.list(&extension_label).await;
    let ns = match ns_list {
        Result::Ok(ref objs) if objs.items.len() == 1 => {
            namespace_check.result = CheckStatus::Success;
            checks.push(namespace_check);
            let ns = objs.items.first().expect("failover namespace must exist");
            ns
        }
        Result::Ok(ref objs) if objs.items.is_empty() => {
            namespace_check.result = CheckStatus::Error;
            namespace_check.error = Some("Failover namespace not found".into());
            namespace_check.hint =
                Some("https://github.com/linkerd/linkerd-failover#troubleshooting");
            checks.push(namespace_check);
            return checks;
        }
        Result::Ok(_) => {
            namespace_check.result = CheckStatus::Error;
            namespace_check.error = Some("Multiple failover namespaces found".into());
            namespace_check.hint =
                Some("https://github.com/linkerd/linkerd-failover#troubleshooting");
            checks.push(namespace_check);
            return checks;
        }
        Result::Err(err) => {
            namespace_check.result = CheckStatus::Error;
            namespace_check.error = Some(err.to_string());
            namespace_check.hint =
                Some("https://github.com/linkerd/linkerd-failover#troubleshooting");
            checks.push(namespace_check);
            return checks;
        }
    };

    let mut deploy_check = CheckResult::new("failover controller is healthy");
    let api = Api::<Deployment>::namespaced(client, ns.name().as_str());
    match api.get_opt("linkerd-failover").await {
        Result::Ok(Some(deploy)) => {
            if deploy.status.map_or(false, |status| {
                status
                    .available_replicas
                    .map_or(false, |replicas| replicas > 0)
            }) {
                deploy_check.result = CheckStatus::Success;
            } else {
                deploy_check.result = CheckStatus::Error;
                deploy_check.error =
                    Some("linkerd-failover deployment has no available replicas".into());
                deploy_check.hint =
                    Some("https://github.com/linkerd/linkerd-failover#troubleshooting");
            }
        }
        Result::Ok(None) => {
            deploy_check.result = CheckStatus::Error;
            deploy_check.error = Some("linkerd-failover deployment not found".into());
            deploy_check.hint = Some("https://github.com/linkerd/linkerd-failover#troubleshooting");
        }
        Result::Err(err) => {
            deploy_check.result = CheckStatus::Error;
            deploy_check.error = Some(err.to_string());
            deploy_check.hint = Some("https://github.com/linkerd/linkerd-failover#troubleshooting");
        }
    };
    checks.push(deploy_check);

    checks
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
