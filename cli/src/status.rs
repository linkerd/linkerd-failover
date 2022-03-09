use kube::Client;
use serde::Serialize;

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

pub async fn status(client: Client) -> Vec<TrafficSplitStatus> {
    unimplemented!()
}

pub fn print_status(results: Vec<TrafficSplitStatus>) -> bool {
    unimplemented!()
}


pub fn json_print_status(results: Vec<TrafficSplitStatus>) -> bool {
    unimplemented!()
}