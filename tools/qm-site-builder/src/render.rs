//! README Markdown to sanitized HTML (spec 8.3 step 5). A README is third-party content shown on
//! the site's domain: raw HTML, scripts, iframes, styles and event attributes are removed; only
//! https (and mailto) URLs survive; relative links and images point at the repository's files.
use ammonia::{Builder, UrlRelative};
use pulldown_cmark::{html, Options, Parser};
use std::collections::HashSet;

/// Renders `markdown`. `raw_base` (for example
/// `https://raw.githubusercontent.com/<owner>/<name>/<branch>/<dir>/`) resolves relative URLs;
/// `None` drops them.
pub fn render_markdown(markdown: &str, raw_base: Option<&str>) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let mut unsafe_html = String::new();
    html::push_html(&mut unsafe_html, Parser::new_ext(markdown, opts));

    let mut b = Builder::default();
    b.url_schemes(HashSet::from(["https", "mailto"]))
        .link_rel(Some("noopener noreferrer nofollow"))
        .strip_comments(true)
        .add_tag_attributes("img", ["alt", "title", "width", "height"])
        .add_tag_attributes("code", ["class"]);
    match raw_base.and_then(|base| url::Url::parse(base).ok()) {
        Some(base) => {
            b.url_relative(UrlRelative::RewriteWithBase(base));
        }
        None => {
            b.url_relative(UrlRelative::Deny);
        }
    }
    b.clean(&unsafe_html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_scripts_and_iframes_are_removed() {
        let md = "# Title\n\n<script>alert(1)</script>\n\n<iframe src=\"https://x.example\"></iframe>\n\n<img src=\"https://a.example/i.png\" onerror=\"alert(2)\">\n\n[js](javascript:alert(3)) [plain](http://insecure.example/) <b style=\"color:red\" onclick=\"x()\">bold</b>\n";
        let out = render_markdown(md, Some("https://raw.githubusercontent.com/o/r/main/"));
        for bad in [
            "<script",
            "alert(1)",
            "<iframe",
            "onerror",
            "javascript:",
            "onclick",
            "style=",
            "http://insecure",
        ] {
            assert!(!out.contains(bad), "{bad} survived in {out}");
        }
        assert!(out.contains("<h1>Title</h1>"));
        assert!(out.contains("<img src=\"https://a.example/i.png\""));
        assert!(out.contains("<b>bold</b>"));
    }

    #[test]
    fn relative_links_rewritten() {
        let md = "![shot](docs/shot.png) [guide](GUIDE.md) [abs](https://example.com/x)";
        let out = render_markdown(
            md,
            Some("https://raw.githubusercontent.com/o/r/main/GCore/"),
        );
        assert!(
            out.contains("src=\"https://raw.githubusercontent.com/o/r/main/GCore/docs/shot.png\""),
            "{out}"
        );
        assert!(
            out.contains("href=\"https://raw.githubusercontent.com/o/r/main/GCore/GUIDE.md\""),
            "{out}"
        );
        assert!(out.contains("href=\"https://example.com/x\""));
        let no_base = render_markdown("[guide](GUIDE.md)", None);
        assert!(!no_base.contains("GUIDE.md\""), "{no_base}");
    }
}
