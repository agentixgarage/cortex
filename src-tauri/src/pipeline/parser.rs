use std::path::Path;

use crate::error::AppError;

/// The result of parsing a document file into text content.
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub text: String,
    pub title: String,
    pub doc_type: String,
}

/// Parse a document at `path` and extract its text content.
///
/// Dispatches based on file extension. Supported types:
/// - `pdf`           — via pdf-extract
/// - `docx` / `doc`  — via docx-rust
/// - `txt` / `md`    — read directly as UTF-8
/// - `csv`           — read directly as UTF-8
/// - `xlsx` / `xls` / `ods` — via calamine
/// - `png` / `jpg` / `jpeg` / `tiff` — via tesseract OCR (v1.2 #4)
/// - anything else   — returns unsupported-type error
///
/// Note (v1.2 #4 scope): scanned/image-only PDFs (no text layer) still
/// return whatever pdf-extract finds, which may be empty or near-empty.
/// OCR-ing a scanned PDF requires rasterizing each page to an image first —
/// tracked separately as ROADMAP.md #4b (needs a PDF-render dependency,
/// e.g. pdfium-render — deliberately not added in this pass). Once a page
/// is rasterized, it flows through the exact same `ocr_image` path below.
pub fn parse_document(path: &Path) -> Result<ParsedDocument, AppError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let title = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let text = match ext.as_str() {
        "pdf" => parse_pdf(path)?,
        "docx" | "doc" => parse_docx(path)?,
        "txt" | "md" => std::fs::read_to_string(path).map_err(AppError::from)?,
        "csv" => std::fs::read_to_string(path).map_err(AppError::from)?,
        "xlsx" | "xls" | "ods" => parse_spreadsheet(path)?,
        "png" | "jpg" | "jpeg" | "tiff" => ocr_image(path)?,
        other => {
            return Err(AppError::Parse(format!(
                "Unsupported file type: {}",
                other
            )))
        }
    };

    Ok(ParsedDocument {
        text,
        title,
        doc_type: ext,
    })
}

/// Run tesseract OCR over an image file, returning the extracted text.
/// Uses the English trained-data language pack ("eng") — matches the
/// system default `tesseract` CLI behavior.
///
/// Errors surface as `AppError::Parse` with the underlying tesseract error
/// message (missing trained-data, corrupt image, etc.) rather than
/// panicking — mirrors `parse_pdf`'s error-not-panic contract so one bad
/// image doesn't kill the indexing worker.
fn ocr_image(path: &Path) -> Result<String, AppError> {
    let path_str = path
        .to_str()
        .ok_or_else(|| AppError::Parse(format!("non-UTF8 path: {}", path.display())))?;

    let mut api = tesseract::Tesseract::new(None, Some("eng"))
        .map_err(|e| AppError::Parse(format!("tesseract init failed: {}", e)))?
        .set_image(path_str)
        .map_err(|e| AppError::Parse(format!("tesseract set_image failed: {}", e)))?;

    api.get_text()
        .map_err(|e| AppError::Parse(format!("tesseract OCR failed: {}", e)))
}

fn parse_pdf(path: &Path) -> Result<String, AppError> {
    let bytes = std::fs::read(path).map_err(AppError::from)?;
    // pdf-extract 0.10 panics on some malformed / encrypted / non-standard-encoding PDFs
    // (FromUtf8Error, "missing unicode map and encoding", etc.). Catch the unwind so a
    // single bad PDF doesn't kill the tokio worker running the indexing scan.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(&bytes)
    }));
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => Err(AppError::Parse(e.to_string())),
        Err(panic_payload) => {
            let msg = panic_payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic_payload.downcast_ref::<&str>().copied())
                .unwrap_or("pdf-extract panicked");
            Err(AppError::Parse(format!(
                "pdf-extract panic parsing {}: {}",
                path.display(),
                msg
            )))
        }
    }
}

