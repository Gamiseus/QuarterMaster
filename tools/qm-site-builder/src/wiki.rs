//! The modding wiki: one Markdown file, split into a page per part (each `# ` heading), with
//! heading ids, a section list per page, coloured code blocks and a search record per section.
//!
//! Heading ids follow GitHub's rule, so the file's own `#anchor` links work both on GitHub and
//! here: on the site each is rewritten to the page that holds the heading. A link that resolves
//! nowhere is reported in `Wiki::problems`, and the build stops on it.
use crate::highlight::highlight;
use crate::site::esc;
use pulldown_cmark::{html, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::Serialize;
use std::collections::HashMap;

/// The page that holds every part, at `wiki/all/`. No part may take this slug.
pub const ALL_SLUG: &str = "all";

/// A `##` or `###` heading on a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub level: u8,
    pub id: String,
    pub title: String,
}

/// One search result: a section of a wiki page (or, added by the site, a mod).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchRecord {
    #[serde(rename = "t")]
    pub title: String,
    /// Where it is, for the result's second line: the wiki page's name.
    #[serde(rename = "p")]
    pub page: String,
    /// Relative to the site root.
    #[serde(rename = "u")]
    pub url: String,
    #[serde(rename = "x")]
    pub text: String,
}

pub struct WikiPage {
    /// The folder under `wiki/`; empty for the first page, which is `wiki/` itself.
    pub slug: String,
    /// The part number, from a `Part N:` heading.
    pub number: Option<u32>,
    /// The heading without `Part N:` and without a trailing `(…)`.
    pub label: String,
    /// The id of the page's own heading.
    pub id: String,
    pub sections: Vec<Section>,
    /// The page's HTML with links relative to its own folder.
    pub html: String,
    /// The same with every wiki link as a bare `#anchor`, for the one-page view.
    pub html_single: String,
    pub search: Vec<SearchRecord>,
}

impl WikiPage {
    /// The page's address relative to the site root.
    pub fn url(&self) -> String {
        if self.slug.is_empty() {
            "wiki/".to_string()
        } else {
            format!("wiki/{}/", self.slug)
        }
    }
}

pub struct Wiki {
    pub pages: Vec<WikiPage>,
    /// Links that go nowhere. A build with any of these fails.
    pub problems: Vec<String>,
    anchors: HashMap<String, usize>,
}

impl Wiki {
    /// Where the heading with this id is, relative to the site root.
    pub fn locate(&self, id: &str) -> Option<String> {
        let page = &self.pages[*self.anchors.get(id)?];
        Some(if page.id == id {
            page.url()
        } else {
            format!("{}#{id}", page.url())
        })
    }
}

/// GitHub's heading id: lower case; letters, digits, `-` and `_` kept; each space a hyphen;
/// everything else dropped.
pub fn slug(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// `Part 7: Working with other mods (x)` as `(Some(7), "Working with other mods")`.
fn part_label(title: &str) -> (Option<u32>, String) {
    let mut number = None;
    let mut label = title.trim();
    if let Some((n, rest)) = label
        .strip_prefix("Part ")
        .and_then(|r| r.split_once(':'))
        .and_then(|(n, rest)| Some((n.trim().parse().ok()?, rest)))
    {
        number = Some(n);
        label = rest.trim();
    }
    if label.ends_with(')') {
        if let Some(p) = label.rfind(" (") {
            label = label[..p].trim_end();
        }
    }
    (number, label.to_string())
}

/// The first version a changelog names: the `1.2.0` of its first `## 1.2.0 …` heading.
pub fn first_version(changelog: &str) -> Option<String> {
    changelog.lines().find_map(|l| {
        let v: String = l
            .strip_prefix("## ")?
            .trim_start_matches(['v', 'V'])
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        (v.contains('.') && v.starts_with(|c: char| c.is_ascii_digit())).then_some(v)
    })
}

fn options() -> Options {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts
}

struct Heading {
    /// Indexes of the heading's start and end events.
    start: usize,
    end: usize,
    level: u8,
    id: String,
    text: String,
}

/// The text of a run of inline events, without markup.
fn plain(events: &[Event]) -> String {
    events
        .iter()
        .map(|e| match e {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => t.as_ref(),
            Event::SoftBreak | Event::HardBreak => " ",
            _ => "",
        })
        .collect()
}

/// Every heading, with GitHub's ids: a repeated id gets `-1`, `-2`, … in document order.
fn headings(events: &[Event]) -> Vec<Heading> {
    let mut out = Vec::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Heading { level, .. }) = &events[i] {
            let end = (i + 1..events.len())
                .find(|&j| matches!(events[j], Event::End(TagEnd::Heading(_))))
                .unwrap_or(events.len() - 1);
            let text = plain(&events[i + 1..end]);
            let base = slug(&text);
            let seen = used.entry(base.clone()).or_insert(0);
            let id = if *seen == 0 {
                base
            } else {
                format!("{base}-{seen}")
            };
            *seen += 1;
            out.push(Heading {
                start: i,
                end,
                level: *level as u8,
                id,
                text,
            });
            i = end;
        }
        i += 1;
    }
    out
}

