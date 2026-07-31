use std::path::{Path, PathBuf};

/// Whether document-authored resources may leave the local authorization
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceAccess {
    /// Default conversion policy: inline data and same-document fragments are
    /// allowed, local files require an explicit root, and network access is
    /// denied.
    Sanitized,
    /// Explicitly trusted input may retain network URLs and, without a root,
    /// legacy working-directory-relative paths.
    Trusted,
}

/// A canonical directory that explicitly authorizes local document resources.
///
/// Canonicalizing the directory once makes later descendant checks resistant
/// to both `..` traversal and symlink escapes.
#[derive(Debug, Clone)]
pub(crate) struct AuthorizedResourceRoot {
    canonical: PathBuf,
}

impl AuthorizedResourceRoot {
    pub(crate) fn parse(path: &Path) -> Option<Self> {
        let canonical = path.canonicalize().ok()?;
        canonical.is_dir().then_some(Self { canonical })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.canonical
    }

    fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.canonical)
    }

    fn resolve(&self, base: &Path, reference: &str) -> Option<PathBuf> {
        let reference = Path::new(reference);
        let candidate = if reference.is_absolute() {
            reference.to_path_buf()
        } else {
            base.join(reference)
        };
        let canonical = candidate.canonicalize().ok()?;
        canonical.starts_with(&self.canonical).then_some(canonical)
    }
}

/// Network-fetch policy for remote (`http`/`https`) document resources,
/// modelled on Gotenberg's `downloadFrom` controls. Precedence when deciding a
/// URL (see [`crate::security::network`]): a deny host match always rejects;
/// then an allow host match accepts and bypasses the IP-class checks; otherwise
/// the host is resolved and the enabled IP-class rejections apply.
///
/// Allow/deny entries are host patterns: an exact host (`cdn.example.com`), or a
/// `.`-prefixed suffix matching any subdomain (`.example.com`). Matching the
/// parsed host — not the URL string — avoids allow-list bypasses via query,
/// path, or userinfo.
#[derive(Debug, Clone)]
// The fields are read only by the `remote` fetch path; without it no network
// load happens, so the policy is inert.
#[cfg_attr(not(feature = "remote"), allow(dead_code))]
pub(crate) struct NetworkPolicy {
    /// Host patterns; a URL whose host matches any bypasses the IP-class checks.
    pub(crate) allow: Vec<String>,
    /// Host patterns; a URL whose host matches any is always rejected.
    pub(crate) deny: Vec<String>,
    /// Reject a URL whose host resolves to a non-public IP (loopback, RFC1918,
    /// link-local, IPv6 unique-local).
    pub(crate) deny_private_ips: bool,
    /// Reject a URL whose host resolves to a public IP.
    pub(crate) deny_public_ips: bool,
    /// Maximum number of redirect hops followed for one fetch.
    pub(crate) max_redirects: u32,
    /// Maximum accepted response body size, in bytes.
    pub(crate) max_body_size: u64,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            allow: Vec::new(),
            deny: Vec::new(),
            // Deny-by-default: reject URLs resolving to a private/reserved
            // address (this covers the 169.254.169.254 metadata endpoint) unless
            // the host is allow-listed. Opt out with `download_deny_private_ips`.
            deny_private_ips: true,
            deny_public_ips: false,
            max_redirects: 8,
            max_body_size: 10 * 1024 * 1024,
        }
    }
}

/// Resource resolution policy carried through one document conversion.
///
/// The optional root is the sole authority for local files in sanitized mode.
/// Trusted conversions without a root retain the historical current-directory
/// behavior; setting a root confines trusted local resources as well.
#[derive(Debug, Clone)]
pub(crate) struct DocumentResources {
    access: ResourceAccess,
    base: Option<PathBuf>,
    root: Option<AuthorizedResourceRoot>,
    network: NetworkPolicy,
}

impl DocumentResources {
    pub(crate) fn new(
        access: ResourceAccess,
        base_path: Option<&Path>,
        authorized_root: Option<&Path>,
    ) -> Self {
        let root = authorized_root
            .or(base_path)
            .and_then(AuthorizedResourceRoot::parse);
        let base = match base_path {
            Some(path) => path
                .canonicalize()
                .ok()
                .filter(|path| path.is_dir())
                .filter(|path| root.as_ref().is_none_or(|root| root.contains(path))),
            None => root
                .as_ref()
                .map(AuthorizedResourceRoot::path)
                .map(Path::to_path_buf),
        };
        Self {
            access,
            base,
            root,
            network: NetworkPolicy::default(),
        }
    }

