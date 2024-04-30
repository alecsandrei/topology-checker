use core::panic;
use gdal::{
    vector::{FeatureIterator, Layer, LayerAccess, ToGdal},
    Dataset, LayerOptions,
};
use geo::{
    BoundingRect, Contains, ConvexHull, EuclideanDistance, Intersects, LineString, LinesIter,
    TryConvert, Within,
};
use geo_types::geometry;
use rayon::{
    iter::{FlatMapIter, ParallelIterator},
    prelude::*,
};
use std::{borrow::BorrowMut, collections::HashSet, ops::Deref};

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
        let features = layer.features();
        let geometries: Vec<geo_types::geometry::Geometry> = features
            .into_iter()
            .map(|feature| feature.geometry().unwrap().to_geo().unwrap())
            .collect();
        let lines = gather_lines_par(geometries);

        let dangles: &Vec<geo::Point> = &lines
            .iter()
            .par_bridge()
            .filter(|line| !line.is_closed())
            .flat_map(|line| {
                let bbox = line.bounding_rect().unwrap();
                let mut points = line.points().into_iter();
                let mut first = Some(points.next().unwrap());
                let mut last = Some(points.last().unwrap());
                for other_line in lines
                    .iter()
                    .par_bridge()
                    .filter(|linestring| linestring.intersects(&bbox))
                    .collect::<Vec<&LineString>>()
                    .into_iter()
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

                    if first.is_none() && last.is_none() {
                        break;
                    }
                }
                vec![first, last]
                    .into_iter()
                    .filter_map(|x| x)
                    .collect::<Vec<_>>()
            })
            .collect();

        println!("{}", dangles.len());
    }
}

pub fn gather_lines_par(geometries: Vec<geo_types::geometry::Geometry>) -> Vec<LineString<f64>> {
    geometries
        .into_iter()
        .par_bridge()
        .flat_map_iter(|geometry| match geometry {
            geo_types::Geometry::LineString(linestring) => {
                Box::new(std::iter::once(linestring)) as Box<dyn Iterator<Item = LineString<f64>>>
            }
            geo_types::Geometry::MultiLineString(multilinestring) => {
                Box::new(multilinestring.into_iter().map(|linestring| linestring))
                    as Box<dyn Iterator<Item = LineString<f64>>>
            }
            _ => panic!("Other type than line found."),
        })
        .collect()
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
