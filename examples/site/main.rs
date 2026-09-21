//! The demo site: what `examples/features` drive and what `tests/e2e.rs`
//! runs against. Plain HTML and vanilla JavaScript served by axum, with a
//! small JSON API and a WebSocket echo — enough surface for every step.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Default)]
struct App {
    orders: Mutex<HashMap<String, String>>,
    next_order: Mutex<u64>,
}

type Shared = Arc<App>;

fn page(name: &str) -> &'static str {
    match name {
        "login" => include_str!("static/login.html"),
        "dashboard" => include_str!("static/dashboard.html"),
        "order" => include_str!("static/order.html"),
        "profile" => include_str!("static/profile.html"),
        "broken" => include_str!("static/broken.html"),
        _ => include_str!("static/live.html"),
    }
}

fn session(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies
        .split(';')
        .map(str::trim)
        .find_map(|c| c.strip_prefix("session="))
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

async fn login_page() -> Html<&'static str> {
    Html(page("login"))
}

#[derive(Deserialize)]
struct Login {
    email: String,
    password: String,
}

async fn login(Form(form): Form<Login>) -> Response {
    if form.password == "secret" && !form.email.is_empty() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::SET_COOKIE,
            format!("session={}; Path=/", form.email)
                .parse()
                .expect("cookie"),
        );
        (headers, Redirect::to("/dashboard")).into_response()
    } else {
        Redirect::to("/login?error=1").into_response()
    }
}

async fn logout() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        "session=; Path=/; Max-Age=0".parse().expect("cookie"),
    );
    (headers, Redirect::to("/login")).into_response()
}

async fn dashboard(headers: HeaderMap) -> Response {
    match session(&headers) {
        Some(email) => Html(page("dashboard").replace("{{email}}", &email)).into_response(),
        None => Redirect::to("/login").into_response(),
    }
}

async fn order_page(headers: HeaderMap) -> Response {
    match session(&headers) {
        Some(_) => Html(page("order")).into_response(),
        None => Redirect::to("/login").into_response(),
    }
}

async fn create_order(State(app): State<Shared>, headers: HeaderMap) -> Response {
    if session(&headers).is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "sign in first"})),
        )
            .into_response();
    }
    let id = {
        let mut n = app.next_order.lock().expect("counter");
        *n += 1;
        format!("ord-{n}")
    };
    app.orders
        .lock()
        .expect("orders")
        .insert(id.clone(), "paid".to_string());
    (
        StatusCode::CREATED,
        Json(json!({"id": id, "status": "paid"})),
    )
        .into_response()
}

async fn read_order(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    match app.orders.lock().expect("orders").get(&id) {
        Some(status) => Json(json!({"id": id, "status": status})).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no such order"})),
        )
            .into_response(),
    }
}

async fn profile_page() -> Html<&'static str> {
    Html(page("profile"))
}

#[derive(Deserialize)]
struct Profile {
    country: String,
    #[serde(default)]
    newsletter: Option<String>,
    #[serde(default)]
    bio: String,
}

async fn save_profile(Form(form): Form<Profile>) -> Html<String> {
    let newsletter = if form.newsletter.is_some() {
        "yes"
    } else {
        "no"
    };
    Html(format!(
        "<!doctype html><title>Profile saved</title><h1>Profile saved</h1>\
         <p data-test=\"summary\">country={} newsletter={newsletter} bio={}</p><a href=\"/profile\">Back</a>",
        form.country, form.bio
    ))
}

async fn broken() -> Html<&'static str> {
    Html(page("broken"))
}

async fn live() -> Html<&'static str> {
    Html(page("live"))
}

async fn ws(upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(echo)
}

async fn echo(mut socket: WebSocket) {
    while let Some(Ok(message)) = socket.recv().await {
        if let Message::Text(text) = message {
            let reply = format!("echo: {text}");
            if socket.send(Message::Text(reply.into())).await.is_err() {
                break;
            }
        }
    }
}

fn router() -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/login") }))
        .route("/login", get(login_page).post(login))
        .route("/logout", get(logout))
        .route("/dashboard", get(dashboard))
        .route("/orders/new", get(order_page))
        .route("/api/orders", post(create_order))
        .route("/api/orders/{id}", get(read_order))
        .route("/profile", get(profile_page).post(save_profile))
        .route("/broken", get(broken))
        .route("/live", get(live))
        .route("/ws", get(ws))
        .with_state(Arc::new(App::default()))
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mut port: u16 = 3000;
    while let Some(arg) = args.next() {
        if arg == "--port" {
            port = args.next().and_then(|p| p.parse().ok()).unwrap_or(port);
        }
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind");
    // The first line of stdout is the port: `--port 0` lets a test pick a free one.
    println!("{}", listener.local_addr().expect("addr").port());
    eprintln!(
        "bddkit demo site on http://127.0.0.1:{}",
        listener.local_addr().expect("addr").port()
    );
    axum::serve(listener, router()).await.expect("serve");
}
