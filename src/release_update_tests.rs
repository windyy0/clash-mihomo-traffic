use super::*;
use chrono::TimeZone;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn fixture_release(tag: &str, body: &str) -> Value {
    let version = Release::parse_tag(tag).unwrap();
    let assets: Vec<_> = [false, true]
        .into_iter()
        .map(|portable| {
            let name = asset_name(&version, portable);
            json!({
                "name": name,
                "state": "uploaded",
                "browser_download_url": format!("{RELEASES_PAGE}/download/{tag}/{name}")
            })
        })
        .collect();
    json!({"tag_name": tag, "draft": false, "prerelease": false, "body": body, "assets": assets})
}

async fn fixture_api(
    replies: Vec<(&'static str, &'static str, String, &'static str)>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://{}/releases", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for (path, status, body, extra_headers) in replies {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(connection.read_u8().await.unwrap());
                assert!(headers.len() < 4096);
            }
            let headers = std::str::from_utf8(&headers).unwrap();
            assert!(headers.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
            assert!(!headers.to_ascii_lowercase().contains("authorization:"));
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
                body.len()
            );
            connection.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (api, server)
}

fn fixture_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

async fn fixture_request(status: &'static str, body: String) -> Result<Release, CheckError> {
    let (api, server) = fixture_api(vec![("/releases/latest", status, body, "")]).await;
    let result = request_latest(&fixture_client(), &api).await;
    server.await.unwrap();
    result
}

#[tokio::test]
async fn reads_latest_release_and_matches_the_package_type() {
    let mut version = current();
    version.patch += 10;
    let newer = fixture_release(&format!("v{version}"), "## 新功能\n\n- **自动检查**");
    let release = fixture_request("200 OK", newer.to_string()).await.unwrap();
    assert!(release.is_newer());
    assert!(release.download_url(false).unwrap().ends_with("-setup.exe"));
    assert!(
        release
            .download_url(true)
            .unwrap()
            .ends_with("-portable.zip")
    );
    assert!(release.notes_html().contains("<strong>自动检查</strong>"));
    assert!(
        release
            .notes_html()
            .contains("•&nbsp;<strong>自动检查</strong>")
    );

    let published = fixture_release(&format!("v{CURRENT}"), "首版更新说明");
    let latest = fixture_request("200 OK", published.to_string())
        .await
        .unwrap();
    assert!(!latest.is_newer());
    assert!(latest.matches_current());
    assert!(latest.notes_html().contains("首版更新说明"));
}

