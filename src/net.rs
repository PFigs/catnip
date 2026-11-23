//! Sends a request off the UI thread and puts the answer in an inbox that `poll` drains.

use std::sync::{Arc, Mutex};

use rune::Waker;

use crate::github::Request;

pub type Answer = Result<String, String>;

pub struct Net<M> {
    pub inbox: Arc<Mutex<Vec<M>>>,
    pub waker: Option<Waker>,
    /// The token pasted in the web build. Native looks up its own at each request.
    pub token: Option<String>,
    /// The requests a test build records in place of sending them.
    #[cfg(test)]
    pub sent: Mutex<Vec<Request>>,
}

impl<M: Send + 'static> Net<M> {
    pub fn new(token: Option<String>) -> Self {
        Net {
            inbox: Arc::default(),
            waker: None,
            token,
            #[cfg(test)]
            sent: Mutex::default(),
        }
    }

    #[cfg(test)]
    pub fn send(&self, req: Request, _done: impl FnOnce(Answer) -> M + Send + 'static) {
        self.sent.lock().unwrap().push(req);
    }

    #[cfg(not(test))]
    pub fn send(&self, req: Request, done: impl FnOnce(Answer) -> M + Send + 'static) {
        let inbox = self.inbox.clone();
        let waker = self.waker.clone();
        let deliver = move |answer| {
            inbox.lock().unwrap().push(done(answer));
            if let Some(wake) = waker {
                wake();
            }
        };
        transport::send(self.token.clone(), req, deliver);
    }

    pub fn drain(&self) -> Vec<M> {
        std::mem::take(&mut *self.inbox.lock().unwrap())
    }
}

/// The message for an answer that is not a success. `sign_in` says how to get a new token.
#[cfg(not(test))]
fn status_error(status: u16, body: &str, sign_in: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(|m| format!(": {m}")))
        .unwrap_or_default();
    match status {
        401 => format!("GitHub refused the token (401){detail}. {sign_in}"),
        403 | 429 => format!(
            "GitHub said no ({status}){detail}. The rate limit is reached, or the token has no access."
        ),
        _ => format!("GitHub answered {status}{detail}."),
    }
}

