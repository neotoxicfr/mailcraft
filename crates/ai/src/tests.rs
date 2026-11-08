use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

use serde_json::json;

use super::{
    AiError, Assistant, Choice, Provider, SenderFacts, chat_text, classify_body, parse_verdicts,
    short, verdicts_from_text,
};

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

type Seen = (Vec<String>, String);

/// A local stand-in for an API: answers `answer` once, gives back the headers and body it got.
fn serve(answer: String) -> (String, std::thread::JoinHandle<Seen>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/v1/x",
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
        let mut out = stream;
        write!(
            out,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{answer}",
            answer.len()
        )
        .unwrap();
        (headers, String::from_utf8(body).unwrap())
    });
    let _ = rustls::crypto::ring::default_provider().install_default();
    (url, server)
}

const VERDICTS: &str = "{\"verdicts\":[{\"adresse\":\"news@shop.example\",\"categorie\":\"newsletter\",\"confiance\":0.9,\"raison\":\"Lettre d'info\",\"action_suggeree\":\"garder\"}]}";

#[test]
fn anthropic_round_trip_against_a_local_server() {
    let answer =
        json!({ "stop_reason": "end_turn", "content": [{ "type": "text", "text": VERDICTS }] });
    let (url, server) = serve(answer.to_string());
    let http = reqwest::blocking::Client::new();
    let choice = Choice {
        provider: Provider::Anthropic,
        url: url.clone(),
        ..Choice::default()
    };
    let verdicts = Assistant::new(&http, " sk-test ", &choice)
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
    assert!(body.contains("\"json_schema\"") && body.contains("claude-haiku-5-5"));

    // No key: nothing is sent.
    let r = Assistant::new(&http, "", &choice).classify(&[facts()]);
    assert!(matches!(r, Err(AiError::Key(_))));
}

#[test]
fn openai_format_round_trip_and_local_ollama_without_key() {
    let answer =
        json!({ "choices": [{ "finish_reason": "stop", "message": { "content": VERDICTS } }] });
    let (url, server) = serve(answer.to_string());
    let http = reqwest::blocking::Client::new();
    let choice = Choice {
        provider: Provider::Mistral,
        model: "mon-modele".into(),
        url,
    };
    let verdicts = Assistant::new(&http, "k-123", &choice)
        .classify(&[facts()])
        .unwrap();
    assert_eq!(verdicts[0].adresse, "news@shop.example");
    let (headers, body) = server.join().unwrap();
    assert!(
        headers
            .iter()
            .any(|h| h.trim() == "authorization: bearer k-123")
    );
    let sent: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(sent["model"], "mon-modele");
    assert_eq!(sent["messages"][0]["role"], "system");
    assert_eq!(sent["response_format"]["json_schema"]["strict"], true);

    // Ollama: no key needed, and none is sent.
    let answer = json!({ "choices": [{ "message": { "content": "Résumé du jour." } }] });
    let (url, server) = serve(answer.to_string());
    let choice = Choice {
        provider: Provider::Ollama,
        model: String::new(),
        url,
    };
    let text = Assistant::new(&http, "", &choice)
        .digest(&[("Shop".into(), "Promo".into(), "-20 %".into())])
        .unwrap();
    assert_eq!(text, "Résumé du jour.");
    let (headers, body) = server.join().unwrap();
    assert!(!headers.iter().any(|h| h.starts_with("authorization")));
    assert!(body.contains("llama3.2"));
}

#[test]
fn chat_answers_refusals_fences_and_defaults() {
    let refused = json!({ "choices": [{ "message": { "content": null, "refusal": "non" } }] });
    assert!(matches!(chat_text(&refused), Err(AiError::Answer(_))));
    let cut = json!({ "choices": [{ "finish_reason": "length", "message": { "content": "{" } }] });
    assert!(chat_text(&cut).is_err());
    let fenced = format!("Voici :\n```json\n{VERDICTS}\n```");
    assert_eq!(verdicts_from_text(&fenced).unwrap().len(), 1);
    assert_eq!(Choice::default().model(), "claude-haiku-5-5");
    assert!(Provider::Anthropic.key_looks_valid("sk-ant-abc"));
    assert!(!Provider::OpenAi.key_looks_valid("sk-ant-abc"));
    assert!(!Provider::Gemini.key_looks_valid("deux mots"));
    assert!(!Provider::Ollama.needs_key() && Provider::Gemini.needs_key());
}

#[test]
fn short_texts() {
    assert_eq!(short("  un   deux\ntrois ", 50), "un deux trois");
    assert_eq!(short("éééééé", 3), "ééé…");
}
