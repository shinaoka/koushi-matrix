use super::render_markdown_document;

#[test]
fn headings_get_stable_ids_and_no_raw_markers_remain() {
    let doc = render_markdown_document("# Title\n\nintro *emph*\n\n## Part `x`\n\n#### Deep\n");
    assert!(
        doc.html.contains("<h1 id=\"md-h0\">Title</h1>"),
        "{}",
        doc.html
    );
    assert!(
        doc.html
            .contains("<h2 id=\"md-h1\">Part <code>x</code></h2>"),
        "{}",
        doc.html
    );
    assert!(doc.html.contains("<em>emph</em>"), "{}", doc.html);
    assert!(
        !doc.plain_text.contains('#') && !doc.plain_text.contains('*'),
        "{}",
        doc.plain_text
    );
}

#[test]
fn tables_survive_the_document_sanitizer() {
    let doc = render_markdown_document("| a | b |\n|---|---|\n| 1 | 2 |\n");
    for tag in ["<table>", "<thead>", "<tbody>", "<th>a</th>", "<td>2</td>"] {
        assert!(doc.html.contains(tag), "{tag} missing in {}", doc.html);
    }
}

#[test]
fn fenced_code_keeps_language_class_and_code_block_dto() {
    let doc = render_markdown_document("```rust\nfn main() {}\n```\n");
    assert!(
        doc.html.contains("<code class=\"language-rust\">"),
        "{}",
        doc.html
    );
    assert_eq!(doc.code_blocks.len(), 1);
    assert_eq!(doc.code_blocks[0].language.as_deref(), Some("rust"));
    assert_eq!(doc.code_blocks[0].body, "fn main() {}\n");
}

#[test]
fn math_uses_the_message_math_markup() {
    let doc = render_markdown_document("Energy $E=mc^2$ and\n\n$$\\int_0^1 x\\,dx$$\n");
    assert!(
        doc.html.contains("<span data-mx-maths=\"E=mc^2\">"),
        "{}",
        doc.html
    );
    assert!(
        doc.html
            .contains("<div data-mx-maths=\"\\int_0^1 x\\,dx\">"),
        "{}",
        doc.html
    );
}

#[test]
fn raw_html_is_text_never_markup() {
    let doc = render_markdown_document(
        "<script>alert(1)</script>\n\ninline <b onclick=\"x()\">bold</b> <img src=\"mxc://a/b\">\n",
    );
    assert!(!doc.html.contains("<script"), "{}", doc.html);
    assert!(!doc.html.contains("<b "), "{}", doc.html);
    assert!(!doc.html.contains("<img"), "{}", doc.html);
    assert!(
        doc.plain_text.contains("<script>alert(1)</script>"),
        "{}",
        doc.plain_text
    );
}

#[test]
fn unsafe_link_schemes_and_ids_are_dropped() {
    let doc = render_markdown_document("[x](javascript:alert(1)) [y](https://example.org)\n");
    assert!(!doc.html.contains("javascript:"), "{}", doc.html);
    assert!(
        doc.html.contains("href=\"https://example.org\""),
        "{}",
        doc.html
    );
}
