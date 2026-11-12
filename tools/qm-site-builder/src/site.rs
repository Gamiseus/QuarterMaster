//! The static site: plain HTML pages from small built-in templates, the machine index and an
//! Atom feed. Output is deterministic: entries are sorted and the only time in it is the
//! index's `generated_at`, so a deploy changes only when the mods or the wiki do.
use crate::wiki::{SearchRecord, Section, Wiki, ALL_SLUG};
use qm_schema::{IndexEntry, IndexFile, Listing};
use std::path::Path;

pub const SITE_NAME: &str = "QuarterMaster";

/// The wiki heading that `/publish/` forwards to: the steps for listing a mod.
pub const PUBLISH_ANCHOR: &str = "listing-a-mod-in-the-launchers-browse-tab";

/// The stylesheet, script, font and logo, written to `static/`. A file of the same name in the
/// site repository's own `static/` folder replaces the built-in one.
const ASSETS: &[(&str, &[u8])] = &[
    ("site.css", include_bytes!("../assets/site.css")),
    ("site.js", include_bytes!("../assets/site.js")),
    ("logo.png", include_bytes!("../assets/logo.png")),
    ("stencil.woff2", include_bytes!("../assets/stencil.woff2")),
    (
        "stencil-OFL.txt",
        include_bytes!("../assets/stencil-OFL.txt"),
    ),
];

/// One mod page's extra content.
pub struct ModPage {
    pub entry: IndexEntry,
    /// Sanitized README HTML.
    pub readme_html: String,
    /// `(mod id, version range)`, sorted.
    pub requires: Vec<(String, String)>,
    pub conflicts: Vec<(String, String)>,
    /// Sanitized HTML of the latest release's notes.
    pub notes_html: String,
    /// The picture for the mod's card, copied to `mods/<id>/card.<ext>`.
    pub image: Option<CardImage>,
    /// What the mod's code is able to do, in the wiki's words; empty for plain sandboxed Lua.
    pub capabilities: Vec<String>,
    /// The listed archive was scanned by this build (not carried over from an earlier one).
    pub scanned: bool,
    /// A line shown at the top of the page, such as a newer release being checked.
    pub notice: Option<String>,
}

/// A mod's card picture: its bytes, and the file type they were found to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardImage {
    /// `png`, `jpg`, `gif` or `webp`.
    pub ext: &'static str,
    pub bytes: Vec<u8>,
}

/// The file type of a picture, from its first bytes.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// `<dt>` and `<dd>` for a requires or conflicts list; nothing when it is empty.
fn relation_fact(label: &str, list: &[(String, String)]) -> String {
    if list.is_empty() {
        return String::new();
    }
    let items: Vec<String> = list
        .iter()
        .map(|(id, range)| {
            let range = if range.trim().is_empty() || range.trim() == "*" {
                String::new()
            } else {
                format!(" {}", esc(range))
            };
            format!("<a href=\"../{id}/\">{id}</a>{range}", id = esc(id))
        })
        .collect();
    format!("<dt>{label}</dt><dd>{}</dd>", items.join(", "))
}

