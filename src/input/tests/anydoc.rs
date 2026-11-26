use super::collect_values;
use super::fixture_path;
use super::read_err;
use super::uri;
use super::workspace_tempdir;
use super::write_base64_fixture;
use crate::input::Input;
use crate::input::open_input_values;
use fluent_uri::UriRef;
use std::fs;

#[test]
fn anydoc_converts_pdf_to_default_markdown_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.pdf");
    write_base64_fixture("anydoc/sample.pdf.base64", &path);
    let values = collect_values(Input::try_from(uri(&path)).unwrap());

    assert_eq!(values.len(), 1);
    assert!(
        values[0]["content"]["body"]
            .as_str()
            .unwrap()
            .contains("Hello PDF")
    );
    assert_eq!(values[0]["origin"]["scheme"], "file");
    assert_eq!(values[0]["origin"]["filename"], "sample.pdf");
}

#[test]
fn anydoc_converts_rtf_to_custom_content_field() {
    let path = fixture_path("anydoc/sample.rtf");
    let values = collect_values(open_input_values(vec![uri(&path)], "markdown").unwrap());

    assert!(
        values[0]["content"]["markdown"]
            .as_str()
            .unwrap()
            .contains("Hello from RTF")
    );
    assert!(values[0]["content"].get("body").is_none());
}

#[test]
fn anydoc_converts_office_docx_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.docx");
    write_base64_fixture("anydoc/sample.docx.base64", &path);

    let values = collect_values(Input::try_from(uri(&path)).unwrap());

    assert_eq!(values.len(), 1);
    assert!(
        values[0]["content"]["body"]
            .as_str()
            .unwrap()
            .contains("Hello from DOCX")
    );
}

#[test]
fn anydoc_mixed_file_import_preserves_origin_metadata() {
    let dir = workspace_tempdir();
    let pdf = dir.path().join("sample.pdf");
    write_base64_fixture("anydoc/sample.pdf.base64", &pdf);
    let rtf = dir.path().join("sample.rtf");
    fs::copy(fixture_path("anydoc/sample.rtf"), &rtf).unwrap();
    let values = collect_values(open_input_values(vec![uri(&rtf), uri(&pdf)], "body").unwrap());

    assert_eq!(values.len(), 2);
    let mut filenames = values
        .iter()
        .map(|value| value["origin"]["filename"].as_str().unwrap())
        .collect::<Vec<_>>();
    filenames.sort();
    assert_eq!(filenames, ["sample.pdf", "sample.rtf"]);
    for value in values {
        assert_eq!(value["origin"]["scheme"], "file");
        assert!(value["origin"].get("authority").is_none());
        assert!(!value["origin"]["path"].as_str().unwrap().starts_with('/'));
        assert!(value["origin"].get("query").is_none());
        assert!(value["origin"].get("fragment").is_none());
        assert!(value["content"]["body"].is_string());
    }
}

#[test]
fn anydoc_recursive_glob_imports_pdf_files() {
    let dir = workspace_tempdir();
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).unwrap();
    write_base64_fixture("anydoc/sample.pdf.base64", &nested.join("sample.pdf"));
    let pattern = dir
        .path()
        .join("**")
        .join("*.pdf")
        .to_string_lossy()
        .into_owned();

    let values =
        collect_values(open_input_values(vec![UriRef::parse(pattern).unwrap()], "body").unwrap());

    assert_eq!(values.len(), 1);
    assert!(values[0]["content"]["body"].is_string());
    assert_eq!(values[0]["origin"]["filename"], "sample.pdf");
    assert!(
        !values[0]["origin"]["path"]
            .as_str()
            .unwrap()
            .starts_with('/')
    );
}

#[test]
fn anydoc_multiple_extension_globs_combine_inputs() {
    let dir = workspace_tempdir();
    let pdf = dir.path().join("a.pdf");
    let rtf = dir.path().join("b.rtf");
    write_base64_fixture("anydoc/sample.pdf.base64", &pdf);
    fs::copy(fixture_path("anydoc/sample.rtf"), &rtf).unwrap();
    let pdf_pattern = dir.path().join("**/*.pdf").to_string_lossy().into_owned();
    let rtf_pattern = dir.path().join("**/*.rtf").to_string_lossy().into_owned();

    let values = collect_values(
        open_input_values(
            vec![
                UriRef::parse(rtf_pattern).unwrap(),
                UriRef::parse(pdf_pattern).unwrap(),
            ],
            "body",
        )
        .unwrap(),
    );

    assert_eq!(values.len(), 2);
    let mut filenames = values
        .iter()
        .map(|value| value["origin"]["filename"].as_str().unwrap())
        .collect::<Vec<_>>();
    filenames.sort();
    assert_eq!(filenames, ["a.pdf", "b.rtf"]);
}

#[test]
fn anydoc_conversion_error_includes_source_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.pdf");
    fs::write(&path, b"not a PDF").unwrap();

    let err = read_err(open_input_values(vec![uri(&path)], "body"));

    assert!(err.contains("invalid.pdf"));
    assert!(err.len() > "invalid.pdf".len());
}

#[test]
fn anydoc_rejects_image_only_pdf_with_ocr_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image-only.pdf");
    write_base64_fixture("anydoc/image-only.pdf.base64", &path);

    let err = read_err(open_input_values(vec![uri(&path)], "body"));

    assert!(err.contains("image-only.pdf"));
    assert!(err.contains("needs OCR"));
}
