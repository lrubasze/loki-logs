use clap::Parser;
use itertools::Itertools;
use reqwest::blocking::Client;
use reqwest::Identity;
use serde_json::{json, Value};
use std::env;
use std::{fs, time::Duration};

const PATH_TEMPLATE: &str = ".tsh/keys/teleport.parity.io/{user}@parity.io-app/teleport.parity.io";
const PARITY_ZOMBIENET_UID: &str = "PCF9DACBDF30E12B3";

// Generate cert and key:
// ```
//   tsh login --proxy=teleport.parity.io --auth=google
//   tsh apps login grafana
// ```
//
// Data sources:
//   loki.parity-zombienet -> uid: PCF9DACBDF30E12B3
//
//
// more details:
// - all data sources:
//   URL=https://grafana.teleport.parity.io/api/datasources
// - parity-zombienet data source
//   URL=https://grafana.teleport.parity.io/api/datasources/name/loki.parity-zombienet
// ```
//  curl  \
//      --cert "/Users/lukasz/.tsh/keys/teleport.parity.io/lukasz@parity.io-app/teleport.parity.io/grafana.crt" \
//      --key "/Users/lukasz/.tsh/keys/teleport.parity.io/lukasz@parity.io-app/teleport.parity.io/grafana.key" \
//      -H "Accept: application/json" -H "Content-Type: application/json"
//      $URL | jq '.'
// ```
//
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Namespace of the pod
    namespace: String,

    /// Pod name
    pod: String,

    /// Number of lines to fetch
    #[arg(short, long, default_value_t = 1000u64)]
    lines: u64,

    /// Start time for logs in Grafana format (default: now-24h)
    #[arg(short, long, default_value = "now-24h")]
    from: String,

    /// End time for logs in Grafana format (default: now)
    #[arg(short, long, default_value = "now")]
    to: String,
    /// Print raw JSON response
    #[arg(long)]
    raw: bool,
}

struct Loki {
    client: Client,
}

impl Loki {
    fn new() -> Result<Self, anyhow::Error> {
        let identity = get_identity()?;
        let client = Client::builder()
            .use_rustls_tls()
            .identity(identity)
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Loki { client })
    }

    fn get_logs(
        &self,
        namespace: &str,
        pod: &str,
        from: &str,
        to: &str,
        lines: u64,
        raw: bool,
    ) -> Result<(), anyhow::Error> {
        let body = json!({
            "queries": [
                {
                    "refId": "A",
                    "expr": format!("{{namespace=\"{}\", pod=\"{}\"}}", namespace, pod),
                    "queryType": "range",
                    "datasource": {
                        "type": "loki",
                        "uid": PARITY_ZOMBIENET_UID
                    },
                    "direction":"forward",
                    // NOTE! ATM there is a limit max_entries_limit=5000, which we cannot exceed
                    "maxLines": lines,
                    "format": "log",
                    "step": "",
                    "datasourceId": 24,
                    "intervalMs": 500,
                    "maxDataPoints": 1272
                }
            ],
            "from": from,
            "to": to
        });

        let res = self
            .client
            .post("https://grafana.teleport.parity.io/api/ds/query")
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()?;

        let text = res.text()?;
        let json_response: Value = serde_json::from_str(&text)?;

        if raw {
            println!("{}", text);
            return Ok(());
        }
        if let Some(log_lines) = json_response
            .get("results")
            .and_then(|r| r.get("A"))
            .and_then(|a| a.get("frames"))
            .and_then(|frames| frames.get(0))
            .and_then(|frame| frame.get("data"))
            .and_then(|data| {
                let x = data.get("values");
                x
            })
            .and_then(|values| {
                values.get(1).and_then(|v1| v1.as_array()).and_then(|v1| {
                    values
                        .get(2)
                        .and_then(|v2| v2.as_array())
                        .map(|v2| (v1, v2))
                })
            })
            // Lines must be sorted according to the timestamp
            .and_then(|(timestamps, lines)| {
                let lines: Vec<&Value> = timestamps
                    .iter()
                    .zip(lines.iter())
                    .sorted_by_key(|(tstamp, _)| tstamp.as_u64().unwrap())
                    .map(|(_, lines)| lines)
                    .collect();
                Some(lines)
            })
        {
            for log in log_lines {
                if let Some(log_str) = log.as_str() {
                    println!("{}", log_str);
                }
            }
        } else {
            println!("No log lines found in the response.");
        }

        Ok(())
    }
}

fn get_identity() -> Result<Identity, anyhow::Error> {
    let home_dir = env::var("HOME").expect("HOME environment variable not set");
    let username = env::var("USER").expect("USER environment variable not set");

    let cert_path = format!(
        "{}/{}/grafana.crt",
        home_dir,
        PATH_TEMPLATE.replace("{user}", &username)
    );
    let key_path = format!(
        "{}/{}/grafana.key",
        home_dir,
        PATH_TEMPLATE.replace("{user}", &username)
    );

    let mut pem = fs::read(cert_path)?;
    let mut key_pem = fs::read(key_path)?;
    pem.append(&mut key_pem);

    let identity = Identity::from_pem(&pem)?;

    Ok(identity)
}

fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    let loki = Loki::new()?;

    loki.get_logs(
        &args.namespace,
        &args.pod,
        &args.from,
        &args.to,
        args.lines,
        args.raw,
    )?;
    Ok(())
}