pub struct SiteInput<'a> {
    pub index: &'a IndexFile,
    pub pages: &'a [ModPage],
    /// The modding wiki; without it the site has no `wiki/` and no `publish/`.
    pub wiki: Option<&'a Wiki>,
    /// The changelog as HTML; without it the site has no `changelog/`.
    pub changelog_html: Option<&'a str>,
    /// The QuarterMaster version the wiki describes.
    pub version: Option<&'a str>,
    /// `https://gamiseus.github.io`, without a trailing slash.
    pub site_url: &'a str,
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// The shared page frame. `root` is the path back to the site root (`""`, `"../"`, ...), and
/// `wide` gives the page the wiki's two-column width.
fn page(
    input: &SiteInput,
    title: &str,
    root: &str,
    current: &str,
    wide: bool,
    body: &str,
) -> String {
    let mut items = vec![("", "Home"), ("mods/", "Mods")];
    if input.wiki.is_some() {
        items.push(("wiki/", "Modding wiki"));
    }
    if input.changelog_html.is_some() {
        items.push(("changelog/", "Changelog"));
    }
    let nav = items
        .iter()
        .map(|(href, label)| {
            let cur = if *href == current {
                " aria-current=\"page\""
            } else {
                ""
            };
            format!("<a href=\"{root}{href}\"{cur}>{label}</a>")
        })
        .collect::<Vec<_>>()
        .join("");
    let full_title = if title == SITE_NAME {
        SITE_NAME.to_string()
    } else {
        format!("{} - {SITE_NAME}", esc(title))
    };
    let main_class = if wide { " class=\"wide\"" } else { "" };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{full_title}</title>\n<link rel=\"stylesheet\" href=\"{root}static/site.css\">\n<link rel=\"icon\" href=\"{root}static/logo.png\">\n<link rel=\"alternate\" type=\"application/atom+xml\" title=\"Mod updates\" href=\"{root}updates.xml\">\n<script defer src=\"{root}static/site.js\"></script>\n</head>\n<body>\n<header class=\"top\"><a class=\"brand\" href=\"{home}\"><img src=\"{root}static/logo.png\" alt=\"\" width=\"28\" height=\"28\">{SITE_NAME}</a><nav aria-label=\"Site\">{nav}</nav><form class=\"search\" role=\"search\" action=\"{root}search/\"><input type=\"search\" name=\"q\" placeholder=\"Search\" aria-label=\"Search the wiki and the mods\"></form></header>\n<main{main_class}>\n{body}\n</main>\n<footer class=\"foot\">QuarterMaster is a mod manager for Mud and Blood. This site has no tracking and no third-party scripts.</footer>\n</body>\n</html>\n",
        home = if root.is_empty() { "./" } else { root },
    )
}

fn sorted(index: &IndexFile) -> Vec<&IndexEntry> {
    let mut v: Vec<&IndexEntry> = index.mods.iter().collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

fn by_release(index: &IndexFile) -> Vec<&IndexEntry> {
    let mut v: Vec<&IndexEntry> = index.mods.iter().collect();
    v.sort_by(|a, b| b.released_at.cmp(&a.released_at).then(a.id.cmp(&b.id)));
    v
}

fn listing_badge(e: &IndexEntry) -> &'static str {
    match e.listing {
        Listing::Registered => "<span class=\"badge\">Registered</span>",
        Listing::Discovered => "<span class=\"badge muted\">Discovered</span>",
    }
}

fn date(ts: &str) -> &str {
    ts.get(..10).unwrap_or(ts)
}

fn authors_of(e: &IndexEntry) -> String {
    if e.authors.is_empty() {
        e.repository
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string()
    } else {
        e.authors.join(", ")
    }
}

fn home(input: &SiteInput) -> String {
    let latest: String = by_release(input.index)
        .iter()
        .take(5)
        .map(|e| {
            format!(
                "<li><a href=\"mods/{id}/\">{name}</a> {v} <span class=\"muted\">{d}</span></li>",
                id = esc(&e.id),
                name = esc(&e.name),
                v = esc(&e.latest_version),
                d = esc(date(&e.released_at))
            )
        })
        .collect();
    let latest = if latest.is_empty() {
        "<p class=\"empty\">Nothing here yet.</p>".to_string()
    } else {
        format!("<ul class=\"plain\">{latest}</ul>")
    };
    let making = if input.wiki.is_some() {
        "<h2 class=\"stencil\">Make a mod</h2>\n<p>The <a href=\"wiki/\">modding wiki</a> covers everything a mod can do, from replacing a sound to patching game code, and how to <a href=\"publish/\">list your mod here</a>.</p>\n"
    } else {
        ""
    };
    let body = format!(
        "<h1 class=\"stencil\">QuarterMaster</h1>\n<p class=\"lead\">A mod manager for Mud and Blood: install mods, keep load orders and saves apart per profile, and update with one click.</p>\n<p><a class=\"button\" href=\"https://github.com/Gamiseus/QuarterMaster/releases\">Download QuarterMaster</a></p>\n<h2 class=\"stencil\">Getting started</h2>\n<ol><li>Download and run QuarterMaster.</li><li>Let it find your Mud and Blood folder.</li><li>Open Mods, then Browse, and install what you like. Updates arrive on their own.</li></ol>\n{making}<h2 class=\"stencil\">Latest releases</h2>\n{latest}\n{more}",
        more = match input.index.mods.len() {
            0 => String::new(),
            count => format!("<p><a href=\"mods/\">All mods ({count})</a> &middot; <a href=\"updates.xml\">Atom feed</a></p>"),
        }
    );
    page(input, SITE_NAME, "", "", false, &body)
}

