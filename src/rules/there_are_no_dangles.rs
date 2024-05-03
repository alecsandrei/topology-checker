use geo::{BoundingRect, Geometry, Intersects, LineString, Point};
use rayon::{iter::ParallelIterator, prelude::*};
use crate::utils::flatten_linestring;

pub fn there_are_no_dangles(lines: Vec<Geometry>) -> Result<Vec<Point>, geo_types::Error> {
    let lines = flatten_linestring(lines);
    let dangles: Vec<Option<Point>> = lines
        .iter()
        .par_bridge()
        .filter(|line| !line.is_closed())
        .flat_map(|line: &LineString| {
            let bbox = line
                .bounding_rect()
                .expect("Failed to create linestring bbox.");
            let mut points = line.points().into_iter();
            let mut first = points.next();
            let mut last = points.last();
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

                if first.is_none() && last.is_none() {
                    break;
                }
            }
            vec![first, last]
        })
        .collect();
    Ok(dangles
        .into_iter()
        .par_bridge()
        .filter_map(|dangle| dangle)
        .collect::<Vec<Point>>())
}
