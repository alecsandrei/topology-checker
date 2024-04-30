use core::panic;
use gdal::{
    vector::{FeatureIterator, Layer, LayerAccess, ToGdal},
    Dataset, LayerOptions,
};
use geo::{
    BoundingRect, Contains, ConvexHull, EuclideanDistance, Intersects, LineString, LinesIter,
    Within,
};
use std::collections::HashSet;
use rayon::prelude::*;

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

    pub fn there_are_no_dangles(&self) -> Vec<geo::Point> {
        let mut layer = self.0.layers().next().unwrap();
        let features = layer.features();
        let mut dangles: Vec<geo::Point> = Vec::new();
        let logic = |lines: Vec<geo::LineString>| -> Vec<geo::Point> {
            // let lines = lines.collect::<Vec<geo::LineString>>();
            lines.iter().filter(|linestring| !linestring.is_closed()).map(|line| {
                let bbox = line.bounding_rect().unwrap();
                let mut points = line.points().into_iter();
                let mut first = Some(points.next().unwrap());
                let mut last = Some(points.last().unwrap());
                for other_line in lines
                    .iter()
                    .filter(|linestring| linestring.intersects(&bbox))
                {
                    if other_line == line {
                        let sublines: Vec<geo::Line> = line.lines().collect();
                        if let Some(point) = &first {
                            for subline in sublines[1..].iter() {
                                if subline.intersects(point) {
                                    first = None;
                                    break;
                                }
                            }
                        }
                        if let Some(point) = &last {
                            for subline in sublines[0..sublines.len() - 1].iter() {
                                if subline.intersects(point) {
                                    last = None;
                                    break;
                                }
                            }
                        }
                    } else {
                        if let Some(point) = &first {
                            if other_line.intersects(point) {
                                first = None
                            }
                        }
                        if let Some(point) = &last {
                            if other_line.intersects(point) {
                                last = None
                            }
                        }
                    }

                    if let (None, None) = (&first, &last) {
                        break;
                    }
                }

                if let Some(point) = first {
                    dangles.push(point);
                }
                if let Some(point) = last {
                    dangles.push(point)
                }
            });
            dangles
        };

        let start = std::time::Instant::now();
        let dangles = logic(gather_lines_vec(features));

        println!("Time elapsed in logic is: {:?}", start.elapsed());
        println!("{}", dangles.len());
        dangles
    }
}

pub fn gather_lines(features: FeatureIterator) -> Box<dyn Iterator<Item = LineString> + '_> {
    let iterator = features
        .map(|feature| {
            let geometry = feature.geometry().unwrap();
            let geometry = match geometry.geometry_name().as_str() {
                "MULTILINESTRING" => {
                    let geometry: geo::MultiLineString =
                        geometry.to_geo().unwrap().try_into().unwrap();
                    Box::new(geometry.into_iter()) as Box<dyn Iterator<Item = LineString>>
                }
                "LINESTRING" => {
                    let geometry: geo::LineString = geometry.to_geo().unwrap().try_into().unwrap();
                    Box::new(std::iter::once(geometry).into_iter())
                        as Box<dyn Iterator<Item = LineString>>
                }
                _ => panic!("Wrong names"),
            };
            geometry
        })
        .flatten();
    Box::new(iterator) as Box<dyn Iterator<Item = LineString>>
}

pub fn gather_lines_vec(features: FeatureIterator) -> Vec<geo::LineString> {
    let mut lines: Vec<geo::LineString> = Vec::new();
    features.for_each(|feature| {
        let geometry = feature.geometry().unwrap();
        match geometry.geometry_name().as_str() {
            "MULTILINESTRING" => {
                let geometry: geo::MultiLineString = geometry.to_geo().unwrap().try_into().unwrap();
                geometry.into_iter().for_each(|line| lines.push(line))
            }
            "LINESTRING" => {
                let geometry: geo::LineString = geometry.to_geo().unwrap().try_into().unwrap();
                lines.push(geometry)
            }
            _ => panic!("Wrong names"),
        };
    });
    lines
}

pub fn geometries_to_file(geometries: Vec<geo::Point>, out_path: &str) {
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
