use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::llm::{self, ResearchHit, ResearchSourceKind, WebSearchResult};

async fn serve_once(status: &str, body: &str, content_type: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    let content_type = content_type.to_string();
    let status = status.to_string();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0_u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    format!("http://{addr}/page")
}

#[tokio::test]
async fn fetch_url_returns_success_for_meaningful_page() {
    let body = "<html><title>Reservoir Notes</title><body>This page describes eigenvalue topology, manipulable relationships, and how spectral modes relate to perception in a readable way that matters for the current question.</body></html>";
    let url = serve_once("200 OK", body, "text/html").await;
    let result = llm::fetch_url(&url, "manipulable relationships")
        .await
        .unwrap();

    assert!(result.succeeded());
    assert_eq!(result.source_kind, ResearchSourceKind::Browse);
    assert_eq!(result.anchor, "manipulable relationships");
    assert!(result.meaning_summary.contains("Why it may matter:"));
    assert!(result.raw_text.contains("manipulable relationships"));
}

#[tokio::test]
async fn fetch_url_marks_http_404_as_failure() {
    let body = "<html><title>404</title><body>Nothing here.</body></html>";
    let url = serve_once("404 Not Found", body, "text/html").await;
    let result = llm::fetch_url(&url, "broken link").await.unwrap();

    assert!(!result.succeeded());
    assert_eq!(
        result.soft_failure_reason.as_deref(),
        Some("HTTP 404 from the source.")
    );
}

#[tokio::test]
async fn fetch_url_marks_soft_404_body_as_failure() {
    let body = "<html><title>Page Not Found</title><body>Page Not Found. The page you are trying to reach cannot be found. Error.</body></html>";
    let url = serve_once("200 OK", body, "text/html").await;
    let result = llm::fetch_url(&url, "paper").await.unwrap();

    assert!(!result.succeeded());
    assert!(
        result
            .soft_failure_reason
            .as_deref()
            .unwrap_or_default()
            .contains("error")
    );
}

#[tokio::test]
async fn fetch_url_marks_js_gate_as_failure() {
    let body = "<html><title>Access Denied</title><body>Access denied. Enable JavaScript to continue.</body></html>";
    let url = serve_once("200 OK", body, "text/html").await;
    let result = llm::fetch_url(&url, "paper").await.unwrap();

    assert!(!result.succeeded());
    assert!(
        result
            .soft_failure_reason
            .as_deref()
            .unwrap_or_default()
            .contains("access-gate")
    );
}

#[test]
fn web_search_prompt_body_puts_meaning_first() {
    let result = WebSearchResult {
        source_kind: ResearchSourceKind::Search,
        raw_text: "One raw hit".to_string(),
        hits: vec![ResearchHit {
            title: "Reservoir Computing".to_string(),
            snippet: "A structured snippet.".to_string(),
            url: "https://example.com/paper".to_string(),
        }],
        anchor: "reservoir computing".to_string(),
        meaning_summary: "Why it may matter: relevant\nWhat it seems to suggest: concrete\nBest next move: browse".to_string(),
    };

    let prompt_body = result.prompt_body();
    assert!(prompt_body.starts_with("Why it may matter:"));
    assert!(prompt_body.contains("Top results:"));
    assert!(prompt_body.contains("https://example.com/paper"));
}

#[test]
fn strip_model_artifacts_reports_removed_tokens() {
    let (stripped, report) =
        llm::strip_model_artifacts_with_report("hello<end_of_turn> [INST]world<|im_end|>");

    assert_eq!(stripped, "hello world");
    let report = report.expect("artifact cleanup report should exist");
    assert_eq!(report.removed_total, 3);
    assert_eq!(
        report.before_chars,
        "hello<end_of_turn> [INST]world<|im_end|>".len()
    );
    assert_eq!(report.after_chars, stripped.len());
    assert!(
        report
            .removed_tokens
            .iter()
            .any(|token| token.token == "<end_of_turn>" && token.count == 1)
    );
}