/// The card for one mod: its picture (or its initial on a plain panel), name, summary and facts.
/// `link` is the mod's page as the listing page should write it.
fn mod_card(input: &SiteInput, e: &IndexEntry, link: &str) -> String {
    let authors = authors_of(e);
    let tags: String = e
        .topics
        .iter()
        .map(|t| format!("<span class=\"tag\">{}</span>", esc(t)))
        .collect();
    let picture = match input
        .pages
        .iter()
        .find(|p| p.entry.id == e.id)
        .and_then(|p| p.image.as_ref())
    {
        Some(image) => format!(
            "<img class=\"shot\" src=\"{link}card.{}\" alt=\"\" loading=\"lazy\">",
            image.ext
        ),
        None => format!(
            "<div class=\"shot blank\" aria-hidden=\"true\">{}</div>",
            esc(&e
                .name
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string())
        ),
    };
    format!(
        "<li class=\"card\" data-search=\"{search}\">{picture}<div class=\"body\"><h2><a href=\"{link}\">{name}</a></h2>{badge}<p>{summary}</p><p class=\"meta\">{authors} &middot; {v} &middot; {d}</p><p class=\"tags\">{tags}</p></div></li>",
        search = esc(&format!("{} {} {} {}", e.name, e.summary, authors, e.topics.join(" ")).to_lowercase()),
        name = esc(&e.name),
        badge = listing_badge(e),
        summary = esc(&e.summary),
        authors = esc(&authors),
        v = esc(&e.latest_version),
        d = esc(date(&e.released_at)),
    )
}

