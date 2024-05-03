use std::time::Instant;
use topology_checker::{
    geometries_to_file,
    rules::Rules,
    rules::{must_not_intersect, there_are_no_dangles_improved, there_are_no_dangles},
    VectorDataset,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dataset = VectorDataset::new(&args[1]);
    let geometries = dataset.from_gdal();
    println!("{:?}", Rules::available(&geometries[0]).unwrap());
    let start = Instant::now();
    let points = there_are_no_dangles_improved(geometries);
    // let intersections = must_not_intersect(geometries);
    println!("There are no dangles time execution: {:?}", start.elapsed());
    geometries_to_file(points, "./assets/dangles_improved.shp");
    // geometries_to_file(intersections.1, "./assets/intersection_points.shp");
    // geometries_to_file(intersections.0, "./assets/intersection_lines.shp");
    // println!("{}", dangles.unwrap().len());
}
