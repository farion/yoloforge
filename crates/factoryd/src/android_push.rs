//! Native Android push. Google receives only a registration and opaque inbox identifiers.
use super::*;
use axum::Extension;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const PACKAGE: &str = "com.trigonmedia.yoloforge";
const OAUTH_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Clone, Serialize, Deserialize)]
struct PublicConfig {
    project_id: String,
    sender_id: String,
    application_id: String,
    api_key: String,
}

#[derive(Serialize, Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Config {
    pub enabled: bool,
    label: String,
    public: PublicConfig,
    account: ServiceAccount,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsUpdate {
    enabled: bool,
    label: String,
    app_json: Option<String>,
    service_json: Option<String>,
}

fn invalid() -> StoreError {
    StoreError::Invalid("Firebase Android app configuration invalid")
}

fn invalid_account() -> StoreError {
    StoreError::Invalid("Firebase service account key invalid")
}

fn public_config(raw: &str) -> store_result::Result<PublicConfig> {
    if raw.len() > 65_536 {
        return Err(invalid());
    }
    let doc: Value = serde_json::from_str(raw).map_err(|_| invalid())?;
    let project = &doc["project_info"];
    let client = doc["client"]
        .as_array()
        .and_then(|clients| {
            clients
                .iter()
                .find(|c| c["client_info"]["android_client_info"]["package_name"] == PACKAGE)
        })
        .ok_or_else(invalid)?;
    let key = client["api_key"]
        .as_array()
        .and_then(|keys| keys.first())
        .ok_or_else(invalid)?;
    fn text(value: &Value) -> store_result::Result<String> {
        Ok(value
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 256
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
            })
            .ok_or_else(invalid)?
            .into())
    }
    let config = PublicConfig {
        project_id: text(&project["project_id"])?,
        sender_id: text(&project["project_number"])?,
        application_id: text(&client["client_info"]["mobilesdk_app_id"])?,
        api_key: text(&key["current_key"])?,
    };
    if !config
        .project_id
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || !config.sender_id.bytes().all(|b| b.is_ascii_digit())
        || !config
            .application_id
            .starts_with(&format!("1:{}:android:", config.sender_id))
    {
        return Err(invalid());
    }
    Ok(config)
}

impl ServiceAccount {
    fn signing_key(&self) -> Option<RsaKeyPair> {
        let pem = self
            .private_key
            .trim()
            .strip_prefix("-----BEGIN PRIVATE KEY-----")?
            .strip_suffix("-----END PRIVATE KEY-----")?;
        let data = STANDARD
            .decode(pem.split_ascii_whitespace().collect::<String>())
            .ok()?;
        RsaKeyPair::from_pkcs8(&data).ok()
    }
}

impl Config {
    pub fn sender(&self) -> &str {
        &self.public.application_id
    }

    pub fn status(&self) -> Value {
        json!({"enabled":self.enabled,"label":self.label,"configured":true,"project_id":self.public.project_id})
    }
}

fn owner(app: &App, who: &Principal) -> String {
    who.device_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| bootstrap_owner(app.token_hash))
}

fn bootstrap_owner(hash: Option<[u8; 32]>) -> String {
    hash.map(|bytes| {
        "root:".to_owned() + &bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    })
    .unwrap_or_default()
}

mod store_result {
    pub type Result<T> = std::result::Result<T, super::StoreError>;
}