fn mod_list(input: &SiteInput) -> String {
    let cards: String = sorted(input.index)
        .iter()
        .map(|e| mod_card(input, e, &format!("{}/", esc(&e.id))))
        .collect();
    let body = if cards.is_empty() {
        let how = if input.wiki.is_some() {
            " Mods appear here once they are published: <a href=\"../publish/\">how to list a mod</a>."
        } else {
            ""
        };
        format!("<h1 class=\"stencil\">Mods</h1>\n<p class=\"empty\">Nothing here yet.{how}</p>")
    } else {
        format!(
            "<h1 class=\"stencil\">Mods</h1>\n<p><label for=\"q\">Filter</label> <input id=\"q\" type=\"search\" placeholder=\"Name, author or tag\" autocomplete=\"off\"></p>\n<ul class=\"cards\" id=\"mods\">{cards}</ul>\n<p id=\"none\" class=\"empty\" hidden>Nothing matches.</p>"
        )
    };
    page(input, "Mods", "../", "mods/", true, &body)
}
fn mod_page(input: &SiteInput, p: &ModPage) -> String {
    let e = &p.entry;
    let authors = if e.authors.is_empty() {
        "-".to_string()
    } else {
        e.authors.join(", ")
    };
    let license = e.license.clone().unwrap_or_else(|| "Not stated".into());
    let size_kb = e.size_bytes.div_ceil(1024);
    let body = format!(
        "<p><a href=\"../\">All mods</a></p>\n<div class=\"title\"><h1 class=\"stencil\">{name}</h1>{badge}</div>\n<p class=\"lead\">{summary}</p>\n{notice}{picture}<p><a class=\"button\" href=\"quartermaster://install/{id}\">Install with QuarterMaster</a></p>\n<p class=\"muted small\">If the button does nothing, open QuarterMaster, then Mods, Browse, and search for {name}.</p>\n<dl class=\"facts\"><dt>Version</dt><dd>{v} (release {tag}, {d})</dd><dt>Authors</dt><dd>{authors}</dd><dt>License</dt><dd>{license}</dd><dt>Download</dt><dd><a href=\"{url}\">{asset}</a> ({size_kb} KB, SHA-256 <code>{sha}</code>)</dd>{requires}{conflicts}{can}<dt>Source</dt><dd><a href=\"https://github.com/{repo}\">{repo}</a></dd></dl>\n{notes}<h2 class=\"stencil\">Readme</h2>\n<article class=\"readme\">{readme}</article>",
        notice = p.notice.as_ref().map(|n| format!("<p class=\"notice\">{}</p>\n", esc(n))).unwrap_or_default(),
        can = match (p.scanned, p.capabilities.is_empty()) {
            (false, _) => String::new(),
            (true, true) => "<dt>Can do</dt><dd>Nothing outside the game: plain sandboxed Lua.</dd>".to_string(),
            (true, false) => format!("<dt>Can do</dt><dd>{}</dd>", esc(&p.capabilities.join(", "))),
        },
        picture = p.image.as_ref().map(|i| format!("<p><img class=\"shot wide\" src=\"card.{}\" alt=\"\"></p>\n", i.ext)).unwrap_or_default(),
        requires = relation_fact("Requires", &p.requires), conflicts = relation_fact("Conflicts with", &p.conflicts),
        notes = if p.notes_html.trim().is_empty() { String::new() } else { format!("<h2 class=\"stencil\">Changes in {}</h2>\n<article class=\"readme\">{}</article>\n", esc(&e.latest_version), p.notes_html) },
        name = esc(&e.name), badge = listing_badge(e), summary = esc(&e.summary), id = esc(&e.id),
        v = esc(&e.latest_version), tag = esc(&e.release_tag), d = esc(date(&e.released_at)),
        authors = esc(&authors), license = esc(&license), url = esc(&e.download_url), asset = esc(&e.asset_name),
        sha = esc(&e.sha256), repo = esc(&e.repository),
        readme = if p.readme_html.trim().is_empty() { "<p class=\"muted\">This mod has no readme.</p>".to_string() } else { p.readme_html.clone() },
    );
    page(input, &e.name, "../../", "mods/", false, &body)
}

fn atom(input: &SiteInput) -> String {
    let base = input.site_url;
    let entries: String = by_release(input.index)
        .iter()
        .take(50)
        .map(|e| format!(
            "  <entry>\n    <id>{base}/mods/{id}/#{v}</id>\n    <title>{name} {v}</title>\n    <updated>{d}</updated>\n    <link href=\"{base}/mods/{id}/\"/>\n    <summary>{summary}</summary>\n  </entry>\n",
            id = esc(&e.id), v = esc(&e.latest_version), name = esc(&e.name), d = esc(&e.released_at), summary = esc(&e.summary)
        ))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\">\n  <id>{base}/updates.xml</id>\n  <title>QuarterMaster mod updates</title>\n  <updated>{g}</updated>\n  <link rel=\"self\" href=\"{base}/updates.xml\"/>\n{entries}</feed>\n",
        g = esc(&input.index.generated_at)
    )
}

/// Which wiki page the sidebar is on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Here {
    Page(usize),
    All,
}

/// A page's `##` sections, each with its `###` sections nested under it.
fn section_list(sections: &[Section]) -> String {
    let mut out = String::from("<ol class=\"secs\">");
    let (mut item_open, mut sub_open) = (false, false);
    for s in sections {
        let link = format!("<a href=\"#{}\">{}</a>", esc(&s.id), esc(&s.title));
        if s.level == 2 {
            if sub_open {
                out.push_str("</ol>");
                sub_open = false;
            }
            if item_open {
                out.push_str("</li>");
            }
            out.push_str("<li>");
            out.push_str(&link);
            item_open = true;
        } else {
            if !item_open {
                out.push_str("<li>");
                item_open = true;
            }
            if !sub_open {
                out.push_str("<ol>");
                sub_open = true;
            }
            out.push_str(&format!("<li>{link}</li>"));
        }
    }
    if sub_open {
        out.push_str("</ol>");
    }
    if item_open {
        out.push_str("</li>");
    }
    out.push_str("</ol>");
    out
}