#[cfg(all(not(test), not(target_arch = "wasm32")))]
mod transport {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};

    use ureq::Agent;

    use super::{Answer, status_error};
    use crate::github::Request;

    const SIGN_IN: &str =
        "Run `gh auth login`, or set GITHUB_TOKEN to a classic token with `repo` and `read:org`.";
    const LIMIT: Duration = Duration::from_secs(30);

    /// A token found once. A failed lookup runs again at the next request.
    static TOKEN: Mutex<Option<String>> = Mutex::new(None);

    fn found(t: String) -> Option<String> {
        Some(t.trim().to_owned()).filter(|t| !t.is_empty())
    }

    /// gh on the PATH, else where Homebrew installs it: an app opened from the macOS Finder
    /// gets a PATH without Homebrew.
    const GH: [&str; 3] = ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"];

    /// The token `gh auth token` prints, or None when gh is missing or fails.
    fn gh_token() -> Result<Option<String>, String> {
        let spawn = |gh: &&str| {
            Command::new(gh)
                .args(["auth", "token"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()
        };
        let Some(mut gh) = GH.iter().find_map(spawn) else {
            return Ok(None);
        };
        let started = Instant::now();
        loop {
            match gh.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(None) if started.elapsed() < LIMIT => thread::sleep(Duration::from_millis(20)),
                Ok(None) => {
                    let _ = gh.kill();
                    let _ = gh.wait();
                    return Err("`gh auth token` did not answer in 30 seconds.".into());
                }
                _ => return Ok(None),
            }
        }
        let mut out = String::new();
        if let Some(mut stdout) = gh.stdout.take() {
            let _ = stdout.read_to_string(&mut out);
        }
        Ok(found(out))
    }

    fn token() -> Result<String, String> {
        if let Some(token) = TOKEN.lock().unwrap().clone() {
            return Ok(token);
        }
        let token = match std::env::var("GITHUB_TOKEN").ok().and_then(found) {
            Some(token) => token,
            None => gh_token()?.ok_or(format!("No GitHub token. {SIGN_IN}"))?,
        };
        *TOKEN.lock().unwrap() = Some(token.clone());
        Ok(token)
    }

    fn agent() -> &'static Agent {
        static AGENT: OnceLock<Agent> = OnceLock::new();
        AGENT.get_or_init(|| {
            let config = Agent::config_builder()
                .timeout_global(Some(LIMIT))
                .http_status_as_error(false)
                .build();
            Agent::new_with_config(config)
        })
    }

    fn fetch(req: &Request) -> Answer {
        let auth = format!("Bearer {}", token()?);
        let answer = match &req.body {
            Some(body) => agent()
                .post(&req.url)
                .header("Authorization", &auth)
                .header("Accept", req.accept)
                .header("User-Agent", "catnip")
                .send(body.as_str()),
            None => agent()
                .get(&req.url)
                .header("Authorization", &auth)
                .header("Accept", req.accept)
                .header("User-Agent", "catnip")
                .call(),
        };
        let mut answer = answer.map_err(|e| match e {
            ureq::Error::Timeout(_) => "GitHub did not answer in 30 seconds.".to_owned(),
            ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
                format!("No network: can't reach GitHub ({e}).")
            }
            e => e.to_string(),
        })?;
        let status = answer.status().as_u16();
        let body = answer
            .body_mut()
            .with_config()
            .limit(64 << 20)
            .read_to_string()
            .map_err(|e| e.to_string())?;
        if status == 401 {
            *TOKEN.lock().unwrap() = None;
        }
        if (200..300).contains(&status) {
            Ok(body)
        } else {
            Err(status_error(status, &body, SIGN_IN))
        }
    }

    pub fn send(_: Option<String>, req: Request, deliver: impl FnOnce(Answer) + Send + 'static) {
        let job = Arc::new(Mutex::new(Some((req, deliver))));
        let worker = job.clone();
        let started = rune::task::spawn("github request", move || {
            let taken = worker.lock().unwrap().take();
            if let Some((req, deliver)) = taken {
                deliver(fetch(&req));
            }
        });
        if let Err(e) = started {
            let taken = job.lock().unwrap().take();
            if let Some((_, deliver)) = taken {
                deliver(Err(format!("Can't start the request: {e}")));
            }
        }
    }
}

#[cfg(all(not(test), target_arch = "wasm32"))]
mod transport {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{RequestInit, Response};

    use super::{Answer, status_error};
    use crate::github::Request;

    const SIGN_IN: &str = "Paste a token in Settings.";

    fn text(e: JsValue) -> String {
        e.as_string().unwrap_or_else(|| format!("{e:?}"))
    }

    async fn fetch(token: Option<String>, req: Request) -> Answer {
        let token = token.ok_or(format!("No GitHub token. {SIGN_IN}"))?;
        let init = RequestInit::new();
        init.set_method(if req.body.is_some() { "POST" } else { "GET" });
        if let Some(body) = &req.body {
            init.set_body(&JsValue::from_str(body));
        }
        let request = web_sys::Request::new_with_str_and_init(&req.url, &init).map_err(text)?;
        let headers = request.headers();
        headers
            .set("Authorization", &format!("Bearer {token}"))
            .map_err(text)?;
        headers.set("Accept", req.accept).map_err(text)?;
        if req.body.is_some() {
            headers
                .set("Content-Type", "application/json")
                .map_err(text)?;
        }
        let window = web_sys::window().ok_or("No window.")?;
        let response: Response = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|_| "No network: can't reach GitHub.".to_owned())?
            .dyn_into()
            .map_err(text)?;
        let body = JsFuture::from(response.text().map_err(text)?)
            .await
            .map_err(text)?;
        let body = body.as_string().unwrap_or_default();
        if response.ok() {
            Ok(body)
        } else {
            Err(status_error(response.status(), &body, SIGN_IN))
        }
    }

    pub fn send(token: Option<String>, req: Request, deliver: impl FnOnce(Answer) + 'static) {
        wasm_bindgen_futures::spawn_local(async move { deliver(fetch(token, req).await) });
    }
}
