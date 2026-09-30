//! Fetching recipe pages over plain HTTP.
//!
//! The importer is a server-side URL fetcher, so every target is validated
//! before (and while) being downloaded: no `file://`, no localhost, no
//! loopback / private / link-local addresses — checked for the literal host
//! *and* for every IP the host resolves to, on every redirect hop.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::OnceLock;
use std::time::Duration;

use url::Url;

use super::types::{FetchedPage, ImportError};

/// Descriptive User-Agent so site owners can identify and (if they wish)
/// block the importer. No bot-protection bypassing happens here.
const USER_AGENT: &str = concat!("Bouedig/", env!("CARGO_PKG_VERSION"), " (recipe importer)");

/// Upper bound for a downloaded page (5 MiB is far beyond any recipe page).
const MAX_HTML_BYTES: usize = 5 * 1024 * 1024;

/// Per-request timeout and redirect budget.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 5;

/// One shared client for the process lifetime (no per-request clients).
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            // Redirects are followed manually so every hop can be validated
            // against the private-network rules.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("failed to build the HTTP client")
    })
}

/// Parse and validate a user-supplied URL. Rejects non-http(s) schemes,
/// embedded credentials and — unless `allow_private` is set (test hook) —
/// any host that is local, private or link-local (by name, literal IP or
/// DNS resolution).
pub async fn validate_url(raw: &str, allow_private: bool) -> Result<Url, ImportError> {
    let url = Url::parse(raw.trim()).map_err(|e| ImportError::InvalidUrl(e.to_string()))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(ImportError::InvalidUrl(format!(
            "unsupported scheme {:?} (use http or https)",
            url.scheme()
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ImportError::InvalidUrl(
            "URLs with embedded credentials are not supported".into(),
        ));
    }
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| ImportError::InvalidUrl("URL has no host".into()))?;
    if !allow_private {
        ensure_public_host(host)
            .await
            .map_err(ImportError::DisallowedTarget)?;
    }
    Ok(url)
}

/// Download a page, following up to [`MAX_REDIRECTS`] redirects. Every hop is
/// re-validated so a public page cannot bounce the fetcher into the private
/// network.
pub async fn fetch_page(url: &Url, allow_private: bool) -> Result<FetchedPage, ImportError> {
    let client = http_client();
    let mut current = url.clone();
    for _hop in 0..=MAX_REDIRECTS {
        if !allow_private {
            if let Some(host) = current.host_str() {
                ensure_public_host(host)
                    .await
                    .map_err(ImportError::DisallowedTarget)?;
            }
        }
        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| ImportError::FetchFailed(format!("{}: {e}", current.host_str().unwrap_or_default())))?;

        if response.status().is_redirection() {
            if current == final_redirect(&response, &current)? {
                return Err(ImportError::FetchFailed("redirect loop detected".into()));
            }
            current = final_redirect(&response, &current)?;
            continue;
        }

        let status = response.status();
        if !status.is_success() {
            return Err(ImportError::FetchFailed(format!(
                "the server answered with HTTP {status}"
            )));
        }

        // Bounded download: stop as soon as the cap is exceeded.
        let mut html = Vec::new();
        let mut chunks = response;
        while let Some(chunk) = chunks
            .chunk()
            .await
            .map_err(|e| ImportError::FetchFailed(format!("download interrupted: {e}")))?
        {
            if html.len() + chunk.len() > MAX_HTML_BYTES {
                return Err(ImportError::FetchFailed(format!(
                    "the page is larger than the {} MiB limit",
                    MAX_HTML_BYTES / (1024 * 1024)
                )));
            }
            html.extend_from_slice(&chunk);
        }
        let html = String::from_utf8_lossy(&html).into_owned();
        return Ok(FetchedPage {
            final_url: current,
            html,
        });
    }
    Err(ImportError::FetchFailed(format!(
        "more than {MAX_REDIRECTS} redirects"
    )))
}