/// Raw HTML in the source is shown as text, never passed through.
fn inert(e: &Event) -> Event<'static> {
    match e {
        Event::Html(t) | Event::InlineHtml(t) => Event::Text(t.to_string().into()),
        e => e.clone().into_static(),
    }
}

/// What a page is, before it is rendered.
struct Meta {
    slug: String,
    number: Option<u32>,
    label: String,
    id: String,
    /// The page's events are `events[from..to]`.
    from: usize,
    to: usize,
}

impl Meta {
    fn url(&self) -> String {
        if self.slug.is_empty() {
            "wiki/".to_string()
        } else {
            format!("wiki/{}/", self.slug)
        }
    }
}

struct Renderer<'a> {
    events: &'a [Event<'a>],
    by_start: HashMap<usize, &'a Heading>,
    anchors: &'a HashMap<String, usize>,
    pages: &'a [Meta],
    has_changelog: bool,
}

impl Renderer<'_> {
    /// A link's address as page `at` should write it. `single` is the one-page view.
    fn link(&self, dest: &str, at: usize, single: bool, problems: &mut Vec<String>) -> String {
        let here = &self.pages[at];
        if let Some(id) = dest.strip_prefix('#') {
            let Some(&to) = self.anchors.get(id) else {
                problems.push(format!(
                    "{}: the link to #{id} matches no heading.",
                    here.label
                ));
                return dest.to_string();
            };
            if single || to == at {
                return dest.to_string();
            }
            let target = &self.pages[to];
            let wiki_root = if here.slug.is_empty() { "" } else { "../" };
            let folder = if target.slug.is_empty() {
                String::new()
            } else {
                format!("{}/", target.slug)
            };
            let fragment = if target.id == id {
                String::new()
            } else {
                format!("#{id}")
            };
            let url = format!("{wiki_root}{folder}{fragment}");
            return if url.is_empty() { "./".into() } else { url };
        }
        if dest.starts_with("https://") || dest.starts_with("mailto:") {
            return dest.to_string();
        }
        let is_changelog = dest
            .rsplit(['/', '\\'])
            .next()
            .is_some_and(|f| f.eq_ignore_ascii_case("CHANGELOG.md"));
        if is_changelog && self.has_changelog {
            let site_root = if here.slug.is_empty() && !single {
                "../"
            } else {
                "../../"
            };
            return format!("{site_root}changelog/");
        }
        problems.push(format!(
            "{}: the link to {dest} is not an #anchor, an https address or the changelog.",
            here.label
        ));
        dest.to_string()
    }

    /// Page `at` as HTML. With `search`, also collects a record per section.
    fn render(
        &self,
        at: usize,
        single: bool,
        problems: &mut Vec<String>,
        mut search: Option<&mut Vec<SearchRecord>>,
    ) -> String {
        let meta = &self.pages[at];
        let mut out: Vec<Event> = Vec::new();
        let mut i = meta.from;
        /// Adds text to the section being read.
        fn note(search: &mut Option<&mut Vec<SearchRecord>>, text: &str) {
            if let Some(last) = search.as_mut().and_then(|s| s.last_mut()) {
                last.text.push_str(text);
                last.text.push(' ');
            }
        }
        while i < meta.to {
            match &self.events[i] {
                Event::Start(Tag::Heading { .. }) => {
                    let h = self.by_start[&i];
                    let inner: Vec<Event> = self.events[i + 1..h.end].iter().map(inert).collect();
                    let mut inner_html = String::new();
                    html::push_html(&mut inner_html, inner.into_iter());
                    let anchor = if h.level == 1 {
                        String::new()
                    } else {
                        format!(
                            " <a class=\"anchor\" href=\"#{id}\" aria-label=\"Link to this section\">#</a>",
                            id = esc(&h.id)
                        )
                    };
                    out.push(Event::Html(
                        format!(
                            "<h{l} id=\"{id}\">{inner_html}{anchor}</h{l}>\n",
                            l = h.level,
                            id = esc(&h.id)
                        )
                        .into(),
                    ));
                    if h.level <= 3 {
                        if let Some(s) = search.as_mut() {
                            s.push(SearchRecord {
                                title: if h.level == 1 {
                                    meta.label.clone()
                                } else {
                                    h.text.clone()
                                },
                                page: meta.label.clone(),
                                url: if h.level == 1 {
                                    meta.url()
                                } else {
                                    format!("{}#{}", meta.url(), h.id)
                                },
                                text: String::new(),
                            });
                        }
                    } else {
                        note(&mut search, &h.text);
                    }
                    i = h.end + 1;
                    continue;
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let lang: String = match kind {
                        CodeBlockKind::Fenced(info) => info
                            .split_whitespace()
                            .next()
                            .unwrap_or_default()
                            .chars()
                            .filter(char::is_ascii_alphanumeric)
                            .collect(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    let mut code = String::new();
                    let mut j = i + 1;
                    while j < meta.to && !matches!(self.events[j], Event::End(TagEnd::CodeBlock)) {
                        if let Event::Text(t) = &self.events[j] {
                            code.push_str(t);
                        }
                        j += 1;
                    }
                    let attr = if lang.is_empty() {
                        String::new()
                    } else {
                        format!(" data-lang=\"{lang}\"")
                    };
                    out.push(Event::Html(
                        format!(
                            "<pre class=\"code\"{attr}><code>{}</code></pre>\n",
                            highlight(&lang, &code)
                        )
                        .into(),
                    ));
                    note(&mut search, &code);
                    i = j + 1;
                    continue;
                }
                Event::Start(Tag::Table(_)) => {
                    out.push(Event::Html("<div class=\"table-wrap\">".into()));
                    out.push(self.events[i].clone());
                }
                Event::End(TagEnd::Table) => {
                    out.push(self.events[i].clone());
                    out.push(Event::Html("</div>\n".into()));
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    id,
                }) => out.push(Event::Start(Tag::Link {
                    link_type: *link_type,
                    dest_url: self.link(dest_url, at, single, problems).into(),
                    title: title.clone(),
                    id: id.clone(),
                })),
                // The rules in the file only separate the parts, which pages already do.
                Event::Rule if !single => {}
                e => {
                    match e {
                        Event::Text(t) | Event::Code(t) => note(&mut search, t),
                        _ => {}
                    }
                    out.push(inert(e));
                }
            }
            i += 1;
        }
        let mut html = String::new();
        html::push_html(&mut html, out.into_iter());
        html
    }
}

