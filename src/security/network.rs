//! Remote-fetch authorization (SSRF hardening), modelled on Gotenberg's
//! `downloadFrom` controls. Only compiled with the `remote` feature, since no
//! network fetch happens without it.
//!
//! NOTE: pinning uses ureq's `unversioned` resolver API, which ureq documents
//! as not following semver. It is contained entirely in this module.

use super::resources::NetworkPolicy;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// The URL-level decision for one candidate URL, before its host is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UrlDecision {
    /// Deny-list match: reject regardless of anything else.
    Reject,
    /// Allow-list match: fetch, skipping the IP-class checks (still pinned).
    AllowBypassIp,
    /// No list match: fetch, applying the enabled IP-class checks.
    CheckIp,
}

fn url_decision(url: &str, policy: &NetworkPolicy) -> UrlDecision {
    let host = host_of(url);
    let matches = |patterns: &[String]| {
        host.as_deref()
            .is_some_and(|host| patterns.iter().any(|pattern| host_matches(host, pattern)))
    };
    if matches(&policy.deny) {
        UrlDecision::Reject
    } else if matches(&policy.allow) {
        UrlDecision::AllowBypassIp
    } else {
        UrlDecision::CheckIp
    }
}

/// Match a normalised host against a policy pattern: a `.`-prefixed pattern
/// matches any subdomain (`.example.com` matches `a.example.com`, not the apex
/// or `evilexample.com`); otherwise the host must match exactly.
fn host_matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    if pattern.starts_with('.') {
        host.ends_with(&pattern)
    } else {
        host == pattern
    }
}

/// The normalised host of an http(s) URL: lowercased, with userinfo, port,
/// IPv6 brackets, and any trailing dot removed. Matching this parsed host —
/// rather than the URL string — is what makes the allow/deny lists robust
/// against query/path/userinfo bypasses.
fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"))?;
    // Authority ends at the first '/', '?', or '#'; userinfo precedes '@'.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(after) = authority.strip_prefix('[') {
        // Bracketed IPv6 literal: take up to ']'.
        after.split(']').next().unwrap_or(after)
    } else {
        // host[:port] — the host is up to the port separator.
        authority.split(':').next().unwrap_or(authority)
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn ip_class_allowed(ip: IpAddr, deny_private: bool, deny_public: bool) -> bool {
    let private = is_private_ip(ip);
    !((deny_private && private) || (deny_public && !private))
}

/// Fetch `url` subject to `policy`, following redirects manually and
/// re-checking each hop. Every connection goes through [`PinnedResolver`], so
/// the address checked against the IP-class rules is exactly the address
/// connected to — pinning the connection and closing the DNS-rebinding window.
/// Returns the body, or `None` if rejected, if it fails, if it redirects more
/// than `policy.max_redirects` times, or if it exceeds `policy.max_body_size`.
pub(crate) fn fetch_authorized(url: &str, policy: &NetworkPolicy) -> Option<Vec<u8>> {
    let mut current = url.to_string();
    let mut redirects_left = policy.max_redirects;
    loop {
        let bypass_ip_checks = match url_decision(&current, policy) {
            UrlDecision::Reject => return None,
            UrlDecision::AllowBypassIp => true,
            UrlDecision::CheckIp => false,
        };
        let response = pinned_agent(policy, bypass_ip_checks).get(&current).call().ok()?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            if redirects_left == 0 {
                return None;
            }
            redirects_left -= 1;
            let location = response.headers().get("location")?.to_str().ok()?.to_string();
            current = resolve_redirect(&current, &location)?;
            continue;
        }
        let len = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(0);
        if len > policy.max_body_size {
            return None;
        }
        return response
            .into_body()
            .with_config()
            .limit(policy.max_body_size)
            .read_to_vec()
            .ok();
    }
}

