use topology_checker_io::read;

// Deeply nested folders must be rejected with an error, never overflow the stack.
// This runs on a default 2 MiB test thread, like a request handler would.
#[test]
fn deeply_nested_kml_folders_are_rejected_without_overflowing_the_stack() {
    let dir = tempfile::tempdir().unwrap();
    let depth = 70;
    let path = dir.path().join("deep.kml");
    std::fs::write(
        &path,
        format!(
            r#"<?xml version="1.0"?><kml xmlns="http://www.opengis.net/kml/2.2">{}{}</kml>"#,
            "<Folder>".repeat(depth),
            "</Folder>".repeat(depth)
        ),
    )
    .unwrap();
    let error = read::read(&path, None, "deep").unwrap_err();
    assert!(format!("{error:#}").contains("nesting"), "{error:#}");
}

fn nested(depth: usize) -> String {
    format!(
        r#"<?xml version="1.0"?><kml xmlns="http://www.opengis.net/kml/2.2">{}<Placemark><Point><coordinates>1,1</coordinates></Point></Placemark>{}</kml>"#,
        "<Folder>".repeat(depth),
        "</Folder>".repeat(depth)
    )
}

// `nested(n)` is kml > n folders > Placemark > Point > coordinates, so n + 4 levels.
#[test]
fn nesting_limit_is_inclusive_for_kml_and_kmz() {
    let dir = tempfile::tempdir().unwrap();
    let kmz = |depth: usize| {
        let path = dir.path().join(format!("depth{depth}.kmz"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("doc.kml", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, nested(depth).as_bytes()).unwrap();
        zip.finish().unwrap();
        path
    };
    for depth in [20, 21, 70] {
        let kml_path = dir.path().join(format!("depth{depth}.kml"));
        std::fs::write(&kml_path, nested(depth)).unwrap();
        for path in [kml_path, kmz(depth)] {
            let result = read::read(&path, None, "nested");
            if depth == 20 {
                assert_eq!(result.unwrap()[0].layer.len(), 1, "{path:?}");
            } else {
                let error = result.unwrap_err();
                assert!(
                    format!("{error:#}").contains("nesting"),
                    "{path:?}: {error:#}"
                );
            }
        }
    }
}

#[test]
fn comments_cdata_and_empty_elements_do_not_count_towards_nesting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flat.kml");
    std::fs::write(
        &path,
        format!(
            r#"<?xml version="1.0"?><kml xmlns="http://www.opengis.net/kml/2.2"><Document>{}<name><![CDATA[<a><b><c>]]></name><Placemark><Point><coordinates>1,1</coordinates></Point></Placemark></Document></kml>"#,
            "<!-- <Folder><Folder> --><Style/>".repeat(100)
        ),
    )
    .unwrap();
    assert_eq!(read::read(&path, None, "flat").unwrap()[0].layer.len(), 1);
}

#[test]
fn malformed_kml_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.kml");
    std::fs::write(&path, "<kml><Folder></kml>").unwrap();
    assert!(read::read(&path, None, "broken").is_err());
}