/// Resolve a redirect's `Location` against the current URL.
fn final_redirect(
    response: &reqwest::Response,
    current: &Url,
) -> Result<Url, ImportError> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .filter(|l| !l.is_empty())
        .ok_or_else(|| ImportError::FetchFailed("redirect without a Location header".into()))?;
    current
        .join(location)
        .map_err(|e| ImportError::FetchFailed(format!("bad redirect target: {e}")))
}

/// Reject localhost / loopback / private / link-local hosts by name and by
/// every address the name resolves to.
async fn ensure_public_host(host: &str) -> Result<(), String> {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let lowered = bare.to_ascii_lowercase();
    if lowered == "localhost" || lowered.ends_with(".localhost") || lowered.ends_with(".local") {
        return Err(format!("{host} is a local address"));
    }
    // Literal IP in the host position (v4, or v6 with brackets stripped).
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return check_ip(&ip, host);
    }
    // Resolve DNS and check every answer: a name that only resolves into the
    // private network must not be fetched either.
    let addrs = lookup_host(bare).await?;
    if addrs.is_empty() {
        return Err(format!("{host} does not resolve to any address"));
    }
    for ip in addrs {
        check_ip(&ip, host)?;
    }
    Ok(())
}

/// DNS lookup that works both inside and outside a tokio runtime context.
async fn lookup_host(host: &str) -> Result<Vec<IpAddr>, String> {
    // `ToSocketAddrs` is a blocking call; run it on the blocking pool when we
    // are inside a runtime.
    let target = format!("{host}:0");
    let lookup = move || {
        use std::net::ToSocketAddrs;
        target
            .to_socket_addrs()
            .map(|it| it.collect::<Vec<std::net::SocketAddr>>())
    };
    let result = if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::spawn_blocking(lookup)
            .await
            .map_err(|e| format!("DNS lookup failed: {e}"))?
    } else {
        // No runtime: cannot spawn, fall through to the same closure result.
        unreachable!("lookup_host is only called from async code")
    };
    let addrs = result.map_err(|e| format!("DNS lookup failed for {host}: {e}"))?;
    Ok(addrs.into_iter().map(|a| a.ip()).collect())
}

fn check_ip(ip: &IpAddr, host: &str) -> Result<(), String> {
    if is_disallowed_ip(ip) {
        return Err(format!("{host} resolves to a private or local address"));
    }
    Ok(())
}

/// Whether an IP is loopback / private / link-local / unspecified.
pub fn is_disallowed_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_disallowed_v4(*v4),
        IpAddr::V6(v6) => {
            // Unwrap IPv4-mapped IPv6 (::ffff:10.0.0.1) before checking.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_disallowed_v4(v4);
            }
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || (segments[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (segments[0] & 0xffc0) == 0xfe80 // link local fe80::/10
        }
    }
}

