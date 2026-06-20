use std::{
    env,
    error::Error,
    fmt,
    io::{Read, Write},
    net::TcpStream,
    process::ExitCode,
    time::Duration,
};

use serde_json::{Map, Value, json};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || take_flag(&mut args, "--help") || take_flag(&mut args, "-h") {
        print_help();
        return Ok(());
    }

    let base_url = take_option(&mut args, "--url")
        .or_else(|| env::var("ORDERBOOK_API").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
    let client = HttpClient::parse(&base_url)?;
    let command = args
        .first()
        .cloned()
        .ok_or_else(|| CliError("missing command".to_string()))?;
    let rest = args.into_iter().skip(1).collect::<Vec<_>>();

    let output = match command.as_str() {
        "health" => client.get("/api/health")?,
        "book" => {
            let depth = option_value(&rest, "--depth").unwrap_or_else(|| "25".to_string());
            client.get(&format!("/api/book?depth={depth}"))?
        }
        "orders" => client.get("/api/orders")?,
        "trades" => {
            let limit = option_value(&rest, "--limit").unwrap_or_else(|| "50".to_string());
            client.get(&format!("/api/trades?limit={limit}"))?
        }
        "metrics" => client.get("/api/metrics")?,
        "history" => {
            let order_id = positional(&rest, 0, "order id")?;
            client.get(&format!("/api/orders/{order_id}/history"))?
        }
        "cancel" => {
            let order_id = positional(&rest, 0, "order id")?;
            client.delete(&format!("/api/orders/{order_id}"))?
        }
        "mass-cancel" => client.delete("/api/orders")?,
        "submit" => client.post("/api/orders", build_order_payload(&rest)?)?,
        "replace" => {
            let order_id = positional(&rest, 0, "order id")?;
            client.patch(
                &format!("/api/orders/{order_id}"),
                build_order_payload(&rest[1..])?,
            )?
        }
        "kill-switch" => kill_switch(&client, &rest)?,
        _ => return Err(CliError(format!("unknown command: {command}")).into()),
    };

    print_json_or_text(&output);
    Ok(())
}

fn kill_switch(client: &HttpClient, args: &[String]) -> Result<String, Box<dyn Error>> {
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => Ok(client.get("/api/risk/kill-switch")?),
        "on" => Ok(client.post("/api/risk/kill-switch", json!({ "enabled": true }))?),
        "off" => Ok(client.post("/api/risk/kill-switch", json!({ "enabled": false }))?),
        value => Err(CliError(format!(
            "unknown kill-switch action: {value}; use status, on, or off"
        ))
        .into()),
    }
}

fn build_order_payload(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let mut payload = Map::new();
    payload.insert("side".to_string(), string_option(args, "--side", "buy"));
    payload.insert("type".to_string(), string_option(args, "--type", "limit"));
    payload.insert(
        "time_in_force".to_string(),
        string_option(args, "--tif", "gtc"),
    );

    insert_string_if_present(&mut payload, args, "--account-id", "account_id");
    insert_u64_if_present(&mut payload, args, "--quantity", "quantity")?;
    insert_u64_if_present(&mut payload, args, "--price", "price")?;
    insert_u64_if_present(&mut payload, args, "--quote-quantity", "quote_quantity")?;
    insert_u64_if_present(&mut payload, args, "--stop-price", "stop_price")?;

    Ok(Value::Object(payload))
}

fn insert_string_if_present(
    map: &mut Map<String, Value>,
    args: &[String],
    flag: &str,
    field: &str,
) {
    if let Some(value) = option_value(args, flag) {
        map.insert(field.to_string(), Value::String(value));
    }
}

fn insert_u64_if_present(
    map: &mut Map<String, Value>,
    args: &[String],
    flag: &str,
    field: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(value) = option_value(args, flag) {
        map.insert(field.to_string(), Value::from(value.parse::<u64>()?));
    }
    Ok(())
}

