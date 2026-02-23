use reqwest::dns::{Name, Resolve, Resolving};
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Clone)]
struct RejectResolver;

impl Resolve for RejectResolver {
    fn resolve(&self, _name: Name) -> Resolving {
        Box::pin(async move {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "DNS rejected",
            )) as Box<dyn std::error::Error + Send + Sync>)
        })
    }
}

#[tokio::main]
async fn main() {
    let client = reqwest::Client::builder()
        .dns_resolver(std::sync::Arc::new(RejectResolver))
        .build()
        .unwrap();

    let res = client.get("http://127.0.0.1:8000/").send().await;
    println!("{:?}", res);
}