/// The wiki's sidebar. `w` is the path back to `wiki/` (`""` on its first page).
fn wiki_side(input: &SiteInput, wiki: &Wiki, here: Here, w: &str) -> String {
    let mut items = String::new();
    for (k, p) in wiki.pages.iter().enumerate() {
        let current = here == Here::Page(k);
        let href = match here {
            Here::All => format!("#{}", esc(&p.id)),
            Here::Page(_) if p.slug.is_empty() => if w.is_empty() { "./" } else { w }.to_string(),
            Here::Page(_) => format!("{w}{}/", p.slug),
        };
        let label = if k == 0 {
            "Start here".to_string()
        } else {
            esc(&p.label)
        };
        let num = p
            .number
            .map(|n| format!("<span class=\"num\">{n}</span>"))
            .unwrap_or_default();
        let secs = if current && k != 0 && !p.sections.is_empty() {
            section_list(&p.sections)
        } else {
            String::new()
        };
        items.push_str(&format!(
            "<li><a href=\"{href}\"{cur}>{num}{label}</a>{secs}</li>",
            cur = if current {
                " aria-current=\"page\""
            } else {
                ""
            }
        ));
    }
    items.push_str(&format!(
        "<li class=\"all\"><a href=\"{href}\"{cur}>All on one page</a></li>",
        href = if here == Here::All {
            "./".to_string()
        } else {
            format!("{w}{ALL_SLUG}/")
        },
        cur = if here == Here::All {
            " aria-current=\"page\""
        } else {
            ""
        }
    ));
    let version = input
        .version
        .map(|v| format!("<p class=\"side-ver\">For QuarterMaster {}</p>", esc(v)))
        .unwrap_or_default();
    format!("<details class=\"side\" open><summary>Wiki contents</summary><nav aria-label=\"Wiki\">{version}<ol class=\"parts\">{items}</ol></nav></details>")
}

/// `Part 3: Faction mods`, or the label alone for a page without a number.
fn part_name(number: Option<u32>, label: &str) -> String {
    match number {
        Some(n) => format!("Part {n}: {}", esc(label)),
        None => esc(label),
    }
}

/// The list of parts under the wiki's first page, each with its sections.
fn wiki_contents(wiki: &Wiki) -> String {
    let items: String = wiki
        .pages
        .iter()
        .skip(1)
        .map(|p| {
            let secs: Vec<String> = p
                .sections
                .iter()
                .filter(|s| s.level == 2)
                .map(|s| {
                    format!(
                        "<a href=\"{}/#{}\">{}</a>",
                        p.slug,
                        esc(&s.id),
                        esc(&s.title)
                    )
                })
                .collect();
            format!(
                "<li><a class=\"part\" href=\"{}/\">{}</a><p class=\"muted small\">{}</p></li>",
                p.slug,
                part_name(p.number, &p.label),
                secs.join(" &middot; ")
            )
        })
        .collect();
    format!("<h2 id=\"contents\">Contents</h2>\n<ul class=\"contents\">{items}</ul>\n<p><a href=\"{ALL_SLUG}/\">The whole wiki on one page</a>, for your browser's Find or for printing.</p>\n")
}

