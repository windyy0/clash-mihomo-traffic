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
    replies: Vec<(&str, &'static str, String, &'static str)>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://{}/releases", listener.local_addr().unwrap());
    let replies: Vec<_> = replies
        .into_iter()
        .map(|(path, status, body, headers)| (path.to_owned(), status, body, headers))
        .collect();
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
            let length = if extra_headers.contains("Transfer-Encoding: chunked") {
                String::new()
            } else {
                format!("Content-Length: {}\r\n", body.len())
            };
            let response = format!(
                "HTTP/1.1 {status}\r\n{length}Connection: close\r\n{extra_headers}\r\n{body}"
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
async fn reads_current_notes_from_the_exact_tag() {
    let tag = format!("v{CURRENT}");
    let path = format!("/releases/tags/{tag}");
    let installed = fixture_release(&tag, "当前版本说明");
    let (api, server) = fixture_api(vec![
        (
            &path,
            "200 OK",
            installed.to_string(),
            "Link: </releases?page=2>; rel=\"next\"\r\n",
        ),
        (
            &path,
            "200 OK",
            fixture_release("v0.0.0", "其他版本说明").to_string(),
            "",
        ),
    ])
    .await;
    let client = fixture_client();
    let release = Release::parse(request_tag(&client, &api, &tag).await.unwrap()).unwrap();
    assert!(release.matches_current());
    assert!(release.notes_html().contains("当前版本说明"));
    assert_eq!(
        request_tag(&client, &api, &tag).await.err(),
        Some(CheckError::Invalid)
    );
    server.await.unwrap();
}

#[test]
fn retains_only_current_notes_from_a_legacy_history_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(CACHE_FILE);
    let old =
        json!({"tag": "v0.0.0", "notes": "其他版本说明", "installer": false, "portable": false});
    let installed = json!({"tag": format!("v{CURRENT}"), "notes": "当前版本说明", "installer": true, "portable": true});
    std::fs::write(
        &path,
        json!({"current": old, "history": [old, installed]}).to_string(),
    )
    .unwrap();
    let cache = Cache::load(dir.path());
    assert!(
        cache
            .current_release()
            .unwrap()
            .notes_html()
            .contains("当前版本说明")
    );
    cache.save(dir.path()).unwrap();
    let contents = std::fs::read_to_string(path).unwrap();
    assert!(!contents.contains("history") && !contents.contains("其他版本说明"));
}

#[tokio::test]
async fn installer_download_requires_the_published_sha256() {
    let chunked = || {
        (
            "/releases/setup.exe",
            "200 OK",
            "3\r\nabc\r\n0\r\n\r\n".into(),
            "Transfer-Encoding: chunked\r\n",
        )
    };
    let (api, server) = fixture_api(vec![
        ("/releases/setup.exe", "200 OK", "abc".into(), ""),
        chunked(),
        chunked(),
        ("/releases/setup.exe", "200 OK", "abd".into(), ""),
    ])
    .await;
    let url = format!("{api}/setup.exe");
    let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let client = fixture_client();
    for (asset_size, total) in [(None, Some(3)), (Some(3), Some(3)), (None, None)] {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut reports = Vec::new();
        installer::download_verified(
            &client,
            &url,
            hash,
            asset_size,
            file.as_file_mut(),
            |value| {
                reports.push(value);
            },
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(file.path()).unwrap(), b"abc");
        assert_eq!(
            reports.first(),
            Some(&UpdateProgress::Downloading { received: 0, total })
        );
        assert_eq!(
            reports.last(),
            Some(&UpdateProgress::Downloading { received: 3, total })
        );
    }

    let mut file = tempfile::tempfile().unwrap();
    assert!(
        installer::download_verified(&client, &url, hash, None, &mut file, |_| {})
            .await
            .is_err()
    );
    server.await.unwrap();
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
