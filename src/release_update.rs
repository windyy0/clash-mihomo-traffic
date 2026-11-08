use chrono::{DateTime, Months, TimeDelta, TimeZone, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, sync::OnceLock, time::Duration};

mod installer;
pub(crate) mod portable;
pub use installer::download_and_install;

pub const RELEASES_PAGE: &str = "https://github.com/windyy0/clash-mihomo-traffic/releases";
pub const CACHE_FILE: &str = "update-cache.json";
const API: &str = "https://api.github.com/repos/windyy0/clash-mihomo-traffic/releases";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const MAX_RESPONSE: usize = 512 * 1024;
const MAX_CACHE: u64 = 64 * 1024 * 1024;
const MAX_NOTES: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckFrequency {
    Daily,
    #[default]
    Weekly,
    Monthly,
    Never,
}

impl CheckFrequency {
    pub const ALL: [Self; 4] = [Self::Daily, Self::Weekly, Self::Monthly, Self::Never];

    pub fn label(self) -> &'static str {
        match self {
            Self::Daily => "每日",
            Self::Weekly => "每周",
            Self::Monthly => "每月",
            Self::Never => "永不",
        }
    }

    pub fn is_due<Tz: TimeZone>(self, now: DateTime<Tz>, last: Option<DateTime<Tz>>) -> bool {
        let Some(last) = last else {
            return self != Self::Never;
        };
        let next = match self {
            Self::Daily => last.to_utc().checked_add_signed(TimeDelta::days(1)),
            Self::Weekly => last.to_utc().checked_add_signed(TimeDelta::days(7)),
            Self::Monthly => last
                .fixed_offset()
                .checked_add_months(Months::new(1))
                .map(|next| {
                    // During a clock change, use the first occurrence or carry
                    // the previous offset through a skipped local hour.
                    next.naive_local()
                        .and_local_timezone(last.timezone())
                        .earliest()
                        .map_or_else(|| next.to_utc(), |local| local.to_utc())
                }),
            Self::Never => None,
        };
        next.is_some_and(|time| now >= time)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum CheckError {
    Unavailable,
    RateLimited,
    Network,
    Invalid,
}

impl CheckError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "暂无可读取的正式发行版，或仓库暂不可访问。",
            Self::RateLimited => "GitHub 拒绝了请求或已达请求上限，请稍后重试。",
            Self::Network => "无法连接 GitHub，请检查网络后重试。",
            Self::Invalid => "GitHub 返回的发行版数据无法读取，请稍后重试。",
        }
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Clone, Deserialize)]
struct GitHubAsset {
    name: String,
    state: String,
    browser_download_url: String,
    digest: Option<String>,
}

