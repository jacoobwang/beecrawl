use lopdf::Document;

pub struct ParsedPdf {
    pub markdown: String,
    pub num_pages: usize,
    pub total_pages: usize,
}

pub fn parse(bytes: &[u8], max_pages: Option<usize>) -> Result<ParsedPdf, String> {
    let document = Document::load_mem(bytes).map_err(|error| format!("invalid PDF: {error}"))?;
    let pages = document.get_pages();
    let total_pages = pages.len();
    if total_pages == 0 {
        return Err("PDF contains no pages".to_string());
    }
    let num_pages = max_pages.unwrap_or(total_pages).min(total_pages);
    let page_numbers = pages.keys().copied().take(num_pages).collect::<Vec<_>>();
    let markdown = document
        .extract_text(&page_numbers)
        .map_err(|error| format!("could not extract PDF text: {error}"))?
        .trim()
        .to_string();

    Ok(ParsedPdf {
        markdown,
        num_pages,
        total_pages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_pdf() -> Vec<u8> {
        let content = "BT /F1 12 Tf 72 720 Td (Hello BeeCrawl) Tj ET\n";
        let document = format!(
            "%PDF-1.5\n1 0 obj<</Type/Pages/Kids[5 0 R]/Count 1/Resources 3 0 R/MediaBox[0 0 595 842]>>endobj\n2 0 obj<</Type/Font/Subtype/Type1/BaseFont/Courier>>endobj\n3 0 obj<</Font<</F1 2 0 R>>>>endobj\n5 0 obj<</Type/Page/Parent 1 0 R/Contents[4 0 R]>>endobj\n6 0 obj<</Type/Catalog/Pages 1 0 R>>endobj\n4 0 obj<</Length {}>>stream\n{}endstream\nendobj\n",
            content.len(),
            content,
        );
        format!(
            "{document}xref\n0 7\n0000000000 65535 f \n0000000009 00000 n \n0000000096 00000 n \n0000000155 00000 n \n0000000291 00000 n \n0000000191 00000 n \n0000000248 00000 n \ntrailer\n<</Root 6 0 R/Size 7>>\nstartxref\n{}\n%%EOF",
            document.len()
        )
        .into_bytes()
    }

    #[test]
    fn extracts_text_and_page_metadata() {
        let parsed = parse(&text_pdf(), None).unwrap();
        assert_eq!(parsed.markdown, "Hello BeeCrawl");
        assert_eq!(parsed.num_pages, 1);
        assert_eq!(parsed.total_pages, 1);
    }
}
