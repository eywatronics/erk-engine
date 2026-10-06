//! The demo host's resource provider (p1-contract §6).
//!
//! Erk reads no file; the host does, and decides what content may reach.
//! This one serves a relative URL from the opened page's directory and
//! nothing outside it: an absolute path, a `file:` or any other scheme, and
//! a `..` that climbs out of the directory are refused, so a page cannot
//! read `file:///etc/passwd`. `memory://` is the scheme for assets an
//! application embeds; the demo embeds none yet, so it finds nothing.
//! Fonts are the app's own, from the system's (p1-contract §6.2); they do
//! not come here.

use std::path::{Path, PathBuf};

use erk::{ResourceKind, ResourceRequest, Responder};

pub(crate) struct Provider {
    /// The opened page's directory, resolved; `None` if it cannot be.
    root: Option<PathBuf>,
}

impl Provider {
    pub(crate) fn for_page(page: &Path) -> Self {
        let directory = page.parent().unwrap_or(Path::new("."));
        let directory = if directory.as_os_str().is_empty() {
            Path::new(".")
        } else {
            directory
        };
        Self {
            root: directory.canonicalize().ok(),
        }
    }

    /// Answer `request`: the file, or missing.
    pub(crate) fn answer(&self, request: &ResourceRequest, responder: Responder) {
        match self.find(request) {
            Some((mime, data)) => responder.respond(mime, data),
            None => responder.missing(),
        }
    }

    /// The MIME type and bytes the page's directory has for `request`.
    fn find(&self, request: &ResourceRequest) -> Option<(&'static str, Vec<u8>)> {
        if request.kind != ResourceKind::Image {
            return None;
        }
        let url = request.url.trim();
        if url.starts_with("memory://") {
            return None;
        }
        // A scheme (`file:`, `http:`) or a drive letter, and absolute paths.
        if url.contains(':') || url.starts_with(['/', '\\']) {
            return None;
        }
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let root = self.root.as_ref()?;
        let resolved = root.join(path).canonicalize().ok()?;
        if !resolved.starts_with(root) || !resolved.is_file() {
            return None;
        }
        let data = std::fs::read(&resolved).ok()?;
        let mime = match resolved
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            _ => "",
        };
        Some((mime, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(url: &str) -> ResourceRequest {
        ResourceRequest {
            id: 7,
            url: url.to_owned(),
            kind: ResourceKind::Image,
        }
    }

    /// A page in a fresh directory with an image beside it and a secret one
    /// level up.
    fn site() -> (PathBuf, Provider) {
        let base = std::env::temp_dir().join(format!("erk-provider-{}", std::process::id()));
        let site = base.join("site");
        std::fs::create_dir_all(site.join("img")).unwrap();
        std::fs::write(site.join("img/logo.png"), b"\x89PNG\r\n\x1a\nrest").unwrap();
        std::fs::write(base.join("secret.png"), b"secret").unwrap();
        let page = site.join("page.html");
        std::fs::write(&page, "<p>x</p>").unwrap();
        let provider = Provider::for_page(&page);
        (base, provider)
    }

    #[test]
    fn a_relative_url_is_served_from_the_page_directory() {
        let (_base, provider) = site();
        let (mime, data) = provider
            .find(&request("img/logo.png"))
            .expect("the image is served");
        assert_eq!(mime, "image/png");
        assert!(data.starts_with(b"\x89PNG"));
    }

    #[test]
    fn nothing_outside_the_page_directory_is_served() {
        let (base, provider) = site();
        let outside = base.join("secret.png");
        // Inside the directory, but absolute: content names resources by
        // relative URL, never by a path on this machine.
        let inside = base.join("site/img/logo.png");
        for url in [
            "../secret.png",
            "img/../../secret.png",
            "file:///etc/passwd",
            "/etc/passwd",
            "http://example.com/a.png",
            "memory://logo.png",
            &outside.display().to_string(),
            &inside.display().to_string(),
        ] {
            assert!(
                provider.find(&request(url)).is_none(),
                "{url} must not be served"
            );
        }
    }

    #[test]
    fn only_images_come_from_the_page_directory() {
        let (_base, provider) = site();
        for kind in [ResourceKind::Font, ResourceKind::Stylesheet] {
            let asked = ResourceRequest {
                kind,
                ..request("img/logo.png")
            };
            assert!(provider.find(&asked).is_none(), "{kind:?}");
        }
    }
}