impl GitHubRelease {
    fn matching_asset(&self, version: &Version, portable: bool) -> Option<&GitHubAsset> {
        let name = asset_name(version, portable);
        let url = format!("{RELEASES_PAGE}/download/{}/{name}", self.tag_name);
        self.assets.iter().find(|asset| {
            asset.name == name && asset.state == "uploaded" && asset.browser_download_url == url
        })
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Release {
    tag: String,
    notes: String,
    installer: bool,
    portable: bool,
    #[serde(skip)]
    html: OnceLock<String>,
}

impl Release {
    fn parse_tag(tag: &str) -> Option<Version> {
        let version = Version::parse(tag.strip_prefix('v')?).ok()?;
        version.pre.is_empty().then_some(version)
    }

    fn parse(release: GitHubRelease) -> Result<Self, CheckError> {
        let version = Self::parse_tag(&release.tag_name).ok_or(CheckError::Invalid)?;
        if release.draft || release.prerelease {
            return Err(CheckError::Invalid);
        }
        let installer = release.matching_asset(&version, false).is_some();
        let portable = release.matching_asset(&version, true).is_some();
        let mut result = Self {
            tag: release.tag_name,
            notes: release.body.unwrap_or_default(),
            installer,
            portable,
            html: OnceLock::new(),
        };
        result.limit_notes();
        Ok(result)
    }

    fn limit_notes(&mut self) {
        if self.notes.len() > MAX_NOTES {
            self.notes
                .truncate(self.notes.floor_char_boundary(MAX_NOTES));
            self.notes
                .push_str("\n\n说明过长，请在 GitHub 查看完整内容。");
        }
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn notes_html(&self) -> &str {
        self.html.get_or_init(|| {
            // Keep external links and media in the application's GitHub controls.
            let mut options = markdown::Options::gfm();
            let constructs = &mut options.parse.constructs;
            constructs.autolink = false;
            constructs.gfm_autolink_literal = false;
            constructs.label_start_image = false;
            constructs.label_start_link = false;
            markdown::to_html_with_options(&self.notes, &options)
                .map(|html| display_list_paragraphs(&html))
                .unwrap_or_default()
        })
    }

    pub fn matches_current(&self) -> bool {
        Self::parse_tag(&self.tag).is_some_and(|version| version == current())
    }

    pub fn is_newer(&self) -> bool {
        Self::parse_tag(&self.tag).is_some_and(|version| version.cmp_precedence(&current()).is_gt())
    }

    pub fn page_url(&self) -> String {
        format!("{RELEASES_PAGE}/tag/{}", self.tag)
    }

    pub fn download_url(&self, portable: bool) -> Option<String> {
        let available = if portable {
            self.portable
        } else {
            self.installer
        };
        let version = Self::parse_tag(&self.tag)?;
        available.then(|| {
            let name = asset_name(&version, portable);
            format!("{RELEASES_PAGE}/download/{}/{name}", self.tag)
        })
    }
}

// GPUI's rich text view can clip the text beside its list marker. The Markdown
// compiler has already escaped raw HTML, so convert its list tags to blocks
// with textual markers and leave inline formatting and other blocks intact.
fn display_list_paragraphs(mut input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut lists: Vec<Option<u32>> = Vec::new();
    while let Some((text, element)) = input.split_once('<') {
        let Some((tag, rest)) = element.split_once('>') else {
            break;
        };
        output.push_str(text);
        input = rest;
        match tag {
            "ul" => {
                lists.push(None);
                output.push_str("<div>");
            }
            tag if tag == "ol" || tag.starts_with("ol start=") => {
                let first = tag
                    .strip_prefix("ol start=\"")
                    .and_then(|value| value.strip_suffix('"'))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1);
                lists.push(Some(first));
                output.push_str("<div>");
            }
            "/ul" | "/ol" => {
                lists.pop();
                output.push_str("</div>");
            }
            "li" => {
                output.push_str("<div>");
                if let Some(paragraph) = input.trim_start().strip_prefix("<p>") {
                    output.push_str("<p>");
                    input = paragraph;
                }
                for _ in 1..lists.len() {
                    output.push_str("&nbsp;&nbsp;");
                }
                if let Some(Some(index)) = lists.last_mut() {
                    use std::fmt::Write;
                    let _ = write!(output, "{index}.&nbsp;");
                    *index = index.saturating_add(1);
                } else {
                    output.push_str("•&nbsp;");
                }
            }
            "/li" => output.push_str("</div>"),
            _ => {
                output.push('<');
                output.push_str(tag);
                output.push('>');
            }
        }
    }
    output.push_str(input);
    output
}

fn asset_name(version: &Version, portable: bool) -> String {
    let suffix = if portable {
        "portable.zip"
    } else {
        "setup.exe"
    };
    format!("MihomoTraffic-{version}-windows-x64-{suffix}")
}

fn current() -> Version {
    Version::parse(CURRENT).expect("Cargo package version is valid SemVer")
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Cache {
    pub latest: Option<Release>,
    pub current: Option<Release>,
    pub last_attempt: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub last_error: Option<CheckError>,
    pub seen_tag: Option<String>,
    pub dismissed_tag: Option<String>,
}

#[derive(Deserialize)]
struct CacheFile {
    #[serde(flatten)]
    cache: Cache,
    history: Option<Vec<Release>>,
}

impl Cache {
    pub fn load(dir: &Path) -> Self {
        let Ok(file) = std::fs::File::open(dir.join(CACHE_FILE)) else {
            return Self::default();
        };
        let Ok(CacheFile { mut cache, history }) = serde_json::from_reader(file.take(MAX_CACHE))
        else {
            return Self::default();
        };
        cache.latest = cache
            .latest
            .filter(|release| Release::parse_tag(&release.tag).is_some());
        cache.current = cache.current.filter(Release::matches_current);
        // Recover only the installed version's notes from an older cache file.
        if cache.current_release().is_none() {
            cache.current =
                history.and_then(|history| history.into_iter().find(Release::matches_current));
        }
        for release in cache.latest.iter_mut().chain(cache.current.iter_mut()) {
            release.limit_notes();
        }
        cache
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        let mut file = tempfile::Builder::new()
            .prefix(".mihomo-traffic-update-")
            .suffix(".tmp")
            .tempfile_in(dir)?;
        serde_json::to_writer(file.as_file_mut(), self)?;
        anyhow::ensure!(
            file.as_file().metadata()?.len() <= MAX_CACHE,
            "release cache is too large"
        );
        file.persist(dir.join(CACHE_FILE))?;
        Ok(())
    }

    pub fn current_release(&self) -> Option<&Release> {
        self.current.as_ref().or_else(|| {
            self.latest
                .as_ref()
                .filter(|release| release.matches_current())
        })
    }

    pub fn unread_update(&self) -> Option<&Release> {
        self.update_notice()
            .filter(|release| newer_than_tag(release, self.seen_tag.as_deref()))
    }

    pub fn update_notice(&self) -> Option<&Release> {
        self.latest.as_ref().filter(|release| {
            release.is_newer() && newer_than_tag(release, self.dismissed_tag.as_deref())
        })
    }
}

fn newer_than_tag(release: &Release, tag: Option<&str>) -> bool {
    tag.and_then(Release::parse_tag).is_none_or(|previous| {
        Release::parse_tag(release.tag())
            .is_some_and(|version| version.cmp_precedence(&previous).is_gt())
    })
}

fn github_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("MihomoTraffic/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
}

fn fetch_with<T>(
    request: impl AsyncFnOnce(&reqwest::Client) -> Result<T, CheckError>,
) -> Result<T, CheckError> {
    let client = github_client()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| CheckError::Network)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| CheckError::Network)?;
    runtime.block_on(request(&client))
}

pub fn fetch_latest() -> Result<Release, CheckError> {
    fetch_with(async |client| request_latest(client, API).await)
}

pub fn fetch_current() -> Result<Release, CheckError> {
    fetch_with(async |client| {
        Release::parse(request_tag(client, API, concat!("v", env!("CARGO_PKG_VERSION"))).await?)
    })
}

async fn request_latest(client: &reqwest::Client, api: &str) -> Result<Release, CheckError> {
    let release = get_release(client.get(format!("{api}/latest"))).await?;
    Release::parse(release)
}

async fn request_tag(
    client: &reqwest::Client,
    api: &str,
    tag: &str,
) -> Result<GitHubRelease, CheckError> {
    let release = get_release(client.get(format!("{api}/tags/{tag}"))).await?;
    if release.tag_name != tag || release.draft || release.prerelease {
        return Err(CheckError::Invalid);
    }
    Ok(release)
}

async fn get_release(request: reqwest::RequestBuilder) -> Result<GitHubRelease, CheckError> {
    let mut response = request
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| CheckError::Network)?;
    match response.status().as_u16() {
        200 => {}
        404 => return Err(CheckError::Unavailable),
        403 | 429 => return Err(CheckError::RateLimited),
        _ => return Err(CheckError::Network),
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| CheckError::Network)? {
        if chunk.len() > MAX_RESPONSE - bytes.len() {
            return Err(CheckError::Invalid);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| CheckError::Invalid)
}

#[cfg(test)]
#[path = "release_update_tests.rs"]
mod tests;
