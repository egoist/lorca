//! Lorca's Google client end to end: a relay that holds it, a stub of Google's token endpoint,
//! and a Runner (the `lorca` library) that signs a Gmail account in with it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::receivers::tests::{relay_with, runner};

#[test]
fn only_a_native_apps_two_grants_go_out_and_the_client_is_the_relays() {
    let client = Client::new("lorca.apps.googleusercontent.com", "secret", "https://oauth2.test/token").unwrap();
    let code = |redirect: &str| TokenRequest {
        grant_type: "authorization_code".into(),
        code: Some("c".into()),
        code_verifier: Some("v".into()),
        redirect_uri: Some(redirect.into()),
        refresh_token: None,
    };
    let request = code("http://127.0.0.1:53682/callback");
    let form = request.form(&client).unwrap();
    assert_eq!(form, [("grant_type", "authorization_code"), ("code", "c"), ("code_verifier", "v"), ("redirect_uri", "http://127.0.0.1:53682/callback"), ("client_id", "lorca.apps.googleusercontent.com"), ("client_secret", "secret")]);
    assert!(code("http://[::1]:8/cb").form(&client).is_ok() && code("http://localhost:8/cb").form(&client).is_ok());
    for redirect in ["https://evil.test/cb", "http://127.0.0.1.evil.test/cb", "http://user@127.0.0.1/cb", "com.example:/cb"] {
        assert!(code(redirect).form(&client).is_err(), "{redirect}");
    }
    let refresh = TokenRequest { grant_type: "refresh_token".into(), code: None, code_verifier: None, redirect_uri: None, refresh_token: Some("r".into()) };
    assert_eq!(refresh.form(&client).unwrap()[..2], [("grant_type", "refresh_token"), ("refresh_token", "r")]);
    let other = TokenRequest { grant_type: "client_credentials".into(), code: None, code_verifier: None, redirect_uri: None, refresh_token: None };
    assert!(other.form(&client).is_err(), "the relay's client never signs in as itself");
    assert!(TokenRequest { refresh_token: None, ..refresh }.form(&client).is_err());
}

/// Google's token endpoint: records each request's form and answers with fresh tokens. The
/// first expire at once, so the next use refreshes them.
async fn google_stub(requests: Arc<Mutex<Vec<BTreeMap<String, String>>>>) -> String {
    let router = Router::new().route(
        "/token",
        post(move |Form(form): Form<BTreeMap<String, String>>| {
            let n = { let mut requests = requests.lock().unwrap(); requests.push(form); requests.len() };
            async move { Json(json!({ "access_token": format!("access-{n}"), "refresh_token": "refresh", "token_type": "Bearer", "expires_in": if n == 1 { 1 } else { 3600 }, "scope": "gmail.readonly" })) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    url
}

fn gmail() -> lorca::plugins::Manifest {
    lorca::marketplace::bundled().plugins.into_iter().find(|m| m.id == "gmail").unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_runner_signs_google_in_with_lorcas_client_and_never_holds_its_secret() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let google = Client::new("lorca.apps.googleusercontent.com", "lorca-google-secret", &google_stub(requests.clone()).await).unwrap();
    let relay = relay_with(None, Clients { google: Some(google) }).await;
    let health: Value = reqwest::get(format!("{}/v1/health", relay.url)).await.unwrap().json().await.unwrap();
    assert_eq!(health["oauth_clients"], json!({ "google": "lorca.apps.googleusercontent.com" }), "the id, never the secret");

    let (app, _) = runner(&relay).await;
    let status = lorca::plugins::accounts::install(&app, gmail(), "marketplace", Some("Work")).unwrap();
    assert_eq!((status.state.as_str(), status.detail.as_str()), ("needs_auth", "Sign in"), "no client of the user's own to set up");

    // A phone asks, with its own loopback: the page names Lorca's client, the Runner keeps PKCE.
    let redirect = "http://127.0.0.1:53682/callback";
    let started = lorca::plugins::serve_request(&app, "plugins.connect", &json!({ "plugin_id": status.id, "redirect_uri": redirect }), None).await.unwrap();
    let page = reqwest::Url::parse(started["url"].as_str().unwrap()).unwrap();
    let query: BTreeMap<String, String> = page.query_pairs().into_owned().collect();
    assert_eq!((page.host_str(), query["client_id"].as_str()), (Some("accounts.google.com"), "lorca.apps.googleusercontent.com"));
    assert!(!query.contains_key("client_secret"));
    let callback = format!("{redirect}?state={}&code=the-code", query["state"]);
    let finish = json!({ "plugin_id": status.id, "sign_in": started["sign_in"], "url": callback });
    lorca::plugins::serve_request(&app, "plugins.sign_in.finish", &finish, None).await.unwrap();

    // Google got the code with the relay's client and secret, and the Runner's verifier.
    let exchanged = requests.lock().unwrap()[0].clone();
    assert_eq!((exchanged["grant_type"].as_str(), exchanged["code"].as_str(), exchanged["redirect_uri"].as_str()), ("authorization_code", "the-code", redirect));
    assert_eq!((exchanged["client_id"].as_str(), exchanged["client_secret"].as_str()), ("lorca.apps.googleusercontent.com", "lorca-google-secret"));
    assert_eq!(exchanged["code_verifier"].len(), 64);
    let saved = app.plugins.lock().unwrap().sign_in_secret(&status.id, "oauth", "gmail").unwrap();
    assert_eq!((saved["lorca_client"].as_str(), saved["client_secret"].as_null()), (Some("google"), Some(())), "the Runner keeps no secret");
    assert_eq!(saved["tokens"]["access_token"], "access-1");

    // The token expired: the Runner refreshes it through the relay too.
    // (The connection the sign-in starts may refresh it first.)
    let token = lorca::plugins::mcp::service_token(&app, &status.id, "gmail").await.unwrap();
    assert!(token != "access-1" && token.starts_with("access-"), "{token}");
    let refreshed = requests.lock().unwrap()[1].clone();
    assert_eq!((refreshed["grant_type"].as_str(), refreshed["refresh_token"].as_str(), refreshed["client_secret"].as_str()), ("refresh_token", "refresh", "lorca-google-secret"));

    // Only a paired machine swaps codes with the client.
    let anyone = reqwest::Client::new().post(format!("{}/v1/oauth/google/token", relay.url)).form(&[("grant_type", "refresh_token"), ("refresh_token", "refresh")]).send().await.unwrap();
    assert_eq!(anyone.status(), 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_relay_without_the_client_asks_for_the_users_own() {
    let relay = relay_with(None, Clients::default()).await;
    let (app, _) = runner(&relay).await;
    let status = lorca::plugins::accounts::install(&app, gmail(), "marketplace", None).unwrap();
    let started = lorca::plugins::serve_request(&app, "plugins.connect", &json!({ "plugin_id": status.id, "redirect_uri": "http://127.0.0.1:53682/callback" }), None).await;
    let error = started.unwrap_err();
    assert!(error.contains("does not register apps on the fly") && error.contains("GOOGLE_CLIENT_ID"), "{error}");
}
