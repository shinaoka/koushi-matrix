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
fn javascript_and_other_unsafe_link_schemes_are_dropped_and_https_kept() {
    let doc = render_markdown_document(
        "[x](javascript:alert(1)) [d](data:text/html,<script>alert(1)</script>) [v](vbscript:alert(1)) [y](https://example.org)\n",
    );
    for scheme in ["javascript:", "data:", "vbscript:"] {
        assert!(!doc.html.contains(scheme), "{scheme} in {}", doc.html);
    }
    assert!(!doc.html.contains("<script"), "{}", doc.html);
    assert!(
        doc.html.contains("href=\"https://example.org\""),
        "{}",
        doc.html
    );
}

#[test]
fn markdown_images_never_produce_an_img() {
    let doc = render_markdown_document("![x](mxc://a/b) ![y](https://example.org/i.png)\n");
    assert!(!doc.html.contains("<img"), "{}", doc.html);
}

#[test]
fn ids_exist_only_on_headings() {
    let doc = render_markdown_document(
        "# H\n\n| a |\n|---|\n| 1 |\n\n- item\n\n> quote\n\n`c`\n\n```\nfence\n```\n",
    );
    assert_eq!(doc.html.matches(" id=").count(), 1, "{}", doc.html);
    assert!(doc.html.contains("<h1 id=\"md-h0\">"), "{}", doc.html);
}

#[test]
fn duplicate_headings_get_unique_sequential_ids() {
    let doc = render_markdown_document("# Same\n\n## Same\n\n# Same\n");
    for id in ["md-h0", "md-h1", "md-h2"] {
        assert_eq!(
            doc.html.matches(&format!("id=\"{id}\"")).count(),
            1,
            "{id} in {}",
            doc.html
        );
    }
}

#[test]
fn a_document_without_headings_has_no_ids() {
    let doc = render_markdown_document("just text\n\n- a\n- b\n");
    assert!(!doc.html.contains(" id="), "{}", doc.html);
}

#[test]
fn deeply_nested_block_quotes_do_not_crash() {
    let markdown = format!("{} deep\n", ">".repeat(5000));
    let doc = render_markdown_document(&markdown);
    // The sanitizer bounds the nesting, so the React side never sees 5000 levels.
    assert!(
        doc.html.matches("<blockquote>").count() <= 100,
        "{}",
        doc.html.len()
    );
}

#[test]
fn message_path_still_drops_heading_ids() {
    use crate::timeline::html_sanitize::sanitize_matrix_html;
    let out = sanitize_matrix_html("<h1 id=\"md-h0\">T</h1><p id=\"x\">p</p>", &[]);
    assert!(!out.contains("id="), "{out}");
}

#[test]
fn message_path_keeps_its_image_handling() {
    use crate::timeline::html_sanitize::sanitize_matrix_html;
    let body = "<img src=\"mxc://a/b\" alt=\"x\">";
    let out = sanitize_matrix_html(body, &[]);
    // Unlike the document sanitizer, the message sanitizer does not remove `img`.
    assert!(out.contains("<img"), "{out}");
    assert!(!super::super::html_sanitize::sanitize_markdown_document_html(body).contains("<img"));
}