/// An agent that does not auto-follow redirects and resolves through
/// [`PinnedResolver`].
///
/// The default (env) proxy is respected. If a proxy is configured, ureq
/// connects to the proxy and the proxy resolves the target host, so the
/// IP-class check and pinning apply to the proxy connection, not the final
/// target — configuring a safe proxy is the operator's responsibility. The URL
/// allow/deny lists are checked before the request and hold either way.
fn pinned_agent(policy: &NetworkPolicy, bypass_ip_checks: bool) -> ureq::Agent {
    let config = Config::builder().max_redirects(0).build();
    let resolver = PinnedResolver {
        deny_private_ips: policy.deny_private_ips,
        deny_public_ips: policy.deny_public_ips,
        bypass_ip_checks,
    };
    ureq::Agent::with_parts(config, DefaultConnector::default(), resolver)
}

/// Resolves a host but returns only policy-permitted addresses. Because ureq
/// connects to exactly what the resolver returns, the checked address is the
/// connected address.
#[derive(Debug)]
struct PinnedResolver {
    deny_private_ips: bool,
    deny_public_ips: bool,
    bypass_ip_checks: bool,
}

impl Resolver for PinnedResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = DefaultResolver::default().resolve(uri, config, timeout)?;
        if self.bypass_ip_checks || (!self.deny_private_ips && !self.deny_public_ips) {
            return Ok(all);
        }
        // `from_fn` starts the array empty (`len == 0`), seeding only backing
        // storage; `push` appends the real addresses. Mirrors ureq's own
        // `DefaultResolver`.
        let mut allowed =
            ResolvedSocketAddrs::from_fn(|_| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
        for addr in all.iter() {
            if ip_class_allowed(addr.ip(), self.deny_private_ips, self.deny_public_ips) {
                allowed.push(*addr);
            }
        }
        if allowed.is_empty() {
            Err(ureq::Error::HostNotFound)
        } else {
            Ok(allowed)
        }
    }
}

/// Resolve a redirect `Location` against the current absolute URL. Absolute
/// locations are re-checked on the next loop; relative ones stay on the
/// (already-checked) host.
fn resolve_redirect(current: &str, location: &str) -> Option<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_string());
    }
    let origin = origin_of(current)?;
    if location.starts_with('/') {
        return Some(format!("{origin}{location}"));
    }
    // Relative reference (RFC 3986 §5.3): replace the last path segment.
    let base = &current[origin.len()..];
    let base = base.split(['?', '#']).next().unwrap_or(base);
    let dir_end = base.rfind('/').map_or(0, |slash| slash + 1);
    let mut path = base[..dir_end].to_string();
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    path.push_str(location);
    Some(format!("{origin}{path}"))
}

/// `scheme://authority` of an absolute http(s) URL, without a trailing slash.
fn origin_of(url: &str) -> Option<String> {
    let scheme_end = url.find("://")? + 3;
    let authority_len = url[scheme_end..].find('/').unwrap_or(url.len() - scheme_end);
    Some(url[..scheme_end + authority_len].to_string())
}

/// Whether `ip` is not publicly routable (an SSRF target): loopback, private,
/// link-local, CGNAT, unspecified, multicast, benchmarking, documentation,
/// IPv6 unique-local, Teredo, and reserved ranges. Addresses embedding an IPv4
/// address (mapped/translated/6to4/NAT64) are judged by that inner address.
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => is_private_v6(v6),
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private()             // 10/8, 172.16/12, 192.168/16
        || ip.is_loopback()     // 127/8
        || ip.is_link_local()   // 169.254/16
        || ip.is_unspecified()  // 0.0.0.0
        || ip.is_broadcast()    // 255.255.255.255
        || ip.is_multicast()    // 224/4
        || ip.is_documentation() // 192.0.2/24, 198.51.100/24, 203.0.113/24
        || (a == 100 && (64..128).contains(&b)) // 100.64/10 CGNAT
        || (a == 198 && (18..20).contains(&b)) // 198.18/15 benchmarking
        || a >= 240 // 240/4 reserved
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    // An address that embeds an IPv4 address is classified by that v4 address,
    // so an internal target cannot hide inside an IPv6 wrapper.
    if let Some(v4) = embedded_ipv4(ip) {
        return is_private_v4(v4);
    }
    let seg = ip.segments();
    let first = seg[0];
    ip.is_loopback()               // ::1
        || ip.is_unspecified()     // ::
        || ip.is_multicast()       // ff00::/8 (incl. ff02::1 link-local all-nodes)
        || (first & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        || (first & 0xffc0) == 0xfe80 // fe80::/10 link-local
        || (first == 0x2001 && seg[1] == 0x0000) // 2001:0000::/32 Teredo: the embedded client
                                                 // v4 is XOR-obfuscated, so block the whole prefix
}