fn wiki_page(input: &SiteInput, wiki: &Wiki, k: usize) -> String {
    let p = &wiki.pages[k];
    let (w, root) = if p.slug.is_empty() {
        ("", "../")
    } else {
        ("../", "../../")
    };
    let href = |q: &crate::wiki::WikiPage| {
        if q.slug.is_empty() {
            if w.is_empty() { "./" } else { w }.to_string()
        } else {
            format!("{w}{}/", q.slug)
        }
    };
    let prev = k.checked_sub(1).map(|j| &wiki.pages[j]).map(|q| {
        format!(
            "<a class=\"prev\" rel=\"prev\" href=\"{}\"><small>Previous</small>{}</a>",
            href(q),
            if q.slug.is_empty() {
                "Start here".to_string()
            } else {
                part_name(q.number, &q.label)
            }
        )
    });
    let next = wiki.pages.get(k + 1).map(|q| {
        format!(
            "<a class=\"next\" rel=\"next\" href=\"{}\"><small>Next</small>{}</a>",
            href(q),
            part_name(q.number, &q.label)
        )
    });
    let body = format!(
        "<div class=\"docs\">\n{side}\n<article class=\"readme doc\">\n{html}{contents}<nav class=\"pager\" aria-label=\"Wiki pages\">{prev}{next}</nav>\n</article>\n</div>",
        side = wiki_side(input, wiki, Here::Page(k), w),
        html = p.html,
        contents = if k == 0 {
            wiki_contents(wiki)
        } else {
            String::new()
        },
        prev = prev.unwrap_or_default(),
        next = next.unwrap_or_default(),
    );
    let title = if k == 0 {
        "Modding wiki".to_string()
    } else {
        format!("{} - Modding wiki", p.label)
    };
    page(input, &title, root, "wiki/", true, &body)
}

fn wiki_all(input: &SiteInput, wiki: &Wiki) -> String {
    let html: String = wiki.pages.iter().map(|p| p.html_single.as_str()).collect();
    let body = format!(
        "<div class=\"docs\">\n{side}\n<article class=\"readme doc\">\n{html}</article>\n</div>",
        side = wiki_side(input, wiki, Here::All, "../"),
    );
    page(
        input,
        "Modding wiki on one page",
        "../../",
        "wiki/",
        true,
        &body,
    )
}

/// Everything the search page looks through: the wiki's sections, then the mods.
fn search_records(input: &SiteInput) -> Vec<SearchRecord> {
    let mut out: Vec<SearchRecord> = input
        .wiki
        .iter()
        .flat_map(|w| w.pages.iter().flat_map(|p| p.search.iter().cloned()))
        .collect();
    out.extend(sorted(input.index).iter().map(|e| {
        SearchRecord {
            title: e.name.clone(),
            page: "Mods".into(),
            url: format!("mods/{}/", e.id),
            text: format!("{} {} {}", e.summary, authors_of(e), e.topics.join(" "))
                .trim()
                .to_string(),
        }
    }));
    out
}

fn search_page(input: &SiteInput) -> String {
    let fallback = if input.wiki.is_some() {
        " The <a href=\"../wiki/all/\">whole wiki on one page</a> works with your browser's Find."
    } else {
        ""
    };
    let body = format!("<h1 class=\"stencil\">Search</h1>\n<p><label for=\"sq\">Search the wiki and the mods</label></p>\n<p><input id=\"sq\" type=\"search\" autocomplete=\"off\" autofocus></p>\n<p id=\"sstatus\" class=\"muted\" role=\"status\"></p>\n<ol id=\"results\" class=\"results\" data-root=\"../\"></ol>\n<noscript><p>Search needs JavaScript.{fallback}</p></noscript>");
    page(input, "Search", "../", "search/", false, &body)
}

/// A page that only forwards, for an address that moved.
fn redirect(to: &str, label: &str) -> String {
    format!("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{label} - {SITE_NAME}</title>\n<meta http-equiv=\"refresh\" content=\"0; url={to}\">\n<link rel=\"canonical\" href=\"{to}\">\n</head>\n<body>\n<p><a href=\"{to}\">{label}</a></p>\n</body>\n</html>\n", to = esc(to), label = esc(label))
}

/// The site's path on its host, with a slash at each end: `/` for `https://gamiseus.github.io`.
fn root_path(site_url: &str) -> String {
    let path = url::Url::parse(site_url)
        .map(|u| u.path().trim_end_matches('/').to_string())
        .unwrap_or_default();
    format!("{path}/")
}