#[tokio::test]
async fn paginates_history_and_caches_formal_releases_offline() {
    let mut version = current();
    version.patch += 1;
    let tag = format!("v{version}");
    let newer = fixture_release(&tag, "**历史中的新说明**");
    let installed = fixture_release(&format!("v{CURRENT}"), "当前版本说明");
    let mut preview = newer.clone();
    preview["prerelease"] = json!(true);
    let (api, server) = fixture_api(vec![
        (
            "/releases?per_page=20&page=1",
            "200 OK",
            json!([installed, preview]).to_string(),
            "Link: <https://api.github.com/repositories/42/releases?page=2>; rel=\"next\"\r\n",
        ),
        (
            "/releases?per_page=20&page=2",
            "200 OK",
            json!([newer]).to_string(),
            "",
        ),
    ])
    .await;
    let history = request_history(&fixture_client(), &api).await.unwrap();
    server.await.unwrap();
    assert_eq!(history.len(), 2);

    let dir = tempfile::tempdir().unwrap();
    let stale = fixture_release(&tag, "旧版缓存");
    Cache {
        latest: Some(Release::parse(serde_json::from_value(stale).unwrap()).unwrap()),
        history: Some(history),
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let cache = Cache::load(dir.path());
    let releases: Vec<_> = cache.releases().collect();
    assert_eq!(releases.len(), 2);
    assert_eq!(releases[0].tag(), tag);
    assert!(
        releases[0]
            .notes_html()
            .contains("<strong>历史中的新说明</strong>")
    );
    assert!(releases[1].matches_current());
}

#[tokio::test]
async fn an_incomplete_history_fetch_returns_an_error() {
    let (api, server) = fixture_api(vec![
        (
            "/releases?per_page=20&page=1",
            "200 OK",
            "[]".into(),
            "Link: </releases?page=2>; rel=\"next\"\r\n",
        ),
        (
            "/releases?per_page=20&page=2",
            "429 Too Many Requests",
            "{}".into(),
            "",
        ),
    ])
    .await;
    let result = request_history(&fixture_client(), &api).await;
    server.await.unwrap();
    assert_eq!(result.err(), Some(CheckError::RateLimited));
}

#[test]
fn release_notes_keep_nested_and_numbered_list_text() {
    let html = markdown::to_html("3. 首项\n   - 子项\n4. 末项\n\n`<li>`");
    let html = display_list_paragraphs(&html);
    assert!(html.contains("3.&nbsp;首项"));
    assert!(html.contains("&nbsp;&nbsp;•&nbsp;子项"));
    assert!(html.contains("4.&nbsp;末项"));
    assert!(html.contains("<code>&lt;li&gt;</code>"));
}

#[tokio::test]
async fn check_failures_are_distinct_from_an_up_to_date_release() {
    for (status, error) in [
        ("404 Not Found", CheckError::Unavailable),
        ("429 Too Many Requests", CheckError::RateLimited),
        ("503 Service Unavailable", CheckError::Network),
        ("200 OK", CheckError::Invalid),
    ] {
        let result = fixture_request(status, "{}".into()).await;
        assert_eq!(result.err(), Some(error));
    }
}

#[test]
fn cache_round_trips_and_ignores_unverified_assets_and_active_markup() {
    let mut version = current();
    version.patch += 1;
    let tag = format!("v{version}");
    let mut release = fixture_release(
        &tag,
        "## 更新内容\n\n- 说明\n\n![图片](https://example.org/a.png)\n\n<a href=\"file:///C:/test\">文件</a>\n\n[测试](javascript:alert(1))",
    );
    release["assets"][1]["browser_download_url"] = json!("https://example.org/portable.zip");
    let release = Release::parse(serde_json::from_value(release).unwrap()).unwrap();
    assert!(release.download_url(false).is_some());
    assert!(release.download_url(true).is_none());
    let dir = tempfile::tempdir().unwrap();
    let mut cache = Cache {
        latest: Some(release),
        ..Default::default()
    };
    assert!(cache.unread_update().is_some());
    cache.save(dir.path()).unwrap();
    cache.seen_tag = Some(tag);
    cache.last_error = Some(CheckError::Network);
    cache.save(dir.path()).unwrap();
    let cache = Cache::load(dir.path());
    assert!(cache.unread_update().is_none());
    assert_eq!(cache.last_error, Some(CheckError::Network));
    let html = cache.latest.as_ref().unwrap().notes_html();
    assert!(html.contains("<h2>更新内容</h2>"));
    assert!(!html.contains("<img") && !html.contains("<a "));
    std::fs::write(dir.path().join(CACHE_FILE), b"invalid").unwrap();
    assert!(Cache::load(dir.path()).latest.is_none());
}

#[test]
fn dismissed_update_stays_hidden_until_a_higher_release() {
    let mut version = current();
    version.patch += 1;
    let tag = format!("v{version}");
    let release =
        |tag| Release::parse(serde_json::from_value(fixture_release(tag, "")).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    Cache {
        latest: Some(release(&tag)),
        dismissed_tag: Some(tag.clone()),
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let mut cache = Cache::load(dir.path());
    assert!(cache.unread_update().is_none() && cache.update_notice().is_none());
    assert!(cache.latest.as_ref().unwrap().download_url(false).is_some());
    version.patch += 1;
    cache.latest = Some(release(&format!("v{version}")));
    assert!(cache.unread_update().is_some() && cache.update_notice().is_some());
}

#[test]
fn schedules_from_the_last_attempt_and_catches_up_once() {
    use CheckFrequency::*;
    let at = |year, month, day| Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap();
    let last = at(2028, 1, 31);
    let minute = TimeDelta::minutes(1);
    for (frequency, next) in [
        (Daily, at(2028, 2, 1)),
        (Weekly, at(2028, 2, 7)),
        (Monthly, at(2028, 2, 29)),
    ] {
        assert!(frequency.is_due(last, None));
        assert!(!frequency.is_due(next - minute, Some(last)));
        assert!(frequency.is_due(next, Some(last)));
        let resumed = at(2028, 3, 1);
        assert!(frequency.is_due(resumed, Some(last)));
        assert!(!frequency.is_due(resumed + minute, Some(resumed)));
    }
    assert!(Monthly.is_due(at(2027, 2, 28), Some(at(2027, 1, 31))));
    let local = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    let last_local = local.with_ymd_and_hms(2027, 1, 31, 2, 0, 0).unwrap();
    let next_local = local.with_ymd_and_hms(2027, 2, 28, 2, 0, 0).unwrap();
    assert!(!Monthly.is_due(next_local - minute, Some(last_local)));
    assert!(Monthly.is_due(next_local, Some(last_local)));
    assert!(!Never.is_due(last, None));
    assert!(!Never.is_due(at(2028, 12, 31), Some(last)));
}

#[test]
fn update_frequency_migrates_and_round_trips_without_legacy_fields() {
    use crate::config::Config;
    use CheckFrequency::*;
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        Config::load(dir.path()).unwrap().update_check_frequency,
        Weekly
    );
    let path = dir.path().join("config.toml");
    for (source, expected) in [
        ("theme = 'dark'", Weekly),
        (
            "auto_check_updates = true\nupdate_check_minute = 720",
            Daily,
        ),
        (
            "auto_check_updates = false\nupdate_check_minute = 720",
            Never,
        ),
        (
            "auto_check_updates = false\nupdate_check_frequency = 'monthly'",
            Monthly,
        ),
    ] {
        std::fs::write(&path, source).unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.update_check_frequency, expected);
        config.save(dir.path()).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("auto_check_updates") && !saved.contains("update_check_minute"));
        assert_eq!(
            Config::load(dir.path()).unwrap().update_check_frequency,
            expected
        );
    }
}
