use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::time::{Instant, Duration};

use core::GenericResult;
use http_client::{HttpClient, HttpRequest, Method};

pub struct GoogleOauth {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    access_token: RefCell<Option<AccessToken>>,

    client: HttpClient,
}

struct AccessToken {
    token: String,
    expire_time: Instant,
}

const API_ENDPOINT: &'static str = "https://accounts.google.com/o/oauth2";
const API_REQUEST_TIMEOUT: u64 = 5;

impl GoogleOauth {
    pub fn new(client_id: &str, client_secret: &str, refresh_token: &str) -> GoogleOauth {
        GoogleOauth {
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            refresh_token: refresh_token.to_owned(),
            access_token: RefCell::new(None),

            client: HttpClient::new(),
        }
    }

    pub fn get_access_token(&self) -> GenericResult<String> {
        let mut access_token = self.access_token.borrow_mut();

        if let Some(ref access_token) = *access_token {
            let now = Instant::now();

            if access_token.expire_time > now &&
                access_token.expire_time.duration_since(now) > Duration::from_secs(1) // FIXME: Request timeout here?
            {
                return Ok(access_token.token.to_owned());
            }
        }

        debug!("Obtaining a new Google Drive access token...");

        #[derive(Serialize)]
        struct Request<'a> {
            client_id: &'a str,
            client_secret: &'a str,
            refresh_token: &'a str,
            grant_type: &'a str,
        }

        #[derive(Deserialize)]
        struct Response {
            access_token: String,
            expires_in: u64,
        }

        let request = HttpRequest::<Response, GoogleOauthApiError>::new_json(
            Method::Post, API_ENDPOINT.to_owned() + "/token",
            Duration::from_secs(API_REQUEST_TIMEOUT)
        ).with_form(&Request {
            client_id: &self.client_id,
            client_secret: &self.client_secret,
            refresh_token: &self.refresh_token,
            grant_type: "refresh_token",
        })?;

        let request_time = Instant::now();
        let response = self.client.send(request)?;

        *access_token = Some(AccessToken {
            token: response.access_token.to_owned(),
            expire_time: request_time + Duration::from_secs(response.expires_in)
        });

        Ok(response.access_token)
    }
}

#[derive(Debug, Deserialize)]
struct GoogleOauthApiError {
    error_description: String,
}

impl Error for GoogleOauthApiError {
    fn description(&self) -> &str {
        "Google OAuth error"
    }
}

impl fmt::Display for GoogleOauthApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.description(), self.error_description)
    }
}