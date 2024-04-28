use core::panic;
use gdal::{
    vector::{LayerAccess, ToGdal},
    Dataset, LayerOptions,
};
use geo::{EuclideanDistance, HasDimensions, Intersects};
use std::time;

const TOLERANCE: f64 = 0.01;

pub struct VectorDataset(Dataset);

fn open_dataset(path: &str) -> Dataset {
    Dataset::open(path).expect("Could not read file.")
}

impl VectorDataset {
    pub fn new(path: &str) -> Self {
        VectorDataset(open_dataset(path))
    }

    // fn read(&self) -> Vec<gdal::vector::Feature<'_>> {
    //     let mut layer = self.0.layers().next().unwrap();
    //     let features: Vec<_> = layer.features().collect();
    // let geometries: Box<dyn Iterator<Item = geo::Line>> = Box::new(
    //     features
    //         .into_iter()
    //         .map(|feature| {
    //             let geometry = feature.geometry().unwrap();
    //             let geometry_count = geometry.geometry_count();
    //             if geometry_count == 0 {
    //                 let linestring: geo::LineString =
    //                     geometry.to_geo().unwrap().try_into().unwrap();
    //                 Box::new(linestring.lines_iter()) as Box<dyn Iterator<Item = geo::Line>>
    //             } else {
    //                 let multilinestring: geo::MultiLineString =
    //                     geometry.to_geo().unwrap().try_into().unwrap();
    //                 Box::new(
    //                     multilinestring
    //                         .into_iter()
    //                         .map(|linestring| linestring.lines_iter())
    //                         .flatten(),
    //                 ) as Box<dyn Iterator<Item = geo::Line>>
    //             }
    //         })
    //         .flatten(),
    // )
    //     as Box<Box<dyn Iterator<Item = geo::Line>>>;
    // features
    // }

    pub fn there_are_no_dangles(&self) {
        let mut layer = self.0.layers().next().unwrap();
        let features: Vec<_> = layer.features().collect();
        let mut dangles: Vec<geo::Point> = Vec::new();
        let mut logic = |lines: Vec<geo::LineString>| {
            for line in lines.iter().filter(|linestring| !linestring.is_closed()) {
                let mut points = line.points().into_iter();
                let mut first = Some(points.next().unwrap());
                let mut last = Some(points.last().unwrap());
                for other_line in &lines {
                    if other_line == line {
                        let sublines: Vec<geo::Line> = line.lines().collect();
                        if let Some(point) = &first {
                            for subline in sublines[1..].iter() {
                                let distance = point.euclidean_distance(subline);
                                if distance < TOLERANCE {
                                    first = None;
                                    break;
                                }
                            }
                        }
                        if let Some(point) = &last {
                            for subline in sublines[0..sublines.len() - 1].iter() {
                                let distance = point.euclidean_distance(subline);
                                if distance < TOLERANCE {
                                    last = None;
                                    break;
                                }
                            }
                        }
                    } else {
                        if let Some(point) = &first {
                            let distance = point.euclidean_distance(other_line);
                            if distance < TOLERANCE {
                                first = None
                            } else if point.intersects(other_line) {
                                first = None
                            }
                        }
                        if let Some(point) = &last {
                            let distance = point.euclidean_distance(other_line);
                            if distance < TOLERANCE {
                                last = None
                            } else if point.intersects(other_line) {
                                last = None
                            }
                        }
                    }

                    if let (None, None) = (&first, &last) {
                        break;
                    }
                }

                if let Some(point) = first {
                    dangles.push(point)
                }
                if let Some(point) = last {
                    dangles.push(point)
                }
            }
        };

        let mut lines: Vec<geo::LineString> = Vec::new();
        for feature in features {
            let geometry = feature.geometry().unwrap();
            match geometry.geometry_name().as_str() {
                "MULTILINESTRING" => {
                    let geometry: geo::MultiLineString =
                        geometry.to_geo().unwrap().try_into().unwrap();
                    let geometry = geometry.into_iter().filter(|linestring| {
                        linestring.to_gdal().unwrap().is_valid() && !linestring.is_empty()
                    });
                    lines.extend(geometry.into_iter());
                }
                "LINESTRING" => {
                    let geometry: geo::LineString = geometry.to_geo().unwrap().try_into().unwrap();
                    if geometry.to_gdal().unwrap().is_valid() && !geometry.is_empty() {
                        lines.push(geometry);
                    }
                }
                _ => panic!("Wrong names"),
            };
        }
        logic(lines);
        println!("{}", dangles.len());
        geometries_to_file(dangles, "./assets/dangles.shp");
    }
}

fn geometries_to_file(geometries: Vec<geo::Point>, out_path: &str) {
    let geometries: Vec<gdal::vector::Geometry> = geometries
        .into_iter()
        .map(|point| point.to_gdal().unwrap())
        .collect();
    let drv = gdal::DriverManager::get_driver_by_name("ESRI Shapefile").unwrap();
    let mut ds = drv.create_vector_only(out_path).unwrap();
    let mut lyr = ds
        .create_layer(LayerOptions {
            name: "dangles",
            srs: geometries.first().unwrap().spatial_ref().as_ref(),
            ty: gdal::vector::OGRwkbGeometryType::wkbPoint,
            ..Default::default()
        })
        .unwrap();
    geometries.into_iter().for_each(|geom| {
        lyr.create_feature(geom).expect("Couldn't write geometry");
    });
}

fn point_to_geometry(
    point: (f64, f64, f64),
) -> Result<gdal::vector::Geometry, gdal::errors::GdalError> {
    gdal::vector::Geometry::from_wkt(&format!("POINT ({} {})", point.0, point.1))
}

fn print_type_of<T>(_: &T) {
    println!("{}", std::any::type_name::<T>())
}
