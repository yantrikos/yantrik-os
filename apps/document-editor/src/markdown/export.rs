//! The document as a standalone HTML page.

/// The document as a standalone HTML page.
///
/// The renderer is `pulldown-cmark`, which is already in this tree — the Notes app parses
/// Markdown with it — so this is a reuse and not a second Markdown implementation to keep in
/// step with the first. The page it emits is deliberately plain: a charset, a title and the
/// rendered body, because the point of the export is the content, and a stylesheet invented here
/// would be this app's opinion travelling with someone else's document.
pub fn to_html(markdown: &str, title: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut body = String::new();
    html::push_html(&mut body, Parser::new_ext(markdown, options));
    let title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<!doctype html>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n{body}")
}