    /// Attach the remote-fetch policy (builder style; the default denies
    /// private/reserved IPs).
    pub(crate) fn with_network(mut self, network: NetworkPolicy) -> Self {
        self.network = network;
        self
    }

    pub(crate) fn network(&self) -> &NetworkPolicy {
        &self.network
    }

    pub(crate) fn base_path(&self) -> Option<&Path> {
        self.base.as_deref()
    }

    pub(crate) fn has_authorized_root(&self) -> bool {
        self.root.is_some()
    }

    /// Resolve a resource reference at an HTML/CSS boundary.
    ///
    /// The returned local reference is canonical and therefore carries proof
    /// that it is inside the authorized root. A denied reference is represented
    /// by `None`, not by a path-shaped string that later code must revalidate.
    pub(crate) fn resolve(&self, reference: &str, base: Option<&Path>) -> Option<String> {
        let reference = reference.trim();
        match ResourceReference::parse(reference)? {
            ResourceReference::Inline | ResourceReference::Fragment => Some(reference.to_string()),
            // A protocol-relative `//host` has no scheme to fetch here, so deny
            // it rather than return it for the loader to read as a local file.
            ResourceReference::Network
                if self.access == ResourceAccess::Trusted && is_network_url(reference) =>
            {
                Some(reference.to_string())
            }
            ResourceReference::Network | ResourceReference::UnsupportedScheme => None,
            ResourceReference::Local => match &self.root {
                Some(root) => root
                    .resolve(
                        base.or_else(|| self.base_path())
                            .unwrap_or_else(|| root.path()),
                        reference,
                    )
                    .map(|path| path.to_string_lossy().into_owned()),
                None if self.access == ResourceAccess::Trusted => Some(reference.to_string()),
                None => None,
            },
        }
    }

    /// Rewrite every actual CSS `url()` token through the document resource
    /// policy. Text inside comments and strings is deliberately untouched.
    pub(crate) fn rewrite_css_urls(&self, css: &str, base: Option<&Path>) -> String {
        CssUrlRewriter::new(css, self, base).rewrite()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceReference {
    Inline,
    Fragment,
    Network,
    Local,
    UnsupportedScheme,
}

impl ResourceReference {
    fn parse(reference: &str) -> Option<Self> {
        if reference.is_empty() {
            return None;
        }
        if reference.starts_with('#') {
            return Some(Self::Fragment);
        }
        if starts_ascii_case_insensitive(reference, "data:") {
            return Some(Self::Inline);
        }
        if reference.starts_with("//") || is_network_url(reference) {
            return Some(Self::Network);
        }
        if has_explicit_scheme(reference) {
            return Some(Self::UnsupportedScheme);
        }
        Some(Self::Local)
    }
}

fn has_explicit_scheme(reference: &str) -> bool {
    let Some(colon) = reference.find(':') else {
        return false;
    };
    let scheme = &reference[..colon];
    !scheme.is_empty()
        && scheme.bytes().enumerate().all(|(index, byte)| {
            matches!(
                (index, byte),
                (0, b'a'..=b'z' | b'A'..=b'Z')
                    | (_, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'+' | b'-' | b'.')
            )
        })
}

fn starts_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// A fetchable network reference: an absolute http(s) URL, scheme compared
/// case-insensitively. Shared by the resolver and the loader so their notion of
/// "network" can't drift and let a reference slip between the local and network
/// classes.
pub(crate) fn is_network_url(reference: &str) -> bool {
    starts_ascii_case_insensitive(reference, "http://")
        || starts_ascii_case_insensitive(reference, "https://")
}

struct CssUrlRewriter<'a> {
    css: &'a str,
    resources: &'a DocumentResources,
    base: Option<&'a Path>,
    cursor: usize,
    output: String,
}

impl<'a> CssUrlRewriter<'a> {
    fn new(css: &'a str, resources: &'a DocumentResources, base: Option<&'a Path>) -> Self {
        Self {
            css,
            resources,
            base,
            cursor: 0,
            output: String::with_capacity(css.len()),
        }
    }

