fn main() {
    let args: Vec<String> = std::env::args().collect();
    if gdal::version::VersionInfo::has_geos() {
        println!("GEOS enabled.");
    }
    let dataset = topology_checker::VectorDataset::new(&args[1]);
    dataset.there_are_no_dangles();

}