/// Splits and renders the wiki. `has_changelog` says whether the site has a changelog page for
/// the file's link to `CHANGELOG.md` to point at.
pub fn parse(markdown: &str, has_changelog: bool) -> Wiki {
    let events: Vec<Event> = Parser::new_ext(markdown, options()).collect();
    let heads = headings(&events);

    // A page starts at each top-level heading; anything before the first belongs to it.
    let mut starts: Vec<&Heading> = heads.iter().filter(|h| h.level == 1).collect();
    let mut pages: Vec<Meta> = Vec::new();
    if starts.is_empty() {
        pages.push(Meta {
            slug: String::new(),
            number: None,
            label: "Modding wiki".into(),
            id: String::new(),
            from: 0,
            to: events.len(),
        });
    }
    let mut problems = Vec::new();
    for (k, h) in starts.drain(..).enumerate() {
        let (number, label) = part_label(&h.text);
        let mut page_slug = if k == 0 { String::new() } else { slug(&label) };
        if k > 0
            && (page_slug.is_empty()
                || page_slug == ALL_SLUG
                || pages.iter().any(|p| p.slug == page_slug))
        {
            problems.push(format!(
                "The heading \"{}\" needs a different name: its page address \"{page_slug}\" is empty or already taken.",
                h.text
            ));
            page_slug = format!("{page_slug}-{k}");
        }
        if let Some(prev) = pages.last_mut() {
            prev.to = h.start;
        }
        pages.push(Meta {
            slug: page_slug,
            number,
            label,
            id: h.id.clone(),
            from: if k == 0 { 0 } else { h.start },
            to: events.len(),
        });
    }

    let mut anchors: HashMap<String, usize> = HashMap::new();
    for h in &heads {
        let page = pages.iter().rposition(|p| p.from <= h.start).unwrap_or(0);
        anchors.insert(h.id.clone(), page);
    }

    let renderer = Renderer {
        events: &events,
        by_start: heads.iter().map(|h| (h.start, h)).collect(),
        anchors: &anchors,
        pages: &pages,
        has_changelog,
    };
    let mut out = Vec::new();
    for (k, meta) in pages.iter().enumerate() {
        let mut search = Vec::new();
        let html = renderer.render(k, false, &mut problems, Some(&mut search));
        // The one-page view repeats the same links; its problems are the same ones.
        let html_single = renderer.render(k, true, &mut Vec::new(), None);
        for r in &mut search {
            r.text = r.text.split_whitespace().collect::<Vec<_>>().join(" ");
        }
        out.push(WikiPage {
            slug: meta.slug.clone(),
            number: meta.number,
            label: meta.label.clone(),
            id: meta.id.clone(),
            sections: heads
                .iter()
                .filter(|h| h.start >= meta.from && h.start < meta.to && matches!(h.level, 2 | 3))
                .map(|h| Section {
                    level: h.level,
                    id: h.id.clone(),
                    title: h.text.clone(),
                })
                .collect(),
            html,
            html_single,
            search,
        });
    }
    drop(renderer);
    Wiki {
        pages: out,
        problems,
        anchors,
    }
}