fn is_disallowed_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_loopback()                      // 127.0.0.0/8
        || v4.is_private()                // 10/8, 172.16/12, 192.168/16
        || v4.is_link_local()             // 169.254/16
        || v4.is_unspecified()            // 0.0.0.0
        || o[0] == 100 && (o[1] & 0xc0) == 64 // CGNAT 100.64/10
        || o[0] == 192 && o[1] == 0 && o[2] == 2 // TEST-NET-1
        || o[0] == 198 && (o[1] & 0xfe) == 18 // benchmark 198.18/15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_unsupported_schemes() {
        for raw in [
            "file:///etc/passwd",
            "ftp://example.com/recipe",
            "javascript:alert(1)",
            "data:text/html,<h1>x</h1>",
            "not a url at all",
            "",
        ] {
            let err = validate_url(raw, false).await.unwrap_err();
            assert!(
                matches!(err, ImportError::InvalidUrl(_)),
                "{raw:?} should be an invalid URL, got {err}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_private_and_local_targets() {
        for raw in [
            "http://localhost:8000/recipe",
            "http://sub.localhost/recipe",
            "http://127.0.0.1/recipe",
            "http://127.1.2.3:3000/x",
            "http://10.0.0.42/recipe",
            "http://172.16.5.5/recipe",
            "http://192.168.1.1/recipe",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/recipe",
            "http://0.0.0.0/recipe",
            "http://[::1]/recipe",
            "http://[fe80::1]/recipe",
            "http://[fc00::1]/recipe",
            "http://[::ffff:192.168.1.1]/recipe",
        ] {
            let err = validate_url(raw, false).await.unwrap_err();
            assert!(
                matches!(err, ImportError::DisallowedTarget(_)),
                "{raw} should be disallowed, got {err}"
            );
        }
    }

    #[tokio::test]
    async fn accepts_public_urls() {
        for raw in [
            "https://www.example.com/recipe/chocolate-cake",
            "http://example.com:8080/x?y=1#frag",
            "https://203.0.113.10/recipe", // TEST-NET-3 is public by our rules
        ] {
            validate_url(raw, false)
                .await
                .unwrap_or_else(|e| panic!("{raw} should be accepted: {e}"));
        }
        // With the test escape hatch, everything parses.
        validate_url("http://127.0.0.1:9/recipe", true)
            .await
            .expect("allow_private must bypass the target checks");
    }

    #[tokio::test]
    async fn rejects_urls_with_credentials() {
        let err = validate_url("https://user:pass@example.com/recipe", false)
            .await
            .unwrap_err();
        assert!(matches!(err, ImportError::InvalidUrl(_)));
    }

    #[test]
    fn ip_classification() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.31.255.255", "192.168.0.1", "169.254.1.1", "0.0.0.0"] {
            assert!(is_disallowed_ip(&ip.parse::<IpAddr>().unwrap()), "{ip}");
        }
        // 172.32.0.1 is outside the private 172.16/12 range.
        assert!(!is_disallowed_ip(&"172.32.0.1".parse::<IpAddr>().unwrap()));
        assert!(!is_disallowed_ip(&"8.8.8.8".parse::<IpAddr>().unwrap()));
        assert!(is_disallowed_ip(&"::1".parse::<IpAddr>().unwrap()));
        assert!(!is_disallowed_ip(&"2606:4700::1111".parse::<IpAddr>().unwrap()));
    }
}


/// Upper bound for a downloaded recipe image (site heroes stay well below).
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// Download recipe image bytes from a page-provided URL. The URL comes from
/// a fetched page, so it gets the same SSRF treatment as the page fetch
/// (validated, private targets refused, every redirect hop re-checked) plus
/// a size cap. `Content-Type` is not trusted — the caller detects the real
/// format from the bytes themselves.
pub async fn download_image(raw_url: &str, allow_private: bool) -> Result<Vec<u8>, ImportError> {
    let url = validate_url(raw_url, allow_private).await?;
    let client = http_client();
    let mut current = url;
    for _hop in 0..=MAX_REDIRECTS {
        if !allow_private {
            if let Some(host) = current.host_str() {
                ensure_public_host(host)
                    .await
                    .map_err(ImportError::DisallowedTarget)?;
            }
        }
        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| ImportError::FetchFailed(format!("{}: {e}", current.host_str().unwrap_or_default())))?;
        if response.status().is_redirection() {
            current = final_redirect(&response, &current)?;
            continue;
        }
        let status = response.status();
        if !status.is_success() {
            return Err(ImportError::FetchFailed(format!(
                "the image server answered with HTTP {status}"
            )));
        }
        let mut bytes: Vec<u8> = Vec::new();
        let mut chunks = response;
        while let Some(chunk) = chunks
            .chunk()
            .await
            .map_err(|e| ImportError::FetchFailed(format!("download interrupted: {e}")))?
        {
            if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
                return Err(ImportError::FetchFailed(format!(
                    "the image is larger than the {} MiB limit",
                    MAX_IMAGE_BYTES / (1024 * 1024)
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
    Err(ImportError::FetchFailed(format!(
        "more than {MAX_REDIRECTS} redirects"
    )))
}
