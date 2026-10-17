use std::{io::Write, path::Path};
use topology_checker_io::read;

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}
fn kml(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><kml xmlns="http://www.opengis.net/kml/2.2">{body}</kml>"#
    )
}
fn kmz(path: &Path, entries: &[(&str, &str)]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, text) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}
fn payload_data(json: &str) -> String {
    format!(
        r#"<ExtendedData><Data name="topology_checker"><value>{}</value></Data></ExtendedData>"#,
        json.replace('"', "&quot;")
    )
}

#[test]
fn third_party_kml_keeps_names_attributes_ids_and_z_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "foreign.kml",
        &kml(r##"<Document><Folder>
            <Placemark id="7"><name>point</name><description>first</description>
              <ExtendedData>
                <Data name="a"><value>1</value></Data>
                <Data name="b">two</Data>
                <Data><value>unnamed</value></Data>
                <Data name="empty"/>
                <SchemaData schemaUrl="#s"><SimpleData name="c">three</SimpleData></SchemaData>
              </ExtendedData>
              <Point><coordinates>5,52,3</coordinates></Point></Placemark>
            <Placemark><LineString><coordinates>5,52,1 6,53,2</coordinates></LineString></Placemark>
            <Placemark><LinearRing><coordinates>5,52,1 6,52,1 6,53,1 5,52,1</coordinates></LinearRing></Placemark>
            <Placemark><Polygon>
              <outerBoundaryIs><LinearRing><coordinates>0,0 10,0 10,10 0,0</coordinates></LinearRing></outerBoundaryIs>
              <innerBoundaryIs><LinearRing><coordinates>1,1,5 2,1,5 2,2,5 1,1,5</coordinates></LinearRing></innerBoundaryIs>
            </Polygon></Placemark>
            <Placemark><Polygon>
              <outerBoundaryIs><LinearRing><coordinates>0,0 10,0 10,10 0,0</coordinates></LinearRing></outerBoundaryIs>
            </Polygon></Placemark>
            <Placemark><MultiGeometry><Point><coordinates>1,1,9</coordinates></Point></MultiGeometry></Placemark>
            <Placemark/>
          </Folder></Document>"##),
    );
    let data = read::read_with_attributes(&path, None, "foreign", true)
        .unwrap()
        .remove(0);
    assert_eq!(data.layer.len(), 6);
    let first = &data.attributes[0];
    assert_eq!(first["name"], "point");
    assert_eq!(first["description"], "first");
    assert_eq!(first["a"], "1");
    assert_eq!(first["b"], "two");
    assert_eq!(first["c"], "three");
    assert!(!first.contains_key("empty"));
    assert!(data.layer.features().any(|(id, _)| id == 7));
    assert!(data.warnings.iter().any(|w| w.contains("Z/M")));
}