/// A changelog from its first version heading (`## …`) on: the title and the note on how
/// versions are numbered are left out.
pub fn changelog_entries(changelog: &str) -> &str {
    if changelog.starts_with("## ") {
        return changelog;
    }
    changelog.find("\n## ").map_or("", |p| &changelog[p + 1..])
}

/// A first-party Markdown page that is not split, such as the changelog: heading ids and coloured
/// code blocks; links are left as written.
pub fn render_page(markdown: &str) -> String {
    let events: Vec<Event> = Parser::new_ext(markdown, options()).collect();
    let heads = headings(&events);
    let by_start: HashMap<usize, &Heading> = heads.iter().map(|h| (h.start, h)).collect();
    let mut out: Vec<Event> = Vec::new();
    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Heading { .. }) => {
                let h = by_start[&i];
                let inner: Vec<Event> = events[i + 1..h.end].iter().map(inert).collect();
                let mut inner_html = String::new();
                html::push_html(&mut inner_html, inner.into_iter());
                out.push(Event::Html(
                    format!(
                        "<h{l} id=\"{id}\">{inner_html}</h{l}>\n",
                        l = h.level,
                        id = esc(&h.id)
                    )
                    .into(),
                ));
                i = h.end;
            }
            e => out.push(inert(e)),
        }
        i += 1;
    }
    let mut html = String::new();
    html::push_html(&mut html, out.into_iter());
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "# Example Wiki\n\nStart at [Part 1](#part-1-what-a-mod-is) or the [keys](#keys). See the [changelog](../Quartermaster/CHANGELOG.md).\n\n---\n\n# Part 1: What a mod is\n\nIntro <b>text</b>.\n\n## Files\n\nSee [Part 2](#part-2-changing-code-qmtoml), [keys](#keys), [files](#files) and [top](#example-wiki).\n\n### Keys\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n\n# Part 2: Changing code (`qm.toml`)\n\n```toml\n# Not a heading\nkey = \"v\"\n```\n\n## Keys\n\n#### Deep\n\nBack to [files](#files), [the other keys](#keys-1) and [nowhere](#missing). An [odd link](notes.md).\n";

    #[test]
    fn splits_into_a_page_per_part() {
        let w = parse(DOC, true);
        let slugs: Vec<&str> = w.pages.iter().map(|p| p.slug.as_str()).collect();
        assert_eq!(slugs, ["", "what-a-mod-is", "changing-code"]);
        assert_eq!(w.pages[1].number, Some(1));
        assert_eq!(w.pages[2].label, "Changing code");
        assert_eq!(w.pages[2].id, "part-2-changing-code-qmtoml");
        // The comment in the code block is not a heading and does not start a page.
        assert!(w.pages[2]
            .html
            .contains("<span class=\"c\"># Not a heading</span>"));
        assert_eq!(
            w.pages[1].sections,
            [
                Section {
                    level: 2,
                    id: "files".into(),
                    title: "Files".into()
                },
                Section {
                    level: 3,
                    id: "keys".into(),
                    title: "Keys".into()
                },
            ]
        );
        // A repeated heading gets GitHub's numbered id.
        assert_eq!(w.pages[2].sections[0].id, "keys-1");
        assert!(w.pages[2].html.contains("<h4 id=\"deep\">"));
    }

    #[test]
    fn anchor_links_point_at_the_page_that_holds_the_heading() {
        let w = parse(DOC, true);
        let (index, one, two) = (&w.pages[0].html, &w.pages[1].html, &w.pages[2].html);
        // From the first page (wiki/) to a part and to a section in it.
        assert!(index.contains("href=\"what-a-mod-is/\""), "{index}");
        assert!(index.contains("href=\"what-a-mod-is/#keys\""), "{index}");
        assert!(index.contains("href=\"../changelog/\""), "{index}");
        // Between parts, within a page, and back to the first page.
        assert!(one.contains("href=\"../changing-code/\""), "{one}");
        assert!(
            one.contains("href=\"#keys\"") && one.contains("href=\"#files\""),
            "{one}"
        );
        assert!(one.contains("<a href=\"../\">top</a>"), "{one}");
        assert!(two.contains("href=\"../what-a-mod-is/#files\""), "{two}");
        assert!(
            two.contains("<a href=\"#keys-1\">the other keys</a>"),
            "{two}"
        );
        // The one-page view keeps them all as anchors, and keeps the rules between parts.
        assert!(w.pages[1]
            .html_single
            .contains("href=\"#part-2-changing-code-qmtoml\""));
        assert!(w.pages[0].html_single.contains("href=\"../../changelog/\""));
        assert!(w.pages[0].html_single.contains("<hr") && !index.contains("<hr"));

        assert_eq!(
            w.locate("keys-1").as_deref(),
            Some("wiki/changing-code/#keys-1")
        );
        assert_eq!(
            w.locate("part-1-what-a-mod-is").as_deref(),
            Some("wiki/what-a-mod-is/")
        );
        assert_eq!(w.locate("example-wiki").as_deref(), Some("wiki/"));
        assert_eq!(w.locate("missing"), None);
    }

    #[test]
    fn broken_links_are_problems() {
        let w = parse(DOC, true);
        assert_eq!(w.problems.len(), 2, "{:?}", w.problems);
        assert!(w.problems[0].contains("Changing code") && w.problems[0].contains("#missing"));
        assert!(w.problems[1].contains("notes.md"));
        // Without a changelog page, the link to it has nowhere to go either.
        assert_eq!(parse(DOC, false).problems.len(), 3);
    }

    #[test]
    fn raw_html_is_shown_as_text_and_tables_scroll() {
        let w = parse(DOC, true);
        let one = &w.pages[1].html;
        assert!(
            one.contains("&lt;b&gt;text&lt;/b&gt;") && !one.contains("<b>"),
            "{one}"
        );
        assert!(
            one.contains("<div class=\"table-wrap\"><table>") && one.contains("</table>\n</div>"),
            "{one}"
        );
        assert!(
            one.contains("<h2 id=\"files\">Files <a class=\"anchor\" href=\"#files\""),
            "{one}"
        );
        assert!(
            one.starts_with("<h1 id=\"part-1-what-a-mod-is\">Part 1: What a mod is</h1>"),
            "{one}"
        );
    }

    #[test]
    fn search_has_a_record_per_section() {
        let w = parse(DOC, true);
        let s = &w.pages[2].search;
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].title.as_str(), s[0].url.as_str()),
            ("Changing code", "wiki/changing-code/")
        );
        assert!(s[0].text.contains("key = \"v\""), "{}", s[0].text);
        assert_eq!(s[1].url, "wiki/changing-code/#keys-1");
        assert!(s[1].text.starts_with("Deep Back to files"), "{}", s[1].text);
        assert_eq!(serde_json::to_value(&s[1]).unwrap()["p"], "Changing code");
    }

    #[test]
    fn slugs_labels_and_versions() {
        assert_eq!(
            slug("Unit art: `[units.<unit>]`".replace('`', "").as_str()),
            "unit-art-unitsunit"
        );
        assert_eq!(
            slug("Listing a mod in the launcher's Browse tab"),
            "listing-a-mod-in-the-launchers-browse-tab"
        );
        assert_eq!(
            part_label("Part 6: Settings, keybinds and menus (config.toml)"),
            (Some(6), "Settings, keybinds and menus".into())
        );
        assert_eq!(
            part_label("Quick reference"),
            (None, "Quick reference".into())
        );
        assert_eq!(
            first_version("# Log\n\n## 1.2.0 (in progress)\n\n## 1.1.0\n").as_deref(),
            Some("1.2.0")
        );
        assert_eq!(first_version("# Log\n\n## Unreleased\n"), None);
    }

    #[test]
    fn a_page_named_all_or_twice_is_a_problem() {
        let w = parse("# W\n\n# All\n\n# Tools\n\n# Tools\n", true);
        assert_eq!(w.problems.len(), 2, "{:?}", w.problems);
        let slugs: Vec<&str> = w.pages.iter().map(|p| p.slug.as_str()).collect();
        assert_eq!(slugs, ["", "all-1", "tools", "tools-3"]);
    }

    #[test]
    fn plain_page_gets_heading_ids() {
        let html = render_page("# Log\n\n## 1.2.0 (in progress)\n\n- a <i>b</i>\n");
        assert!(
            html.contains("<h2 id=\"120-in-progress\">1.2.0 (in progress)</h2>"),
            "{html}"
        );
        assert!(html.contains("&lt;i&gt;b&lt;/i&gt;"), "{html}");
    }
}
