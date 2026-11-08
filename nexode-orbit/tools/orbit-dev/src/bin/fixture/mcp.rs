use serde_json::{Value, json};
pub fn run(server: &str, names: &str) {
    use std::io::{BufRead, Write};
    let names: Vec<String> = serde_json::from_str(names).unwrap();
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if request["method"] == "shutdown" {
            break;
        }
        let Some(id) = request.get("id") else {
            continue;
        };
        let mut reply = json!({"jsonrpc":"2.0", "id":id});
        let result = match request["method"].as_str() {
            Some("initialize") => Some(
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":server,"version":"1"}}),
            ),
            Some("tools/list") => Some(
                json!({"tools":names.iter().map(|name|json!({"name":name,"description":format!("{server}:{name}"),"inputSchema":{"type":"object","properties":{"token":{"type":"string"}}}})).collect::<Vec<_>>()}),
            ),
            Some("tools/call")
                if request["params"]["name"]
                    .as_str()
                    .is_some_and(|n| names.iter().any(|name| name == n)) =>
            {
                let value = json!({"server":server,"tool":request["params"]["name"],"arguments":request["params"]["arguments"],"pid":std::process::id()});
                Some(json!({"content":[{"type":"text","text":value.to_string()}],"isError":false}))
            }
            _ => None,
        };
        if let Some(result) = result {
            reply["result"] = result;
        } else {
            reply["error"] =
                json!({"code":-32602,"message":"unknown method or original tool name"});
        }
        writeln!(out, "{reply}").unwrap();
        out.flush().unwrap();
    }
}