impl SettingsUpdate {
    pub fn apply(self, previous: Option<Config>) -> store_result::Result<Config> {
        let label = self.label.trim();
        if !valid_text(label, 100) {
            return Err(StoreError::Invalid("Factory notification name required"));
        }
        let (old_public, old_account) = previous
            .map(|c| (Some(c.public), Some(c.account)))
            .unwrap_or_default();
        let public = match self.app_json {
            Some(raw) => public_config(&raw)?,
            None => old_public.ok_or_else(invalid)?,
        };
        let account: ServiceAccount = match self.service_json {
            Some(raw) if raw.len() <= 65_536 => {
                serde_json::from_str(&raw).map_err(|_| invalid_account())?
            }
            Some(_) => return Err(invalid_account()),
            None => old_account.ok_or_else(invalid_account)?,
        };
        if account.kind != "service_account"
            || !valid_text(&account.client_email, 320)
            || !account.client_email.ends_with(".gserviceaccount.com")
            || account.signing_key().is_none()
        {
            return Err(invalid_account());
        }
        if account.project_id != public.project_id {
            return Err(StoreError::Invalid("Firebase project IDs do not match"));
        }
        Ok(Config {
            enabled: self.enabled,
            label: label.into(),
            public,
            account,
        })
    }
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

fn admin_required() -> Response {
    error(
        StatusCode::FORBIDDEN,
        ErrorCode::Unauthorized,
        "Factory administrator required",
    )
}

pub(super) async fn settings(
    State(app): State<App>,
    Extension(who): Extension<Principal>,
) -> Response {
    if who.role != DeviceRole::Admin {
        return admin_required();
    }
    no_store(
        db_call(RequestId::new(), move || {
            Ok(app.store.android_push_config()?.map_or_else(
                || json!({"enabled":false,"configured":false,"label":""}),
                |c| c.status(),
            ))
        })
        .await,
    )
}

pub(super) async fn save(
    State(app): State<App>,
    Extension(who): Extension<Principal>,
    Json(input): Json<SettingsUpdate>,
) -> Response {
    if who.role != DeviceRole::Admin {
        return admin_required();
    }
    no_store(db_call(RequestId::new(), move || app.store.save_android_push(input)).await)
}

pub(super) async fn config(State(app): State<App>) -> Response {
    no_store(
        db_call(RequestId::new(), move || {
            Ok(app
                .store
                .android_push_config()?
                .filter(|c| c.enabled)
                .map_or_else(
                    || json!({"enabled":false}),
                    |c| json!({"enabled":true,"firebase":c.public}),
                ))
        })
        .await,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Registration {
    pub sender: String,
    pub token: String,
}

pub(super) async fn register(
    State(app): State<App>,
    Extension(who): Extension<Principal>,
    Path(id): Path<RequestId>,
    Json(input): Json<Registration>,
) -> Response {
    let owner = owner(&app, &who);
    no_store(
        db_call(RequestId::new(), move || {
            app.store
                .register_android_push(&id.to_string(), &owner, input)?;
            Ok(json!({"registered":true}))
        })
        .await,
    )
}

pub(super) async fn unregister(
    State(app): State<App>,
    Extension(who): Extension<Principal>,
    Path(id): Path<RequestId>,
) -> Response {
    let owner = owner(&app, &who);
    no_store(
        db_call(RequestId::new(), move || {
            app.store.unregister_android_push(&id.to_string(), &owner)?;
            Ok(json!({"registered":false}))
        })
        .await,
    )
}

#[derive(Clone)]
pub(crate) struct Entry {
    pub key: String,
    pub kind: &'static str,
    pub id: String,
}
pub(crate) struct Target {
    pub id: String,
    pub token: String,
}
pub(crate) enum Delivery {
    Sent,
    Unregistered,
    Retry,
}

fn assertion(account: &ServiceAccount, now: u64) -> Option<String> {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "iss":account.client_email,"scope":"https://www.googleapis.com/auth/firebase.messaging",
            "aud":OAUTH_URL,"iat":now,"exp":now+3600
        }))
        .ok()?,
    );
    let input = format!("{header}.{claims}");
    let key = account.signing_key()?;
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .ok()?;
    Some(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

#[derive(Deserialize)]
struct AccessToken {
    access_token: String,
    expires_in: u64,
}

async fn exchange(client: &reqwest::Client, account: &ServiceAccount) -> Option<AccessToken> {
    let assertion = assertion(account, unix_ms() / 1000)?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")
        .append_pair("assertion", &assertion)
        .finish();
    client
        .post(OAUTH_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()
}

fn message(config: &Config, target: &Target, entry: &Entry) -> Value {
    json!({"message":{"token":target.token,"android":{"priority":"HIGH","ttl":"3600s"},"data":{
        "registration":target.id,"factory":config.label,"kind":entry.kind,"id":entry.id
    }}})
}

async fn deliver(
    client: &reqwest::Client,
    config: &Config,
    token: &str,
    target: &Target,
    entry: &Entry,
) -> Delivery {
    let endpoint = format!(
        "https://fcm.googleapis.com/v1/projects/{}/messages:send",
        config.public.project_id
    );
    let Ok(response) = client
        .post(endpoint)
        .bearer_auth(token)
        .json(&message(config, target, entry))
        .send()
        .await
    else {
        return Delivery::Retry;
    };
    if response.status().is_success() {
        return Delivery::Sent;
    }
    if response.status() == StatusCode::NOT_FOUND
        && response.json::<Value>().await.ok().and_then(|value| {
            Some(value["error"]["details"].as_array()?.iter().any(|detail| {
                detail["@type"] == "type.googleapis.com/google.firebase.fcm.v1.FcmError"
                    && detail["errorCode"] == "UNREGISTERED"
            }))
        }) == Some(true)
    {
        return Delivery::Unregistered;
    }
    Delivery::Retry
}

pub async fn run(store: Store, stopping: Arc<AtomicBool>, factory_token: &str) {
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
    else {
        return;
    };
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut access: Option<([u8; 32], u64, Option<AccessToken>)> = None;
    let bootstrap = bootstrap_owner(
        (!factory_token.is_empty()).then(|| Sha256::digest(factory_token.as_bytes()).into()),
    );
    loop {
        interval.tick().await;
        if stopping.load(Ordering::Relaxed) {
            return;
        }
        let db = store.clone();
        let bootstrap = bootstrap.clone();
        let Ok(Ok(Some((config, batch)))) = tokio::task::spawn_blocking(move || {
            let Some(config) = db.android_push_config()?.filter(|c| c.enabled) else {
                return Ok(None);
            };
            let batch = db.android_push_batch(config.sender(), &bootstrap, unix_ms())?;
            Ok::<_, StoreError>(Some((config, batch)))
        })
        .await
        else {
            access = None;
            continue;
        };
        if batch.is_empty() {
            continue;
        }
        let fingerprint = Sha256::digest(config.account.private_key.as_bytes()).into();
        let now = unix_ms() / 1000;
        if access
            .as_ref()
            .is_none_or(|(key, expiry, _)| *key != fingerprint || *expiry <= now)
        {
            let token = exchange(&client, &config.account).await;
            if token.is_none() {
                eprintln!("Android push: Google authorization failed; retry scheduled");
            }
            let expiry = token
                .as_ref()
                .map_or(60, |t| t.expires_in.saturating_sub(30));
            access = Some((fingerprint, now + expiry, token));
        }
        let Some((_, _, Some(token))) = &access else {
            continue;
        };
        use futures_util::{stream, StreamExt};
        stream::iter(batch)
            .for_each_concurrent(8, |(target, entry)| {
                let (client, config, db, bearer) =
                    (&client, &config, store.clone(), token.access_token.as_str());
                let stopping = &stopping;
                async move {
                    if stopping.load(Ordering::Relaxed) {
                        return;
                    }
                    let result = deliver(client, config, bearer, &target, &entry).await;
                    if matches!(result, Delivery::Retry) {
                        eprintln!("Android push: delivery unconfirmed; retry scheduled");
                    }
                    let _ = tokio::task::spawn_blocking(move || {
                        db.android_push_result(target, entry, result)
                    })
                    .await;
                }
            })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::{Client, Method};

    fn app_json(package: &str) -> Value {
        json!({
            "project_info":{"project_number":"123","project_id":"example-push"},
            "client":[{
                "client_info":{"mobilesdk_app_id":"1:123:android:abc","android_client_info":{"package_name":package}},
                "api_key":[{"current_key":"public-key"}]
            }]
        })
    }

    fn sample() -> Config {
        Config {
            enabled: true,
            label: "Werkstatt".into(),
            public: public_config(&app_json(PACKAGE).to_string()).unwrap(),
            account: ServiceAccount {
                project_id: "example-push".into(),
                client_email: "account@example-push.iam.gserviceaccount.com".into(),
                private_key: "TEST ONLY - NOT A REAL KEY".into(),
                kind: "service_account".into(),
            },
        }
    }

    #[test]
    fn google_payload_contains_no_factory_credentials_or_entry_text() {
        assert!(public_config(&app_json("com.other.app").to_string()).is_err());
        let data = message(
            &sample(),
            &Target {
                id: "registration-id".into(),
                token: "fcm-token".into(),
            },
            &Entry {
                id: "inbox-id".into(),
                kind: "question",
                key: "internal-key".into(),
            },
        );
        assert_eq!(
            data["message"]["data"],
            json!({
                "id":"inbox-id","registration":"registration-id","kind":"question","factory":"Werkstatt"
            })
        );
        assert!(
            data["message"].get("notification").is_none(),
            "the app must validate its active binding before display"
        );
        assert!(!data.to_string().contains("internal-key"));
    }

    #[tokio::test]
    async fn firebase_settings_are_admin_only_and_credentials_are_write_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let credentials = || {
            store
                .create_device(CreateDevice {
                    name: "Android".into(),
                    role: DeviceRole::Operator,
                    runner_id: None,
                })
                .unwrap()
                .token
        };
        let user = credentials();
        let other = credentials();
        let admin = "test-admin-access-key-not-for-production";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let app = router(admin, store);
        let service = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = Client::new();
        let request = |method, path, token| {
            http.request(method, format!("{origin}{path}"))
                .header(VERSION_HEADER, "1")
                .bearer_auth(token)
        };
        let settings = "/v1/android-push/settings";
        let config = "/v1/android-push/config";
        let path = format!("/v1/android-push/devices/{}", RequestId::new());
        assert_eq!(
            request(Method::GET, config, "")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(Method::GET, settings, &user)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let invalid = json!({"enabled":true,"label":"Werkstatt","app_json":app_json(PACKAGE).to_string(),
            "service_json":serde_json::to_string(&sample().account).unwrap()});
        assert_eq!(
            request(Method::PUT, settings, &user)
                .json(&invalid)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(Method::PUT, settings, admin)
                .json(&invalid)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(!dir.path().join("db.android-push.json").exists());

        // Seed a test-only config; no key is usable and no Google request is made.
        std::fs::write(
            dir.path().join("db.android-push.json"),
            serde_json::to_vec(&sample()).unwrap(),
        )
        .unwrap();
        let status = request(Method::GET, settings, admin).send().await.unwrap();
        assert_eq!(status.headers()["cache-control"], "no-store");
        assert_eq!(
            status.json::<Value>().await.unwrap(),
            json!({"enabled":true,"configured":true,"label":"Werkstatt","project_id":"example-push"})
        );
        let public = request(Method::GET, config, &user)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(
            public,
            json!({"enabled":true,"firebase":{
                "project_id":"example-push","sender_id":"123","application_id":"1:123:android:abc","api_key":"public-key"
            }})
        );
        let binding = json!({"sender":"1:123:android:abc","token":"sample-token"});
        assert_eq!(
            request(Method::PUT, &path, &user)
                .json(&binding)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(Method::DELETE, &path, &other)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(Method::PUT, &path, &other)
                .json(&binding)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            request(Method::DELETE, &path, &user)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(Method::PUT, &path, &other)
                .json(&binding)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        service.abort();
    }
}