#[test]
fn topology_checker_payloads_restore_multi_geometries_and_ignore_foreign_json() {
    let dir = tempfile::tempdir().unwrap();
    let points = "<MultiGeometry><Point><coordinates>1,1</coordinates></Point><Point><coordinates>2,2</coordinates></Point></MultiGeometry>";
    let lines = "<MultiGeometry><LineString><coordinates>1,1 2,2</coordinates></LineString><LineString><coordinates>3,3 4,4</coordinates></LineString></MultiGeometry>";
    let polygons = "<MultiGeometry><Polygon><outerBoundaryIs><LinearRing><coordinates>0,0 1,0 1,1 0,0</coordinates></LinearRing></outerBoundaryIs></Polygon><Polygon><outerBoundaryIs><LinearRing><coordinates>5,5 6,5 6,6 5,5</coordinates></LinearRing></outerBoundaryIs></Polygon></MultiGeometry>";
    let mixed = "<MultiGeometry><Point><coordinates>1,1</coordinates></Point><LineString><coordinates>1,1 2,2</coordinates></LineString></MultiGeometry>";
    let placemark = |geometry: &str, json: &str| {
        format!("<Placemark>{}{geometry}</Placemark>", payload_data(json))
    };
    let body = [
        placemark(
            points,
            r#"{"id":-3,"properties":{"k":"v"},"type":"MULTIPOINT"}"#,
        ),
        placemark(
            lines,
            r#"{"id":4,"properties":{},"type":"MULTILINESTRING"}"#,
        ),
        placemark(
            polygons,
            r#"{"id":5,"properties":{},"type":"MULTIPOLYGON"}"#,
        ),
        placemark(
            mixed,
            r#"{"id":6,"properties":{},"type":"GEOMETRYCOLLECTION"}"#,
        ),
        placemark(mixed, r#"{"id":null,"properties":{},"type":null}"#),
        placemark(points, "not json"),
        placemark(points, r#"{"properties":"not an object"}"#),
    ]
    .concat();
    let path = write(dir.path(), "payload.kml", &kml(&body));
    let data = read::read_with_attributes(&path, None, "payload", true)
        .unwrap()
        .remove(0);
    let ids = data.layer.features().map(|(id, _)| id).collect::<Vec<_>>();
    assert!(ids.contains(&-3) && ids.contains(&4) && ids.contains(&5) && ids.contains(&6));
    assert_eq!(data.layer.len(), 7);
    assert_eq!(data.attributes[0]["k"], "v");
    // Payloads that are not ours stay visible as ordinary attributes.
    assert_eq!(data.attributes[5]["topology_checker"], "not json");
}

#[test]
fn payloads_that_disagree_with_their_geometry_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mixed = "<MultiGeometry><Point><coordinates>1,1</coordinates></Point><LineString><coordinates>1,1 2,2</coordinates></LineString></MultiGeometry>";
    for (kind, message) in [
        ("MULTIPOINT", "multipoint"),
        ("MULTILINESTRING", "multiline"),
        ("MULTIPOLYGON", "multipolygon"),
    ] {
        let json = format!(r#"{{"id":1,"properties":{{}},"type":"{kind}"}}"#);
        let path = write(
            dir.path(),
            &format!("{kind}.kml"),
            &kml(&format!(
                "<Placemark>{}{mixed}</Placemark>",
                payload_data(&json)
            )),
        );
        let error = read::read(&path, None, "bad").unwrap_err();
        assert!(format!("{error:#}").contains(message), "{kind}: {error:#}");
    }
}

#[test]
fn kml_structure_limits_and_root_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let nested = format!("<a>{}{}</a>", "<b>".repeat(70), "</b>".repeat(70));
    let path = write(
        dir.path(),
        "deep-data.kml",
        &kml(&format!(
            "<Placemark><ExtendedData>{nested}</ExtendedData></Placemark>"
        )),
    );
    let error = read::read(&path, None, "deep").unwrap_err();
    assert!(format!("{error:#}").contains("nesting"), "{error:#}");

    let path = write(
        dir.path(),
        "root.kml",
        r#"<?xml version="1.0"?><Placemark><name>bare</name></Placemark>"#,
    );
    assert!(read::read(&path, None, "root").is_err());
}

#[test]
fn kmz_picks_doc_kml_or_the_only_document() {
    let dir = tempfile::tempdir().unwrap();
    let point = |x: u8| {
        kml(&format!(
            "<Placemark><Point><coordinates>{x},1</coordinates></Point></Placemark>"
        ))
    };

    let path = dir.path().join("named.kmz");
    kmz(
        &path,
        &[
            ("other.kml", &point(1)),
            ("DOC.KML", &point(2)),
            ("icon.png", "x"),
        ],
    );
    let data = read::read(&path, None, "named").unwrap().remove(0);
    assert_eq!(data.layer.len(), 1);

    let path = dir.path().join("single.kmz");
    kmz(&path, &[("map.kml", &point(3))]);
    assert_eq!(read::read(&path, None, "single").unwrap()[0].layer.len(), 1);

    let path = dir.path().join("ambiguous.kmz");
    kmz(&path, &[("a.kml", &point(1)), ("b.kml", &point(2))]);
    let error = read::read(&path, None, "ambiguous").unwrap_err();
    assert!(format!("{error:#}").contains("doc.kml"), "{error:#}");

    let path = dir.path().join("empty.kmz");
    kmz(&path, &[("readme.txt", "none")]);
    assert!(read::read(&path, None, "empty").is_err());

    let path = dir.path().join("unsafe.kmz");
    kmz(&path, &[("../escape.kml", &point(1))]);
    let error = read::read(&path, None, "unsafe").unwrap_err();
    assert!(format!("{error:#}").contains("unsafe"), "{error:#}");
}

#[test]
fn third_party_gpx_waypoints_routes_and_tracks_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "foreign.gpx",
        r#"<?xml version="1.0"?><gpx version="1.1" creator="other" xmlns="http://www.topografix.com/GPX/1/1">
          <wpt lat="52" lon="5"><ele>3</ele><name>peak</name><desc>summit</desc></wpt>
          <wpt lat="53" lon="6"/>
          <wpt lat="54" lon="7"><desc>{"id":-9,"properties":{"k":"v"},"type":"POINT"}</desc></wpt>
          <rte><name>road</name><number>4</number><desc>main road</desc>
            <rtept lat="52" lon="5"><ele>1</ele></rtept><rtept lat="53" lon="6"/></rte>
          <rte><rtept lat="52" lon="5"/><rtept lat="53" lon="6"/></rte>
          <rte><desc>{"id":8,"properties":{"k":"v"},"type":"LINESTRING"}</desc>
            <rtept lat="52" lon="5"/><rtept lat="53" lon="6"/></rte>
          <trk><name>walk</name><number>5</number><desc>morning</desc>
            <trkseg><trkpt lat="52" lon="5"><ele>1</ele></trkpt><trkpt lat="53" lon="6"/></trkseg>
            <trkseg><trkpt lat="54" lon="7"/><trkpt lat="55" lon="8"/></trkseg></trk>
          <trk><trkseg><trkpt lat="52" lon="5"/><trkpt lat="53" lon="6"/></trkseg></trk>
          <trk><desc>{"id":10,"properties":{"k":"v"},"type":"MULTILINESTRING"}</desc>
            <trkseg><trkpt lat="52" lon="5"/><trkpt lat="53" lon="6"/></trkseg></trk>
        </gpx>"#,
    );
    let data = read::read_with_attributes(&path, None, "foreign", true)
        .unwrap()
        .remove(0);
    assert_eq!(data.layer.len(), 9);
    let attributes = &data.attributes;
    assert_eq!(attributes[0]["name"], "peak");
    assert_eq!(attributes[0]["description"], "summit");
    assert_eq!(attributes[2]["k"], "v");
    assert_eq!(attributes[3]["name"], "road");
    assert_eq!(attributes[3]["description"], "main road");
    assert_eq!(attributes[5]["k"], "v");
    assert_eq!(attributes[6]["description"], "morning");
    assert_eq!(attributes[8]["k"], "v");
    let ids = data.layer.features().map(|(id, _)| id).collect::<Vec<_>>();
    assert!(ids.contains(&-9) && ids.contains(&4) && ids.contains(&5));
    assert!(data.warnings.iter().any(|w| w.contains("Z/M")));
}