/// The IPv4 address embedded in an IPv6 address, if any: IPv4-mapped
/// (`::ffff:a.b.c.d`), IPv4-translated (deprecated `::ffff:0:a.b.c.d`),
/// IPv4-compatible (deprecated `::a.b.c.d`, `::/96`), 6to4 (`2002::/16`), or the
/// NAT64 well-known prefix (`64:ff9b::/96`).
fn embedded_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return Some(v4);
    }
    let seg = ip.segments();
    let v4 = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    if seg[..6] == [0, 0, 0, 0, 0xffff, 0] {
        return Some(v4(seg[6], seg[7])); // IPv4-translated: ::ffff:0:a.b.c.d
    }
    if seg[0] == 0x2002 {
        return Some(v4(seg[1], seg[2])); // 6to4: 2002:AABB:CCDD::
    }
    if seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2..6] == [0, 0, 0, 0] {
        return Some(v4(seg[6], seg[7])); // NAT64: 64:ff9b::a.b.c.d
    }
    // IPv4-compatible `::a.b.c.d`; `::` and `::1` are excluded so they stay
    // classified as unspecified/loopback rather than `0.0.0.x`.
    if seg[..6] == [0, 0, 0, 0, 0, 0] && (seg[6] != 0 || seg[7] > 1) {
        return Some(v4(seg[6], seg[7])); // ::a.b.c.d
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> NetworkPolicy {
        NetworkPolicy::default()
    }

    #[test]
    fn host_matches_exact_and_suffix() {
        assert!(host_matches("cdn.example.com", "cdn.example.com"));
        assert!(!host_matches("cdn.example.com", "example.com"));
        // Suffix (subdomain wildcard): matches subdomains, not apex or lookalikes.
        assert!(host_matches("a.example.com", ".example.com"));
        assert!(host_matches("a.b.example.com", ".example.com"));
        assert!(!host_matches("example.com", ".example.com"));
        assert!(!host_matches("evilexample.com", ".example.com"));
        // Case-insensitive.
        assert!(host_matches("cdn.example.com", "CDN.Example.COM"));
    }

    #[test]
    fn host_of_normalises_authority() {
        assert_eq!(host_of("https://Cdn.Example.COM/x").as_deref(), Some("cdn.example.com"));
        assert_eq!(host_of("https://u:p@host.example:8443/x?q#f").as_deref(), Some("host.example"));
        assert_eq!(host_of("http://[::1]:9000/x").as_deref(), Some("::1"));
        assert_eq!(host_of("http://example.com./x").as_deref(), Some("example.com"));
        assert_eq!(host_of("ftp://example.com/x"), None);
    }

    #[test]
    fn url_decision_precedence() {
        // Deny host wins over an allow host.
        let mut p = policy();
        p.deny = vec!["blocked.example.com".to_string()];
        p.allow = vec![".example.com".to_string()];
        assert_eq!(url_decision("http://blocked.example.com/x", &p), UrlDecision::Reject);
        assert_eq!(url_decision("http://ok.example.com/x", &p), UrlDecision::AllowBypassIp);

        assert_eq!(url_decision("http://any.host/x", &policy()), UrlDecision::CheckIp);

        // Subdomain wildcard: apex and lookalikes are not allow-listed.
        let mut p = policy();
        p.allow = vec![".example.com".to_string()];
        assert_eq!(url_decision("http://a.example.com/x", &p), UrlDecision::AllowBypassIp);
        assert_eq!(url_decision("http://example.com/x", &p), UrlDecision::CheckIp);
        assert_eq!(url_decision("http://evilexample.com/x", &p), UrlDecision::CheckIp);

        // The userinfo trick cannot satisfy an allow host: the real host is parsed.
        assert_eq!(
            url_decision("http://a.example.com@169.254.169.254/x", &p),
            UrlDecision::CheckIp
        );
    }

    #[test]
    fn ip_class_allowed_respects_flags() {
        let internal: IpAddr = "127.0.0.1".parse().unwrap();
        let external: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(!ip_class_allowed(internal, true, false));
        assert!(ip_class_allowed(external, true, false));
        assert!(ip_class_allowed(internal, false, true));
        assert!(!ip_class_allowed(external, false, true));
        assert!(ip_class_allowed(internal, false, false));
        assert!(ip_class_allowed(external, false, false));
    }

    // Non-public cases are the not-globally-reachable ranges of the IANA IPv4
    // Special-Purpose Address Registry; public cases sample allocated unicast
    // space, including neighbours just outside the reserved ranges.
    #[test]
    fn ipv4_classes() {
        for ip in [
            "127.0.0.1",       // 127/8 loopback
            "10.0.0.1",        // 10/8 private
            "172.16.5.4",      // 172.16/12 private
            "192.168.1.1",     // 192.168/16 private
            "169.254.1.1",     // 169.254/16 link-local
            "100.64.0.1",      // 100.64/10 CGNAT
            "0.0.0.0",         // 0/8 unspecified
            "224.0.0.1",       // 224/4 multicast (all-systems)
            "239.255.255.250", // 224/4 multicast (SSDP)
            "198.18.0.1",      // 198.18/15 benchmarking
            "198.19.255.255",  // 198.18/15 benchmarking (upper bound)
            "192.0.2.1",       // 192.0.2/24 documentation (TEST-NET-1)
            "198.51.100.1",    // 198.51.100/24 documentation (TEST-NET-2)
            "203.0.113.1",     // 203.0.113/24 documentation (TEST-NET-3)
            "240.0.0.1",       // 240/4 reserved
            "255.255.255.255", // limited broadcast
        ] {
            assert!(is_private_ip(ip.parse().unwrap()), "{ip} should be private");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "198.17.255.255", "198.20.0.0", "223.255.255.255"] {
            assert!(!is_private_ip(ip.parse().unwrap()), "{ip} should be public");
        }
    }

    #[test]
    fn ipv6_classes() {
        for ip in [
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",            // link-local all-nodes multicast
            "ff05::1:3",          // site-local multicast
            "::ffff:127.0.0.1",   // IPv4-mapped loopback
            "::ffff:0:7f00:1",    // IPv4-translated wrapping 127.0.0.1
            "::ffff:0:a00:1",     // IPv4-translated wrapping 10.0.0.1
            "2002:7f00:1::",      // 6to4 wrapping 127.0.0.1
            "2002:a00:1::",       // 6to4 wrapping 10.0.0.1
            "64:ff9b::7f00:1",    // NAT64 wrapping 127.0.0.1
            "::7f00:1",           // IPv4-compatible wrapping 127.0.0.1
            "::a00:1",            // IPv4-compatible wrapping 10.0.0.1
            "2001:0:4136:e378::", // 2001:0000::/32 Teredo
        ] {
            assert!(is_private_ip(ip.parse().unwrap()), "{ip} should be private");
        }
        for ip in [
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "::ffff:0:808:808",     // IPv4-translated wrapping public 8.8.8.8
            "2002:808:808::",
            "::808:808",
            "2001:4860:4860::8888", // public unicast just outside the Teredo prefix
        ] {
            assert!(!is_private_ip(ip.parse().unwrap()), "{ip} should be public");
        }
    }

    #[test]
    fn redirect_resolution() {
        assert_eq!(
            resolve_redirect("http://a.com/x", "https://b.com/y").as_deref(),
            Some("https://b.com/y")
        );
        assert_eq!(
            resolve_redirect("http://a.com/x/y", "/z").as_deref(),
            Some("http://a.com/z")
        );
        assert_eq!(
            resolve_redirect("http://a.com:8080/x", "z").as_deref(),
            Some("http://a.com:8080/z")
        );
        assert_eq!(
            resolve_redirect("http://a.com/x/y", "z").as_deref(),
            Some("http://a.com/x/z")
        );
        assert_eq!(
            resolve_redirect("http://a.com/x/y?q=1", "z").as_deref(),
            Some("http://a.com/x/z")
        );
        assert_eq!(
            resolve_redirect("http://a.com", "z").as_deref(),
            Some("http://a.com/z")
        );
    }
}