fn sitemap(input: &SiteInput) -> String {
    let mut paths = vec![String::new(), "mods/".to_string()];
    paths.extend(
        sorted(input.index)
            .iter()
            .map(|e| format!("mods/{}/", e.id)),
    );
    if let Some(w) = input.wiki {
        paths.extend(w.pages.iter().map(|p| p.url()));
    }
    if input.changelog_html.is_some() {
        paths.push("changelog/".into());
    }
    let urls: String = paths
        .iter()
        .map(|p| {
            format!(
                "  <url><loc>{}/{}</loc></url>\n",
                esc(input.site_url),
                esc(p)
            )
        })
        .collect();
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{urls}</urlset>\n")
}

fn write(out: &Path, rel: &str, text: &str) -> std::io::Result<()> {
    let p = out.join(rel);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, text)
}

/// Writes every page, the index and the feed under `out`. `static/` gets the built-in assets,
/// then the files of `static_dir` over them.
pub fn write_site(input: &SiteInput, out: &Path, static_dir: Option<&Path>) -> std::io::Result<()> {
    let mut index = input.index.clone();
    index.mods.sort_by(|a, b| a.id.cmp(&b.id));
    write(
        out,
        "api/v1/index.json",
        &(serde_json::to_string_pretty(&index).map_err(std::io::Error::other)? + "\n"),
    )?;
    write(out, "index.html", &home(input))?;
    write(out, "mods/index.html", &mod_list(input))?;
    let mut pages: Vec<&ModPage> = input.pages.iter().collect();
    pages.sort_by(|a, b| a.entry.id.cmp(&b.entry.id));
    for p in pages {
        write(
            out,
            &format!("mods/{}/index.html", p.entry.id),
            &mod_page(input, p),
        )?;
        if let Some(image) = &p.image {
            std::fs::write(
                out.join(format!("mods/{}/card.{}", p.entry.id, image.ext)),
                &image.bytes,
            )?;
        }
    }
    write(out, "updates.xml", &atom(input))?;

    if let Some(wiki) = input.wiki {
        for (k, p) in wiki.pages.iter().enumerate() {
            write(
                out,
                &format!("{}index.html", p.url()),
                &wiki_page(input, wiki, k),
            )?;
        }
        write(
            out,
            &format!("wiki/{ALL_SLUG}/index.html"),
            &wiki_all(input, wiki),
        )?;
        // The author guide used to be its own page; its address still works.
        let guide = wiki
            .locate(PUBLISH_ANCHOR)
            .unwrap_or_else(|| "wiki/".into());
        write(
            out,
            "publish/index.html",
            &redirect(&format!("../{guide}"), "Publishing a mod"),
        )?;
    }
    if let Some(html) = input.changelog_html {
        write(
            out,
            "changelog/index.html",
            &page(
                input,
                "Changelog",
                "../",
                "changelog/",
                false,
                &format!("<article class=\"readme doc\">\n<h1>Changelog</h1>\n{html}</article>"),
            ),
        )?;
    }
    write(out, "search/index.html", &search_page(input))?;
    write(
        out,
        "search.json",
        &(serde_json::to_string(&search_records(input)).map_err(std::io::Error::other)? + "\n"),
    )?;

    // GitHub Pages serves 404.html for any missing address, so its links start at the root.
    write(
        out,
        "404.html",
        &page(
            input,
            "Page not found",
            &root_path(input.site_url),
            "",
            false,
            "<h1 class=\"stencil\">Page not found</h1>\n<p>Nothing is at this address. Try the search box above, or start from the home page.</p>",
        ),
    )?;
    write(out, "sitemap.xml", &sitemap(input))?;
    write(
        out,
        "robots.txt",
        &format!(
            "User-agent: *\nAllow: /\nSitemap: {}/sitemap.xml\n",
            input.site_url
        ),
    )?;
    write(out, ".nojekyll", "")?;

    let static_out = out.join("static");
    std::fs::create_dir_all(&static_out)?;
    for (name, bytes) in ASSETS {
        std::fs::write(static_out.join(name), bytes)?;
    }
    if let Some(s) = static_dir {
        copy_dir(s, &static_out)?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    let mut entries: Vec<_> = std::fs::read_dir(from)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()))?;
        } else {
            std::fs::copy(&p, to.join(e.file_name()))?;
        }
    }
    Ok(())
}