    fn rewrite(mut self) -> String {
        while self.cursor < self.css.len() {
            if self.copy_comment() || self.copy_string() || self.rewrite_url() {
                continue;
            }
            self.copy_char();
        }
        self.output
    }

    fn copy_comment(&mut self) -> bool {
        if !self.remaining().starts_with("/*") {
            return false;
        }
        let end = self.remaining()[2..]
            .find("*/")
            .map_or(self.css.len(), |offset| self.cursor + 2 + offset + 2);
        self.copy_through(end);
        true
    }

    fn copy_string(&mut self) -> bool {
        let Some(quote @ (b'\'' | b'"')) = self.current_byte() else {
            return false;
        };
        let end = quoted_end(self.css.as_bytes(), self.cursor + 1, quote).unwrap_or(self.css.len());
        self.copy_through(end);
        true
    }

    fn rewrite_url(&mut self) -> bool {
        let bytes = self.css.as_bytes();
        if !is_url_function_at(bytes, self.cursor) {
            return false;
        }
        let body_start = self.cursor + 4;
        let Some(close) = css_function_close(bytes, body_start) else {
            self.output.push_str("url(\"\")");
            self.cursor = self.css.len();
            return true;
        };
        let raw_body = &self.css[body_start..close];
        let reference = unquote_url_body(raw_body);
        match reference.and_then(|value| self.resources.resolve(value, self.base)) {
            Some(resolved) => {
                self.output.push_str("url(\"");
                push_css_string(&mut self.output, &resolved);
                self.output.push_str("\")");
            }
            None => self.output.push_str("url(\"\")"),
        }
        self.cursor = close + 1;
        true
    }

    fn current_byte(&self) -> Option<u8> {
        self.css.as_bytes().get(self.cursor).copied()
    }

    fn remaining(&self) -> &str {
        &self.css[self.cursor..]
    }

    fn copy_char(&mut self) {
        let Some(ch) = self.remaining().chars().next() else {
            return;
        };
        self.output.push(ch);
        self.cursor += ch.len_utf8();
    }

    fn copy_through(&mut self, end: usize) {
        if let Some(text) = self.css.get(self.cursor..end) {
            self.output.push_str(text);
            self.cursor = end;
        } else {
            self.cursor = self.css.len();
        }
    }
}

fn is_url_function_at(bytes: &[u8], index: usize) -> bool {
    let Some(candidate) = bytes.get(index..index + 4) else {
        return false;
    };
    candidate[..3].eq_ignore_ascii_case(b"url")
        && candidate[3] == b'('
        && index
            .checked_sub(1)
            .and_then(|previous| bytes.get(previous))
            .is_none_or(|byte| !is_css_ident_byte(*byte))
}

fn is_css_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | 0x80..=0xff)
}

fn css_function_close(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    while let Some(byte) = bytes.get(cursor).copied() {
        match byte {
            b'\'' | b'"' => cursor = quoted_end(bytes, cursor + 1, byte)?,
            b'\\' => cursor = (cursor + 2).min(bytes.len()),
            b')' => return Some(cursor),
            _ => cursor += 1,
        }
    }
    None
}

fn quoted_end(bytes: &[u8], mut cursor: usize, quote: u8) -> Option<usize> {
    while let Some(byte) = bytes.get(cursor).copied() {
        match byte {
            b'\\' => cursor = (cursor + 2).min(bytes.len()),
            byte if byte == quote => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }
    None
}

fn unquote_url_body(body: &str) -> Option<&str> {
    let body = body.trim();
    match body.as_bytes() {
        [quote @ (b'\'' | b'"'), .., closing] if quote == closing => body
            .get(1..body.len().saturating_sub(1))
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        [b'\'' | b'"', ..] => None,
        _ => (!body.is_empty()).then_some(body),
    }
}

fn push_css_string(output: &mut String, value: &str) {
    for ch in value.chars() {
        if matches!(ch, '"' | '\\') {
            output.push('\\');
        }
        output.push(ch);
    }
}

#[cfg(test)]
mod tests;
