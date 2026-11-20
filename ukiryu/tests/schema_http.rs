//! Local HTTP observations of clean ukiryu/ukiryu ab6df353, Ruby 4.0.1,
//! json-schema 6.2.1. No request leaves loopback; no live schema is consulted.
#![cfg(not(target_arch = "wasm32"))]
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use ukiryu::definition::definition_validator::{
    self, RemoteSchemaSource, SchemaStage, ValidatorEnv,
};
use ukiryu::schema::{self, Data, Permitted, SchemaError};

struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<usize>>,
}
impl Server {
    fn start(routes: Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let url = base.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut requests = 0;
            for connection in listener.incoming() {
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                let mut stream = connection.unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                }
                requests += 1;
                let route = routes.get(&path);
                let status = route.map_or(404, |r| r[0].as_u64().unwrap());
                let body = route.map_or("missing", |r| r[1].as_str().unwrap());
                let location = route
                    .and_then(|r| r[2].as_str())
                    .map(|s| format!("Location: {url}{s}\r\n"))
                    .unwrap_or_default();
                write!(stream, "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nConnection: close\r\n{location}\r\n{body}", match status {200 => "OK",302 => "Found",_ => "Not Found"}, body.len()).unwrap();
            }
            requests
        });
        Self {
            base,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = std::net::TcpStream::connect(self.base.trim_start_matches("http://"));
        let count = self.thread.take().unwrap().join().unwrap();
        assert!(count > 0, "empty HTTP replay");
    }
}
fn data(value: &Value) -> Data {
    schema::safe_load(&value.to_string(), Permitted::NONE).unwrap()
}
fn project(data: &Data) -> Value {
    match data {
        Data::Null => Value::Null,
        Data::Bool(b) => json!(b),
        Data::String(s) | Data::Symbol(s) => json!(s),
        Data::Array(a) => Value::Array(a.iter().map(project).collect()),
        Data::Hash(h) => Value::Object(
            h.iter()
                .map(|(k, v)| (k.ruby_to_s().unwrap(), project(v)))
                .collect(),
        ),
        _ => panic!("unexpected downloaded YAML value: {data:?}"),
    }
}
fn error(error: SchemaError) -> Value {
    json!({"error":{"class":error.ruby_class(), "message":error.message()}})
}
#[test]
fn replay_native_http_and_default_fallback() {
    let golden: Value = serde_json::from_str(include_str!("schema_golden/http.json")).unwrap();
    assert_eq!(golden["oracle"], "ab6df353c6dc2d7faf7817a86aafcafa82579896");
    let server = Server::start(golden["routes"].clone());
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_auth = dead.local_addr().unwrap().to_string();
    drop(dead);
    let rows = golden["cases"].as_array().unwrap();
    assert!(rows.len() >= 49);
    let mut failures = Vec::new();
    for case in rows {
        let text = case
            .to_string()
            .replace("${BASE}", &server.base)
            .replace("${DEAD}", &format!("http://{dead_auth}"))
            .replace("${DEAD_AUTH}", &dead_auth)
            .replace("${DEAD_PORT}", dead_auth.rsplit_once(':').unwrap().1);
        let case: Value = serde_json::from_str(&text).unwrap();
        let input = &case["input"];
        let got = match input["op"].as_str().unwrap() {
            "schema" => {
                match schema::fully_validate(&data(&input["schema"]), &data(&input["data"]), false)
                {
                    Ok(messages) => json!({"ok": messages}),
                    Err(e) => error(e),
                }
            }
            "download" => {
                match definition_validator::download_and_load_schema(input["url"].as_str().unwrap())
                {
                    Ok(value) => json!({"ok": value.as_ref().map(project)}),
                    Err(e) => error(e),
                }
            }
            "fallback" => {
                let env = ValidatorEnv {
                    schema_path: None,
                    remote_schema: RemoteSchemaSource::Url(input["url"].as_str().unwrap().into()),
                };
                match definition_validator::validate_report(&data(&input["data"]), None, &env) {
                    Ok(report) => {
                        assert!(matches!(
                            report.schema_stage,
                            Some(SchemaStage::RemoteFallback { .. })
                        ));
                        let r = report.result;
                        json!({"ok":{"valid":r.is_valid(),"errors":r.errors,"warnings":r.warnings,"schema_path":r.schema_path,"error_count":r.error_count(),"warning_count":r.warning_count()}})
                    }
                    Err(e) => error(e),
                }
            }
            other => panic!("unexpected operation: {other}"),
        };
        if got != case["expect"] {
            failures.push(format!(
                "{}: expected {}, got {}",
                case["id"], case["expect"], got
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