/// End-to-end tests over a real loopback HTTP server, exercising the fetch
/// wiring (pinned resolver, redirect loop, size limit) that the unit tests
/// above cannot reach. Loopback resolves to `127.0.0.1`, a private address, so
/// `deny_private_ips` is expected to block unless the host is allow-listed.
#[cfg(test)]
mod server_tests {
    use super::*;
    use crate::security::resources::NetworkPolicy;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve a few fixed paths on `127.0.0.1` and return the bound port. The
    /// accept loop runs on a detached thread for the lifetime of the process.
    fn spawn_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, port);
            }
        });
        port
    }

    fn serve(mut stream: std::net::TcpStream, port: u16) {
        let mut buf = [0u8; 1024];
        let read = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..read]);
        let path = request.split_whitespace().nth(1).unwrap_or("/");
        let response = match path {
            "/ok" => "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHELLO".to_string(),
            "/redirect" => format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/ok\r\nContent-Length: 0\r\n\r\n"
            ),
            "/big" => format!("HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{}", "x".repeat(100)),
            _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string(),
        };
        let _ = stream.write_all(response.as_bytes());
    }

    /// Default policy with loopback allow-listed, to exercise the fetch wiring
    /// against the loopback server despite deny-private-by-default.
    fn allow_loopback() -> NetworkPolicy {
        NetworkPolicy {
            allow: vec!["127.0.0.1".to_string()],
            ..NetworkPolicy::default()
        }
    }

    #[test]
    fn default_policy_blocks_private_ip() {
        // Deny-by-default rejects 127.0.0.1 with no explicit flag set.
        let url = format!("http://127.0.0.1:{}/ok", spawn_server());
        assert!(fetch_authorized(&url, &NetworkPolicy::default()).is_none());
    }

    #[test]
    fn allow_listed_host_fetches_body() {
        let url = format!("http://127.0.0.1:{}/ok", spawn_server());
        assert_eq!(
            fetch_authorized(&url, &allow_loopback()).as_deref(),
            Some(b"HELLO".as_slice())
        );
    }

    #[test]
    fn allow_private_ips_fetches_body() {
        // Opting out of deny-private fetches successfully.
        let url = format!("http://127.0.0.1:{}/ok", spawn_server());
        let policy = NetworkPolicy {
            deny_private_ips: false,
            ..NetworkPolicy::default()
        };
        assert_eq!(fetch_authorized(&url, &policy).as_deref(), Some(b"HELLO".as_slice()));
    }

    #[test]
    fn allow_host_bypasses_deny_private_ips() {
        let url = format!("http://127.0.0.1:{}/ok", spawn_server());
        let policy = NetworkPolicy {
            deny_private_ips: true,
            allow: vec!["127.0.0.1".to_string()],
            ..NetworkPolicy::default()
        };
        assert_eq!(fetch_authorized(&url, &policy).as_deref(), Some(b"HELLO".as_slice()));
    }

    #[test]
    fn redirect_is_followed() {
        let url = format!("http://127.0.0.1:{}/redirect", spawn_server());
        assert_eq!(
            fetch_authorized(&url, &allow_loopback()).as_deref(),
            Some(b"HELLO".as_slice())
        );
    }

    #[test]
    fn redirect_not_followed_when_max_redirects_zero() {
        let url = format!("http://127.0.0.1:{}/redirect", spawn_server());
        let policy = NetworkPolicy {
            max_redirects: 0,
            ..allow_loopback()
        };
        assert!(fetch_authorized(&url, &policy).is_none());
    }

    #[test]
    fn body_over_max_size_is_rejected() {
        let url = format!("http://127.0.0.1:{}/big", spawn_server());
        let policy = NetworkPolicy {
            max_body_size: 10,
            ..allow_loopback()
        };
        assert!(fetch_authorized(&url, &policy).is_none());
    }
}
