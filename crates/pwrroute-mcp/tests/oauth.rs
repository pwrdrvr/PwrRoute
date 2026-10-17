//! The OAuth 2.1 flow against a running server: discovery from the 401,
//! dynamic registration, the in-app approval, the code exchange, and
//! revocation.

mod support;

use std::sync::Arc;

use pwrroute_mcp::{
    ApprovalDecision, Role,
    fake::{FakeAnswer, FakeApprover},
};
use reqwest::{StatusCode, header};
use serde_json::{Value, json};
use support::{ALL_SCOPES, Harness, Pkce, REDIRECT_URI, redirect_params, waiting_id};

#[tokio::test]
async fn a_client_finds_the_authorization_server_from_the_401() {
    let h = Harness::start(Role::Reader).await;
    let response = h.http.post(h.resource()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let challenge = response.headers()[header::WWW_AUTHENTICATE]
        .to_str()
        .unwrap()
        .to_owned();
    let prm_url = h.url("/.well-known/oauth-protected-resource/mcp");
    assert_eq!(
        challenge,
        format!(r#"Bearer resource_metadata="{prm_url}""#)
    );
    let body: Value = response.json().await.unwrap();
    assert_eq!(body, json!({ "error": "unauthorized" }));

    let prm: Value = h
        .http
        .get(&prm_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(prm["resource"], h.resource());
    assert_eq!(prm["resource_name"], "PwrRoute");
    let issuer = format!("{}/", h.base);
    assert_eq!(prm["authorization_servers"], json!([issuer]));

    let response = h
        .http
        .get(h.url("/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let metadata: Value = response.json().await.unwrap();
    assert_eq!(metadata["issuer"], issuer);
    assert_eq!(metadata["registration_endpoint"], h.url("/register"));
    assert_eq!(
        metadata["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(
        metadata["grant_types_supported"],
        json!(["authorization_code"])
    );
    assert_eq!(
        metadata["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );
    assert_eq!(
        metadata["scopes_supported"].as_array().unwrap().len(),
        7,
        "{metadata}"
    );

    // Only the path-suffixed RFC 9728 document is served, as in the siblings.
    let bare = h
        .http
        .get(h.url("/.well-known/oauth-protected-resource"))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status(), StatusCode::NOT_FOUND);
    h.finish().await;
}

#[tokio::test]
async fn approval_happens_in_the_app_and_yields_a_working_token() {
    let h = Harness::start(Role::Opener).await;
    let token = h.login(Some(ALL_SCOPES)).await;
    assert!(token.starts_with("prmcp_"));

    let asked = h.approver.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].client_name, "Test agent");
    assert_eq!(asked[0].suggested_session_name, "Test agent");
    assert_eq!(asked[0].redirect_uri, REDIRECT_URI);
    assert_eq!(asked[0].roles, Role::ALL);

    let sessions = h.access.sessions();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "Test agent");
    assert_eq!(sessions[0].role, Role::Opener);

    let client = h.client(&token).await;
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 6);
    client.cancel().await.unwrap();

    // A second approval of the same client gets a distinct suggested name.
    h.login(Some(ALL_SCOPES)).await;
    assert_eq!(h.approver.asked()[1].suggested_session_name, "Test agent 2");
    h.finish().await;
}

#[tokio::test]
async fn only_roles_within_the_requested_scopes_are_offered() {
    let h = Harness::start(Role::Reader).await;
    // No scope: the Reader's scopes, so only Reader is offered.
    h.login(None).await;
    assert_eq!(h.approver.asked()[0].roles, [Role::Reader]);
    h.finish().await;
}

#[tokio::test]
async fn a_denial_redirects_with_access_denied_and_the_state() {
    let h = Harness::start(Role::Reader).await;
    h.approver.set(FakeAnswer::Deny);
    let client_id = h.register().await;
    let params = h.authorize(&client_id, &Pkce::new(), None).await;
    assert_eq!(params["error"], "access_denied");
    assert_eq!(params["state"], "xyz");
    assert!(!params.contains_key("code"));
    assert!(h.access.sessions().is_empty());
    h.finish().await;
}

#[tokio::test]
async fn an_approval_outside_the_offer_is_a_denial() {
    let h = Harness::start(Role::Reader).await;
    // Editor was not offered: the client asked for the Reader's scopes only.
    h.approver
        .set(FakeAnswer::Decide(ApprovalDecision::Approve {
            session_name: "Sneaky".into(),
            role: Role::Editor,
        }));
    let client_id = h.register().await;
    let params = h.authorize(&client_id, &Pkce::new(), None).await;
    assert_eq!(params["error"], "access_denied");

    h.approver
        .set(FakeAnswer::Decide(ApprovalDecision::Approve {
            session_name: "   ".into(),
            role: Role::Reader,
        }));
    let params = h.authorize(&client_id, &Pkce::new(), None).await;
    assert_eq!(params["error"], "access_denied");
    h.finish().await;
}

#[tokio::test]
async fn the_browser_cannot_approve() {
    let h = Harness::start(Role::Editor).await;
    h.approver.set(FakeAnswer::Hold);
    let client_id = h.register().await;
    let pkce = Pkce::new();
    let resource = h.resource();
    let base = [
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", REDIRECT_URI),
        ("code_challenge", pkce.challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", resource.as_str()),
    ];

    // A decision in the URL is refused outright.
    let mut forged = base.to_vec();
    forged.push(("decision", "allow"));
    let response = h.get_with_query("/authorize", &forged).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // So is any method but GET.
    let response = h.http.post(h.url("/authorize")).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET");

    // The waiting page has no way to act, and keeps waiting.
    let response = h.get_with_query("/authorize", &base).await;
    assert_eq!(response.status(), StatusCode::OK);
    let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("form-action 'none'"), "{csp}");
    let page = response.text().await.unwrap();
    for tag in ["<script", "<form", "<input", "<button"] {
        assert!(!page.contains(tag), "{tag} in {page}");
    }
    let id = waiting_id(&page).unwrap();
    let again = h.get_with_query("/authorize/status", &[("id", &id)]).await;
    assert_eq!(again.status(), StatusCode::OK);

    // Only the app's answer moves it on, once.
    let (request, reply) = h.approver.take_pending().pop().unwrap();
    assert_eq!(request.roles, [Role::Reader]);
    reply.approve("From the window", Role::Reader);
    let params = h.follow_waiting_page(again).await;
    assert!(params.contains_key("code"));
    let replay = h.get_with_query("/authorize/status", &[("id", &id)]).await;
    assert_eq!(replay.status(), StatusCode::NOT_FOUND);
    h.finish().await;
}

#[tokio::test]
async fn authorize_requires_pkce_s256_a_registered_redirect_and_this_resource() {
    let h = Harness::start(Role::Reader).await;
    let client_id = h.register().await;
    let pkce = Pkce::new();
    let resource = h.resource();
    let request = |overrides: &[(&'static str, &'static str)]| {
        let mut query: Vec<(&str, String)> = vec![
            ("response_type", "code".into()),
            ("client_id", client_id.clone()),
            ("redirect_uri", REDIRECT_URI.into()),
            ("code_challenge", pkce.challenge.clone()),
            ("code_challenge_method", "S256".into()),
            ("resource", resource.clone()),
            ("state", "s1".into()),
        ];
        for (key, value) in overrides {
            query.retain(|(k, _)| k != key);
            if !value.is_empty() {
                query.push((key, (*value).into()));
            }
        }
        query
    };
    let get = |query: Vec<(&'static str, String)>| {
        let h = &h;
        async move {
            let pairs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            h.get_with_query("/authorize", &pairs).await
        }
    };

    // Before the redirect URI is trusted, errors are not redirected.
    let response = get(request(&[("redirect_uri", "http://evil.example/cb")])).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = get(request(&[("client_id", "prc_unknown")])).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"], "invalid_client");

    for (overrides, error) in [
        (&[("code_challenge_method", "plain")][..], "invalid_request"),
        (&[("code_challenge", "")][..], "invalid_request"),
        (&[("response_type", "token")][..], "invalid_request"),
        (&[("resource", "")][..], "invalid_target"),
        (
            &[("resource", "http://127.0.0.1:1/mcp")][..],
            "invalid_target",
        ),
        (&[("scope", "links.open admin")][..], "invalid_scope"),
        // Asks for something, but not enough for any role.
        (&[("scope", "links.open")][..], "invalid_scope"),
    ] {
        let response = get(request(overrides)).await;
        assert_eq!(response.status(), StatusCode::FOUND, "{overrides:?}");
        let params = redirect_params(&response);
        assert_eq!(params["error"], error, "{overrides:?}");
        assert_eq!(params["state"], "s1");
    }
    // A loopback redirect may use another port (RFC 8252).
    let response = get(request(&[(
        "redirect_uri",
        "http://127.0.0.1:4567/callback",
    )]))
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(h.approver.asked().len() == 1);
    h.finish().await;
}

#[tokio::test]
async fn codes_are_bound_to_client_verifier_redirect_and_resource_and_used_once() {
    let h = Harness::start(Role::Reader).await;
    let client_id = h.register().await;
    let other_client = h.register().await;
    let pkce = Pkce::new();
    let code = h.authorize(&client_id, &pkce, None).await["code"].clone();
    let resource = h.resource();

    let token = |form: Vec<(&'static str, String)>| {
        let h = &h;
        async move {
            let response = h
                .http
                .post(h.url("/token"))
                .form(&form)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body: Value = response.json().await.unwrap();
            (status, body)
        }
    };
    let form = |client: &str, verifier: &str, redirect: &str, resource: &str| {
        vec![
            ("grant_type", "authorization_code".to_owned()),
            ("client_id", client.to_owned()),
            ("code", code.clone()),
            ("code_verifier", verifier.to_owned()),
            ("redirect_uri", redirect.to_owned()),
            ("resource", resource.to_owned()),
        ]
    };
    let wrong_verifier = Pkce::new().verifier;
    for (attempt, error) in [
        (
            form(&other_client, &pkce.verifier, REDIRECT_URI, &resource),
            "invalid_grant",
        ),
        (
            form(&client_id, &wrong_verifier, REDIRECT_URI, &resource),
            "invalid_grant",
        ),
        (
            form(
                &client_id,
                &pkce.verifier,
                "http://127.0.0.1:9/other",
                &resource,
            ),
            "invalid_grant",
        ),
        (
            form(
                &client_id,
                &pkce.verifier,
                REDIRECT_URI,
                "http://127.0.0.1:1/mcp",
            ),
            "invalid_target",
        ),
    ] {
        let (status, body) = token(attempt).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], error, "{body}");
    }
    // Those failures did not burn the code; the right exchange works once.
    let (status, body) = token(form(&client_id, &pkce.verifier, REDIRECT_URI, &resource)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["scope"],
        "links.resolve browsers.read rules.read activity.hosts"
    );
    let (status, body) = token(form(&client_id, &pkce.verifier, REDIRECT_URI, &resource)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");
    h.finish().await;
}

#[tokio::test]
async fn tokens_do_not_refresh_and_the_token_endpoint_takes_forms_only() {
    let h = Harness::start(Role::Reader).await;
    let client_id = h.register().await;
    let post = |form: &[(&str, &str)]| {
        let form: Vec<(String, String)> = form
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let h = &h;
        async move {
            let r = h
                .http
                .post(h.url("/token"))
                .form(&form)
                .send()
                .await
                .unwrap();
            (r.status(), r.json::<Value>().await.unwrap())
        }
    };
    let (status, body) = post(&[
        ("grant_type", "refresh_token"),
        ("client_id", &client_id),
        ("refresh_token", "x"),
    ])
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant"))
    );
    let (_, body) = post(&[
        ("grant_type", "client_credentials"),
        ("client_id", &client_id),
    ])
    .await;
    assert_eq!(body["error"], "unsupported_grant_type");
    let (_, body) = post(&[("grant_type", "authorization_code"), ("client_id", "nope")]).await;
    assert_eq!(body["error"], "invalid_client");

    let response = h
        .http
        .post(h.url("/token"))
        .json(&json!({ "grant_type": "authorization_code", "client_id": client_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    h.finish().await;
}

#[tokio::test]
async fn registration_is_public_only_and_refuses_script_redirects() {
    let h = Harness::start(Role::Reader).await;
    let response = h
        .http
        .post(h.url("/register"))
        .json(&json!({
            "redirect_uris": [REDIRECT_URI],
            "token_endpoint_auth_method": "client_secret_basic",
            "grant_types": ["client_credentials"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["token_endpoint_auth_method"], "none");
    assert_eq!(body["grant_types"], json!(["authorization_code"]));
    assert!(body.get("client_secret").is_none());

    for redirect_uris in [json!(["javascript:alert(1)"]), json!([]), json!("x")] {
        let response = h
            .http
            .post(h.url("/register"))
            .json(&json!({ "redirect_uris": redirect_uris }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"], "invalid_client_metadata");
    }
    h.finish().await;
}

#[tokio::test]
async fn revocation_applies_to_the_next_request() {
    let h = Harness::start(Role::Reader).await;
    let client_id = h.register().await;
    let pkce = Pkce::new();
    let code = h.authorize(&client_id, &pkce, None).await["code"].clone();
    let body: Value = h
        .exchange(&client_id, &code, &pkce)
        .await
        .json()
        .await
        .unwrap();
    let token = body["access_token"].as_str().unwrap().to_owned();

    let mcp_status = |token: &str| {
        let h = &h;
        let token = token.to_owned();
        async move {
            h.http
                .post(h.resource())
                .bearer_auth(token)
                .header(header::ACCEPT, "application/json, text/event-stream")
                .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(mcp_status(&token).await, StatusCode::OK);

    // Another client cannot revoke it; RFC 7009 still answers 200.
    let other = h.register().await;
    let response = h
        .http
        .post(h.url("/revoke"))
        .form(&[("client_id", other.as_str()), ("token", token.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(mcp_status(&token).await, StatusCode::OK);

    // Its own client can.
    let response = h
        .http
        .post(h.url("/revoke"))
        .form(&[("client_id", client_id.as_str()), ("token", token.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(mcp_status(&token).await, StatusCode::UNAUTHORIZED);
    let session = &h.access.sessions()[0];
    assert!(session.revoked_at.is_some());

    // Revoke from Settings, on a second session.
    let token = h.login(None).await;
    assert_eq!(mcp_status(&token).await, StatusCode::OK);
    let id = h.access.sessions()[0].id.clone();
    assert!(h.access.revoke(&id).unwrap());
    assert_eq!(mcp_status(&token).await, StatusCode::UNAUTHORIZED);
    h.finish().await;
}

#[tokio::test]
async fn sessions_and_clients_survive_a_restart() {
    let h = Harness::start(Role::Opener).await;
    let token = h.login(Some(ALL_SCOPES)).await;
    let client_id = h.register().await;
    let dir = h.stop().await;

    let h = Harness::start_in(dir, Arc::new(FakeApprover::approving(Role::Reader))).await;
    let client = h.client(&token).await;
    assert_eq!(client.list_all_tools().await.unwrap().len(), 6);
    client.cancel().await.unwrap();
    // The registration is remembered too, so the client need not register again.
    let params = h.authorize(&client_id, &Pkce::new(), None).await;
    assert!(params.contains_key("code"));
    h.finish().await;
}

#[tokio::test]
async fn stopping_cancels_waiting_approvals() {
    let h = Harness::start(Role::Reader).await;
    h.approver.set(FakeAnswer::Hold);
    let client_id = h.register().await;
    let pkce = Pkce::new();
    let resource = h.resource();
    let response = h
        .get_with_query(
            "/authorize",
            &[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT_URI),
                ("code_challenge", &pkce.challenge),
                ("code_challenge_method", "S256"),
                ("resource", &resource),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let approver = h.approver.clone();
    let id = approver.asked()[0].id.clone();
    let (_, reply) = approver.take_pending().pop().unwrap();
    h.finish().await;
    assert_eq!(approver.cancelled(), [id]);
    // Nothing waits for an answer any more.
    for _ in 0..100 {
        if !reply.is_pending() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!reply.is_pending());
}