fn string_option(args: &[String], flag: &str, default: &str) -> Value {
    Value::String(option_value(args, flag).unwrap_or_else(|| default.to_string()))
}

fn positional(args: &[String], index: usize, name: &str) -> Result<String, CliError> {
    args.iter()
        .filter(|value| !value.starts_with("--"))
        .nth(index)
        .cloned()
        .ok_or_else(|| CliError(format!("missing {name}")))
}

fn option_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window.first().is_some_and(|value| value == flag))
        .and_then(|window| window.get(1))
        .cloned()
}

fn take_option(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let index = args.iter().position(|value| value == flag)?;
    if index + 1 >= args.len() {
        return None;
    }
    let value = args.remove(index + 1);
    args.remove(index);
    Some(value)
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let Some(index) = args.iter().position(|value| value == flag) else {
        return false;
    };
    args.remove(index);
    true
}

fn print_json_or_text(body: &str) {
    if body.trim().is_empty() {
        println!("ok");
        return;
    }
    match serde_json::from_str::<Value>(body) {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(pretty) => println!("{pretty}"),
            Err(_) => println!("{body}"),
        },
        Err(_) => println!("{body}"),
    }
}

#[derive(Debug)]
struct HttpClient {
    host: String,
    port: u16,
}

impl HttpClient {
    fn parse(base_url: &str) -> Result<Self, Box<dyn Error>> {
        let without_scheme = base_url
            .strip_prefix("http://")
            .ok_or_else(|| CliError("only http:// URLs are supported".to_string()))?;
        let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), port.parse::<u16>()?),
            None => (authority.to_string(), 80),
        };
        Ok(Self { host, port })
    }

    fn get(&self, path: &str) -> Result<String, Box<dyn Error>> {
        self.request("GET", path, None)
    }

    fn post(&self, path: &str, body: Value) -> Result<String, Box<dyn Error>> {
        self.request("POST", path, Some(body))
    }

    fn patch(&self, path: &str, body: Value) -> Result<String, Box<dyn Error>> {
        self.request("PATCH", path, Some(body))
    }

    fn delete(&self, path: &str) -> Result<String, Box<dyn Error>> {
        self.request("DELETE", path, None)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<String, Box<dyn Error>> {
        let body = match body {
            Some(value) => serde_json::to_string(&value)?,
            None => String::new(),
        };
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nUser-Agent: orderbook-cli/0.1\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.host,
            body.len(),
            body
        );

        let mut stream = TcpStream::connect((self.host.as_str(), self.port))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.write_all(request.as_bytes())?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        parse_response(&response)
    }
}

fn parse_response(response: &str) -> Result<String, Box<dyn Error>> {
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError("malformed HTTP response".to_string()))?;
    let status_line = head
        .lines()
        .next()
        .ok_or_else(|| CliError("missing HTTP status".to_string()))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| CliError("missing HTTP status code".to_string()))?
        .parse::<u16>()?;
    if (200..300).contains(&status) {
        return Ok(body.to_string());
    }
    Err(CliError(format!("request failed with status {status}: {body}")).into())
}

fn print_help() {
    println!(
        r#"orderbook-cli

Usage:
  orderbook-cli [--url http://127.0.0.1:8080] <command>

Commands:
  health
  book [--depth 25]
  orders
  trades [--limit 50]
  metrics
  history <order-id>
  cancel <order-id>
  mass-cancel
  kill-switch [status|on|off]
  submit --side buy --type limit --quantity 10 --price 10000 [--tif gtc] [--account-id acct-a]
  replace <order-id> --side buy --type limit --quantity 5 --price 10010

Order flags:
  --type limit|market|market_by_notional|post_only|stop_limit|stop_market
  --side buy|sell
  --quantity <lots>
  --quote-quantity <quote budget>
  --price <ticks>
  --stop-price <ticks>
  --tif gtc|ioc|fok
  --account-id <id>

Environment:
  ORDERBOOK_API=http://127.0.0.1:8080
"#
    );
}

#[derive(Debug)]
struct CliError(String);

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CliError {}
