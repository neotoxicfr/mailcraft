//! Google sign-in identifiers for this build, never in the source: the
//! `MAILCRAFT_GOOGLE_CLIENT_ID` / `MAILCRAFT_GOOGLE_CLIENT_SECRET` environment variables (release
//! builds, from GitHub secrets), or else the file Google gives for a desktop client, saved as
//! `google-client.json` at the root of the repository (ignored by git).

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-env-changed=MAILCRAFT_GOOGLE_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=MAILCRAFT_GOOGLE_CLIENT_SECRET");
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../google-client.json");
    println!("cargo:rerun-if-changed={}", file.display());
    let from_env = std::env::var("MAILCRAFT_GOOGLE_CLIENT_ID").is_ok_and(|v| !v.is_empty());
    if from_env {
        return;
    }
    let Ok(text) = std::fs::read_to_string(&file) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        println!("cargo:warning=google-client.json is not valid JSON: Google sign-in left out");
        return;
    };
    // Google's download: {"installed": {"client_id": …, "client_secret": …, …}}
    let client = json.get("installed").unwrap_or(&json);
    match (
        client["client_id"].as_str(),
        client["client_secret"].as_str(),
    ) {
        (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
            println!("cargo:rustc-env=MAILCRAFT_GOOGLE_CLIENT_ID={id}");
            println!("cargo:rustc-env=MAILCRAFT_GOOGLE_CLIENT_SECRET={secret}");
        }
        _ => println!(
            "cargo:warning=google-client.json has no desktop client id/secret: Google sign-in left out"
        ),
    }
}
