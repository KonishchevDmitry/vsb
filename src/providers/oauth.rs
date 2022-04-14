use std::error::Error;
use std::fmt;
use std::sync::Mutex;
use std::time::{Instant, Duration};

use log::debug;
use serde_derive::{Serialize, Deserialize};

use crate::core::GenericResult;
use crate::http_client::{HttpClient, HttpRequest, Method, headers};

pub struct OauthClient {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    access_token: Mutex<Option<AccessToken>>,

    url: String,
    client: HttpClient,
}

struct AccessToken {
    token: String,
    expire_time: Instant,
}

const API_REQUEST_TIMEOUT: u64 = 5;
const ACCESS_TOKEN_MIN_EXPIRE_TIME: u64 = 60;

impl OauthClient {
    pub fn new(url: &str, client_id: &str, client_secret: &str, refresh_token: &str) -> OauthClient {
        OauthClient {
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            refresh_token: refresh_token.to_owned(),
            access_token: Mutex::new(None),

            url: url.to_owned(),
            client: HttpClient::new(),
        }
    }

    pub fn authenticate<'a, R, E>(&self, request: HttpRequest<'a, R, E>) -> GenericResult<HttpRequest<'a, R, E>> {
        let access_token = self.get_access_token().map_err(|e| format!(
            "Unable obtain OAuth token: {}", e))?;

        Ok(request.with_header(headers::AUTHORIZATION, format!("Bearer {}", access_token))
            .map_err(|_| "Got an invalid OAuth token")?)
    }

    fn get_access_token(&self) -> GenericResult<String> {
        let mut access_token = self.access_token.lock().unwrap();

        if let Some(ref access_token) = *access_token {
            if Instant::now() + Duration::from_secs(ACCESS_TOKEN_MIN_EXPIRE_TIME) <= access_token.expire_time {
                return Ok(access_token.token.to_owned());
            }
        }

        debug!("Obtaining a new OAuth access token...");

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

        let request = HttpRequest::<Response, OauthApiError>::new_json(
            Method::POST, format!("{}/token", self.url),
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
struct OauthApiError {
    error_description: String,
}

impl Error for OauthApiError {
}

impl fmt::Display for OauthApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "OAuth error: {}", self.error_description)
    }
}