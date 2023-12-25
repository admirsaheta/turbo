#![feature(min_specialization)]
#![feature(arbitrary_self_types)]

use anyhow::Result;
use turbo_tasks::{register, value, Vc};
use turbo_tasks_fs::FileSystemPath;
use turbopack_core::issue::{Issue, IssueSeverity, OptionStyledString, StyledString};
use reqwest::Client;

register!();

#[value(transparent)]
pub struct FetchResult(Result<Vc<HttpResponse>, Vc<FetchError>>);

#[value(shared)]
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vc<HttpResponseBody>,
}

#[value(shared)]
#[derive(Debug)]
pub struct HttpResponseBody(pub Vec<u8>);

#[value_impl]
impl HttpResponseBody {
    #[function]
    pub async fn to_string(self: Vc<Self>) -> Result<Vc<String>> {
        let this = &*self.await?;
        Ok(Vc::cell(String::from_utf8_lossy(&this.0).to_string()))
    }
}

#[function]
pub async fn fetch(url: Vc<String>, user_agent: Vc<Option<String>>) -> Result<Vc<FetchResult>> {
    let url = url.await?;
    let user_agent = user_agent.await?;
    let client = Client::new();

    let mut builder = client.get(&url);
    if let Some(user_agent) = &user_agent {
        builder = builder.header("User-Agent", user_agent);
    }

    let response = builder.send().await.and_then(|r| r.error_for_status());
    match response {
        Ok(response) => {
            let status = response.status().as_u16();
            let body = response.bytes().await?.to_vec();

            Ok(Vc::cell(Ok(HttpResponse {
                status,
                body: Vc::cell(HttpResponseBody(body)),
            })))
        }
        Err(err) => Ok(Vc::cell(Err(FetchError::from_reqwest_error(&err, &url).into()))),
    }
}

#[derive(Debug)]
#[value(shared)]
pub enum FetchErrorKind {
    Connect,
    Timeout,
    Status(u16),
    Other,
}

#[value(shared)]
pub struct FetchError {
    pub url: Vc<String>,
    pub kind: Vc<FetchErrorKind>,
    pub detail: Vc<StyledString>,
}

impl FetchError {
    fn from_reqwest_error(error: &reqwest::Error, url: &str) -> FetchError {
        let kind = if error.is_connect() {
            FetchErrorKind::Connect
        } else if error.is_timeout() {
            FetchErrorKind::Timeout
        } else if let Some(status) = error.status() {
            FetchErrorKind::Status(status.as_u16())
        } else {
            FetchErrorKind::Other
        };

        FetchError {
            detail: StyledString::Text(error.to_string()).into(),
            url: Vc::cell(url.to_owned()),
            kind: kind.into(),
        }
    }
}

#[value_impl]
impl FetchError {
    #[function]
    pub async fn to_issue(
        self: Vc<Self>,
        severity: Vc<IssueSeverity>,
        issue_context: Vc<FileSystemPath>,
    ) -> Result<Vc<FetchIssue>> {
        let this = &*self.await?;
        Ok(FetchIssue {
            issue_context,
            severity,
            url: this.url,
            kind: this.kind,
            detail: this.detail,
        }.into())
    }
}

#[value(shared)]
pub struct FetchIssue {
    pub issue_context: Vc<FileSystemPath>,
    pub severity: Vc<IssueSeverity>,
    pub url: Vc<String>,
    pub kind: Vc<FetchErrorKind>,
    pub detail: Vc<StyledString>,
}

#[value_impl]
impl Issue for FetchIssue {
    #[function]
    fn file_path(&self) -> Vc<FileSystemPath> {
        self.issue_context.clone()
    }

    #[function]
    fn severity(&self) -> Vc<IssueSeverity> {
        self.severity.clone()
    }

    #[function]
    fn title(&self) -> Vc<StyledString> {
        StyledString::Text("Error while requesting resource".to_string()).into()
    }

    #[function]
    fn category(&self) -> Vc<String> {
        Vc::cell("fetch".to_string())
    }

    #[function]
    async fn description(&self) -> Result<Vc<OptionStyledString>> {
        let url = self.url.clone();
        let kind = self.kind.clone();

        Ok(Vc::cell(Some(
            StyledString::Text(match &*kind.await? {
                FetchErrorKind::Connect => format!(
                    "There was an issue establishing a connection while requesting {}.",
                    &*url.await?
                ),
                FetchErrorKind::Status(status) => {
                    format!(
                        "Received response with status {} when requesting {}",
                        status, &*url.await?
                    )
                }
                FetchErrorKind::Timeout => format!("Connection timed out when requesting {}", &*url.await?),
                FetchErrorKind::Other => format!("There was an issue requesting {}", &*url.await?),
            }).into(),
        )))
    }

    #[function]
    fn detail(&self) -> Vc<OptionStyledString> {
        self.detail.clone().into()
    }
}
