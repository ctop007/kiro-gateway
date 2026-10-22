use super::*;
use crate::gateway::testing::{free_port, TestServer};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn probe() -> HttpHealth {
    HttpHealth::new(Duration::from_millis(700))
}

#[test]
fn health_is_true_for_a_200_answer_on_the_health_path() {
    let server = TestServer::start(|head| {
        assert!(head.starts_with("GET /health "), "{head}");
        Some(TestServer::json(200, "{\"status\":\"healthy\"}"))
    });
    assert!(probe().is_healthy(server.port));
}

#[test]
fn health_is_false_for_error_statuses() {
    for status in [404, 500, 401] {
        let server = TestServer::start(move |_| Some(TestServer::json(status, "{}")));
        assert!(!probe().is_healthy(server.port), "{status}");
    }
}

#[test]
fn health_is_false_when_nothing_listens() {
    assert!(!probe().is_healthy(free_port()));
}

#[test]
fn health_is_false_for_a_garbage_answer() {
    let server = TestServer::start(|_| Some(b"this is not http\r\n\r\n".to_vec()));
    assert!(!probe().is_healthy(server.port));
}

#[test]
fn health_gives_up_on_a_server_that_accepts_but_never_answers() {
    let server = TestServer::start(|_| None);
    let started = Instant::now();
    assert!(!probe().is_healthy(server.port));
    assert!(started.elapsed() < Duration::from_millis(2500), "took {:?}", started.elapsed());
}

#[test]
fn health_does_not_follow_redirects() {
    let server = TestServer::start(|_| {
        Some(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/health\r\nContent-Length: 0\r\n\r\n".to_vec())
    });
    assert!(!probe().is_healthy(server.port));
}

#[test]
fn a_system_proxy_in_the_environment_is_ignored() {
    // A dead proxy would make every request fail if it were honoured.
    let dead_proxy = format!("http://127.0.0.1:{}", free_port());
    std::env::set_var("HTTP_PROXY", &dead_proxy);
    std::env::set_var("http_proxy", &dead_proxy);
    std::env::set_var("ALL_PROXY", &dead_proxy);
    let server = TestServer::start(|_| Some(TestServer::json(200, "{\"status\":\"healthy\"}")));
    let ok = probe().is_healthy(server.port);
    let models = fetch_models(server.port, SECRET, Duration::from_secs(1));
    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("http_proxy");
    std::env::remove_var("ALL_PROXY");
    assert!(ok);
    assert!(models.is_err(), "the fake answered /health JSON, which is no model list, but it was reached");
    assert_eq!(models.unwrap_err(), ModelsError::BadResponse);
}

fn models_body(ids: &[&str]) -> String {
    let data: Vec<String> = ids.iter().map(|id| format!("{{\"id\":\"{id}\",\"object\":\"model\",\"owned_by\":\"anthropic\"}}")).collect();
    format!("{{\"object\":\"list\",\"data\":[{}]}}", data.join(","))
}

#[test]
fn models_are_returned_sorted_and_without_duplicates() {
    let server = TestServer::start(|_| Some(TestServer::json(200, &models_body(&["claude-sonnet-4.5", "auto", "claude-haiku-4.5", "auto"]))));
    let ids = fetch_models(server.port, SECRET, Duration::from_secs(1)).unwrap();
    assert_eq!(ids, vec!["auto", "claude-haiku-4.5", "claude-sonnet-4.5"]);
}

#[test]
fn the_request_carries_the_bearer_key_and_asks_for_v1_models() {
    let server = TestServer::start(|_| Some(TestServer::json(200, &models_body(&["a"]))));
    fetch_models(server.port, SECRET, Duration::from_secs(1)).unwrap();
    let head = server.requests.lock().unwrap()[0].clone();
    assert!(head.starts_with("GET /v1/models "), "{head}");
    assert!(head.to_lowercase().contains(&format!("authorization: bearer {SECRET}").to_lowercase()), "{head}");
}

#[test]
fn a_wrong_key_answer_is_an_unreachable_error_without_leaking_the_key() {
    let server = TestServer::start(|_| Some(TestServer::json(401, "{\"detail\":\"Invalid or missing API Key\"}")));
    let err = fetch_models(server.port, SECRET, Duration::from_secs(1)).unwrap_err();
    assert_eq!(err, ModelsError::Unreachable);
    assert!(!err.message().contains(SECRET));
    assert_eq!(err.code(), "gateway_unreachable");
}

#[test]
fn server_errors_and_closed_ports_are_unreachable() {
    let server = TestServer::start(|_| Some(TestServer::json(500, "{}")));
    assert_eq!(fetch_models(server.port, SECRET, Duration::from_secs(1)).unwrap_err(), ModelsError::Unreachable);
    assert_eq!(fetch_models(free_port(), SECRET, Duration::from_secs(1)).unwrap_err(), ModelsError::Unreachable);
}

#[test]
fn a_silent_server_hits_the_timeout() {
    let server = TestServer::start(|_| None);
    let started = Instant::now();
    let err = fetch_models(server.port, SECRET, Duration::from_millis(400)).unwrap_err();
    assert_eq!(err, ModelsError::Unreachable);
    assert!(started.elapsed() < Duration::from_millis(2500));
}

#[test]
fn malformed_model_lists_are_bad_responses() {
    for body in ["not json", "[]", "{}", "{\"data\":\"x\"}", "{\"data\":[1,2]}", "{\"data\":null}"] {
        let reply = body.to_string();
        let server = TestServer::start(move |_| Some(TestServer::json(200, &reply)));
        let result = fetch_models(server.port, SECRET, Duration::from_secs(1));
        match result {
            Err(ModelsError::BadResponse) => {}
            Ok(ids) if ids.is_empty() => {} // {"data":[1,2]}: entries without an id are skipped
            other => panic!("{body:?}: {other:?}"),
        }
    }
}

#[test]
fn an_empty_list_is_a_valid_answer() {
    let server = TestServer::start(|_| Some(TestServer::json(200, "{\"object\":\"list\",\"data\":[]}")));
    assert_eq!(fetch_models(server.port, SECRET, Duration::from_secs(1)).unwrap(), Vec::<String>::new());
}

#[test]
fn entries_without_a_string_id_are_skipped() {
    let server = TestServer::start(|_| Some(TestServer::json(200, "{\"data\":[{\"id\":\"ok\"},{\"id\":5},{\"name\":\"x\"},{\"id\":\"\"},\"str\"]}")));
    assert_eq!(fetch_models(server.port, SECRET, Duration::from_secs(1)).unwrap(), vec!["ok"]);
}

#[test]
fn a_connection_closed_without_an_answer_is_unreachable() {
    let port = free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let t = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        drop(stream);
    });
    assert_eq!(fetch_models(port, SECRET, Duration::from_secs(1)).unwrap_err(), ModelsError::Unreachable);
    t.join().unwrap();
}

#[test]
fn error_messages_are_plain_chinese_without_urls() {
    for e in [ModelsError::Unreachable, ModelsError::BadResponse] {
        assert!(!e.message().contains("http"));
        assert!(!e.message().contains("127.0.0.1"));
        assert_eq!(e.code(), "gateway_unreachable");
    }
}
