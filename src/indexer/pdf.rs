use anyhow::{Context, Result};
use std::path::Path;

/// Extract text from a PDF file using the `pdf-extract` crate.
pub fn extract_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read PDF: {:?}", path))?;
    let text = pdf_extract::extract_text_from_mem(&bytes)
        .with_context(|| format!("failed to extract text from PDF: {:?}", path))?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Minimal valid PDF containing the text "Hello World".
    fn minimal_pdf_bytes() -> Vec<u8> {
        let mut buf = Vec::new();

        let obj1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let obj2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let obj3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n";
        let stream_content = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let obj4_header = format!("4 0 obj\n<< /Length {} >>\nstream\n", stream_content.len());
        let obj4_footer = b"\nendstream\nendobj\n";
        let obj5 = b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n";

        // Header
        buf.extend_from_slice(b"%PDF-1.0\n");

        let off1 = buf.len();
        buf.extend_from_slice(obj1);

        let off2 = buf.len();
        buf.extend_from_slice(obj2);

        let off3 = buf.len();
        buf.extend_from_slice(obj3);

        let off4 = buf.len();
        buf.extend_from_slice(obj4_header.as_bytes());
        buf.extend_from_slice(stream_content);
        buf.extend_from_slice(obj4_footer);

        let off5 = buf.len();
        buf.extend_from_slice(obj5);

        // xref
        let xref_offset = buf.len();
        buf.extend_from_slice(b"xref\n");
        buf.extend_from_slice(b"0 6\n");
        buf.extend_from_slice(format!("{:010} 65535 f \n", 0).as_bytes());
        buf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        buf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        buf.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
        buf.extend_from_slice(format!("{:010} 00000 n \n", off4).as_bytes());
        buf.extend_from_slice(format!("{:010} 00000 n \n", off5).as_bytes());

        // trailer
        buf.extend_from_slice(b"trailer\n");
        buf.extend_from_slice(b"<< /Size 6 /Root 1 0 R >>\n");
        buf.extend_from_slice(b"startxref\n");
        buf.extend_from_slice(format!("{}\n", xref_offset).as_bytes());
        buf.extend_from_slice(b"%%EOF\n");

        buf
    }

    #[test]
    fn extract_text_from_minimal_pdf() {
        let mut tmp = NamedTempFile::with_suffix(".pdf").unwrap();
        tmp.write_all(&minimal_pdf_bytes()).unwrap();
        tmp.flush().unwrap();

        let text = extract_text(tmp.path()).unwrap();
        assert!(
            text.contains("Hello") && text.contains("World"),
            "expected 'Hello World' in extracted text, got: {:?}",
            text
        );
    }

    #[test]
    fn extract_text_nonexistent_file() {
        let result = extract_text(Path::new("/nonexistent/file.pdf"));
        assert!(result.is_err());
    }

    #[test]
    fn extract_text_invalid_pdf() {
        let mut tmp = NamedTempFile::with_suffix(".pdf").unwrap();
        tmp.write_all(b"not a pdf").unwrap();
        tmp.flush().unwrap();

        let result = extract_text(tmp.path());
        assert!(result.is_err());
    }
}