fn parse_docx(path: &Path) -> Result<String, AppError> {
    let docx_file = docx_rust::DocxFile::from_file(path)
        .map_err(|e| AppError::Parse(e.to_string()))?;
    let docx = docx_file
        .parse()
        .map_err(|e| AppError::Parse(e.to_string()))?;
    // docx_rust Body has a built-in text() method that walks all paragraphs
    Ok(docx.document.body.text())
}

fn parse_spreadsheet(path: &Path) -> Result<String, AppError> {
    use calamine::{open_workbook_auto, Reader};

    let mut workbook = open_workbook_auto(path)
        .map_err(|e| AppError::Parse(e.to_string()))?;

    let sheet_names: Vec<String> = workbook.sheet_names().to_vec();
    let mut lines: Vec<String> = Vec::new();

    for sheet_name in &sheet_names {
        if let Ok(range) = workbook.worksheet_range(sheet_name) {
            for row in range.rows() {
                let cells: Vec<String> = row
                    .iter()
                    .map(|cell| {
                        use calamine::Data;
                        match cell {
                            Data::String(s) => s.clone(),
                            Data::Float(f) => f.to_string(),
                            Data::Int(i) => i.to_string(),
                            Data::Bool(b) => b.to_string(),
                            Data::DateTime(dt) => dt.to_string(),
                            Data::DateTimeIso(s) => s.clone(),
                            Data::DurationIso(s) => s.clone(),
                            Data::Error(e) => format!("{:?}", e),
                            Data::Empty => String::new(),
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .collect();
                if !cells.is_empty() {
                    lines.push(cells.join(" "));
                }
            }
        }
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_txt_returns_content() {
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        write!(f, "hello from txt").unwrap();
        let result = parse_document(f.path()).unwrap();
        assert_eq!(result.text, "hello from txt");
        assert_eq!(result.doc_type, "txt");
    }

    #[test]
    fn test_parse_md_returns_content() {
        let mut f = NamedTempFile::with_suffix(".md").unwrap();
        write!(f, "# Heading\nsome content").unwrap();
        let result = parse_document(f.path()).unwrap();
        assert!(result.text.contains("Heading"));
        assert_eq!(result.doc_type, "md");
    }

    #[test]
    fn test_parse_csv_returns_content() {
        let mut f = NamedTempFile::with_suffix(".csv").unwrap();
        write!(f, "name,age\nAlice,30").unwrap();
        let result = parse_document(f.path()).unwrap();
        assert!(result.text.contains("Alice"));
    }

    #[test]
    fn test_parse_image_runs_ocr_and_extracts_text() {
        // Fixture generated via ImageMagick: 400x100 white canvas, black
        // text "HELLO CORTEX", 28pt. Verified readable by `tesseract` CLI
        // directly (5.5.1 + leptonica 1.86.0) before being checked in.
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ocr_test.png");
        let result = parse_document(&fixture).unwrap();
        assert!(
            result.text.to_uppercase().contains("HELLO"),
            "OCR text was: {:?}",
            result.text
        );
        assert!(
            result.text.to_uppercase().contains("CORTEX"),
            "OCR text was: {:?}",
            result.text
        );
        assert_eq!(result.doc_type, "png");
    }

    #[test]
    fn test_parse_image_malformed_file_returns_parse_error_not_panic() {
        // Zero-byte file with a .png extension — not a valid image.
        // Must return AppError::Parse, never panic (mirrors parse_pdf's
        // panic-catch contract — one bad file must not kill the indexer).
        let f = NamedTempFile::with_suffix(".png").unwrap();
        let err = parse_document(f.path()).unwrap_err();
        match err {
            AppError::Parse(_) => {}
            other => panic!("Expected Parse error, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_unsupported_extension_returns_error() {
        let f = NamedTempFile::with_suffix(".xyz").unwrap();
        let err = parse_document(f.path()).unwrap_err();
        match err {
            AppError::Parse(msg) => assert!(msg.contains("Unsupported file type")),
            other => panic!("Expected Parse error, got {:?}", other),
        }
    }

    #[test]
    fn test_title_is_filename() {
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        write!(f, "content").unwrap();
        let result = parse_document(f.path()).unwrap();
        assert!(!result.title.is_empty());
    }
}
