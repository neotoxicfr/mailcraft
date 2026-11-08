use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

use serde_json::json;

use super::{AiError, Claude, SenderFacts, classify_body, parse_verdicts, short};

fn facts() -> SenderFacts {
    SenderFacts {
        adresse: "news@shop.example".into(),
        nom: "Shop".into(),
        domaine: "shop.example".into(),
        spf: "pass".into(),
        dkim: "pass".into(),
        dmarc: "pass".into(),
        alertes: vec![],
        desabonnement_possible: true,
        nombre_de_mails: 12,
        objets: vec!["-20 % ce week-end".into()],
        extraits: vec!["Profitez de nos offres".into()],
    }
}

#[test]
fn request_body_shape() {
    let b = classify_body(&[facts()]);
    assert_eq!(b["model"], "claude-haiku-5-5");
    assert_eq!(b["output_config"]["format"]["type"], "json_schema");
    assert_eq!(b["output_config"]["effort"], "low");
    assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
    let item = &b["output_config"]["format"]["schema"]["properties"]["verdicts"]["items"];
    assert_eq!(item["additionalProperties"], false);
    assert_eq!(item["properties"]["categorie"]["enum"][5], "phishing");
    assert!(
        b["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("news@shop.example")
    );
    // No prefill, no sampling parameters (both rejected by current models).
    assert_eq!(b["messages"].as_array().unwrap().len(), 1);
    assert!(b.get("temperature").is_none());
}

#[test]
fn verdict_parsing_refusal_and_thinking_blocks() {
    let ok = json!({
        "stop_reason": "end_turn",
        "content": [
            { "type": "thinking", "thinking": "" },
            { "type": "text", "text": "{\"verdicts\":[{\"adresse\":\"news@shop.example\",\"categorie\":\"promo\",\"confiance\":1.4,\"raison\":\"Publicité\",\"action_suggeree\":\"desabonner\"}]}" }
        ]
    });
    let v = parse_verdicts(&ok).unwrap();
    assert_eq!((v[0].categorie.as_str(), v[0].confiance), ("promo", 1.0));
    let refusal = json!({ "stop_reason": "refusal", "content": [], "stop_details": null });
    assert!(matches!(parse_verdicts(&refusal), Err(AiError::Answer(_))));
    let garbage =
        json!({ "stop_reason": "end_turn", "content": [{ "type": "text", "text": "not json" }] });
    assert!(parse_verdicts(&garbage).is_err());
}

/// A local stand-in for the API: checks the headers and answers with a canned response.
#[test]
fn http_round_trip_against_a_local_server() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/v1/messages",
        listener.local_addr().unwrap().port()
    );
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut headers = Vec::new();
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                length = v.trim().parse().unwrap();
            }
            headers.push(lower);
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).unwrap();
        let answer = json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "{\"verdicts\":[{\"adresse\":\"news@shop.example\",\"categorie\":\"newsletter\",\"confiance\":0.9,\"raison\":\"Lettre d'info\",\"action_suggeree\":\"garder\"}]}" }]
        })
        .to_string();
        let mut out = stream;
        write!(out, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{answer}", answer.len()).unwrap();
        (headers, String::from_utf8(body).unwrap())
    });
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = reqwest::blocking::Client::new();
    let verdicts = Claude::with_url(&http, " sk-test ", &url)
        .classify(&[facts()])
        .unwrap();
    assert_eq!(verdicts[0].categorie, "newsletter");
    let (headers, body) = server.join().unwrap();
    assert!(headers.iter().any(|h| h.trim() == "x-api-key: sk-test"));
    assert!(
        headers
            .iter()
            .any(|h| h.starts_with("anthropic-version: 2026-10-21"))
    );
    assert!(body.contains("\"json_schema\""));

    // No key: nothing is sent.
    assert!(matches!(
        Claude::with_url(&http, "", &url).classify(&[facts()]),
        Err(AiError::Key(_))
    ));
}

#[test]
fn short_texts() {
    assert_eq!(short("  un   deux\ntrois ", 50), "un deux trois");
    assert_eq!(short("éééééé", 3), "ééé…");
}
